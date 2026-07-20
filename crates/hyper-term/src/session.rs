use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg, State};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{self, test::TermSize, Term};
use alacritty_terminal::tty;

use crate::env::{base_env, default_shell};
use crate::event::{EventProxy, SessionEvent, SessionId, Waker};

pub struct SpawnOptions {
    /// None ⇒ system default shell.
    pub shell: Option<String>,
    pub shell_args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    /// Profile-level env additions (merged over the base env).
    pub env: HashMap<String, String>,
    pub window_size: WindowSize,
    pub scrollback: usize,
    pub profile: String,
    pub cursor_style: Option<alacritty_terminal::vte::ansi::CursorStyle>,
    /// Files to delete when the session ends (the ssh askpass password file).
    /// Tying cleanup to the session means a connection that dies before the
    /// password prompt — timeout, DNS failure, rejected host key — still
    /// takes the plaintext password off disk with it.
    pub cleanup_paths: Vec<PathBuf>,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self {
            shell: None,
            shell_args: vec!["--login".into()],
            working_directory: None,
            env: HashMap::new(),
            window_size: WindowSize {
                num_lines: 24,
                num_cols: 80,
                cell_width: 8,
                cell_height: 16,
            },
            scrollback: 1000,
            profile: "default".into(),
            cursor_style: None,
            cleanup_paths: Vec::new(),
        }
    }
}

/// Build the alacritty term config for the given options.
pub fn term_config(opts: &SpawnOptions) -> term::Config {
    term::Config {
        scrolling_history: opts.scrollback,
        default_cursor_style: opts.cursor_style.unwrap_or_default(),
        // Answer kitty keyboard protocol queries/pushes; the app's key
        // encoder honors the resulting TermMode flags (shift+enter etc.).
        kitty_keyboard: true,
        ..Default::default()
    }
}

/// Shell args the app falls back to, matching `defaultShellArgs` in
/// `app/session.ts`.
pub const DEFAULT_SHELL_ARGS: [&str; 1] = ["--login"];

/// Next shell configuration to try after a shell died immediately.
/// Port of `getFallBackShellConfig` (`app/utils/shell-fallback.ts`): first
/// retry the same shell with no args (the args are the usual culprit), then
/// fall back to the system default shell. `None` ⇒ nothing left to try.
pub fn fallback_shell_config(
    shell: &str,
    shell_args: &[String],
    default_shell: &str,
) -> Option<(String, Vec<String>)> {
    if !shell_args.is_empty() {
        return Some((shell.to_string(), Vec::new()));
    }
    if shell != default_shell {
        return Some((
            default_shell.to_string(),
            DEFAULT_SHELL_ARGS.iter().map(|a| a.to_string()).collect(),
        ));
    }
    None
}

/// A terminal grid shared between the UI thread and its PTY event loop.
pub type SharedTerm = Arc<FairMutex<Term<EventProxy>>>;

/// One live PTY session: an alacritty `Term` fed by alacritty's PTY event
/// loop (reader/parser/writer on a background thread).
pub struct Session {
    pub id: SessionId,
    pub term: SharedTerm,
    sender: EventLoopSender,
    pub child_pid: Option<u32>,
    pub profile: String,
    /// Shell and args that were actually spawned, which drive the
    /// fast-nonzero-exit fallback.
    pub spawned_shell: String,
    pub spawned_shell_args: Vec<String>,
    pub started: Instant,
    /// Last title reported via OSC; maintained by the app from events.
    pub title: String,
    cleanup_paths: Vec<PathBuf>,
    io_thread: Option<JoinHandle<(EventLoop<tty::Pty, EventProxy>, State)>>,
}

impl Session {
    pub fn spawn(
        opts: SpawnOptions,
        event_tx: crossbeam_channel::Sender<SessionEvent>,
        waker: Arc<dyn Waker>,
    ) -> io::Result<Session> {
        let id = SessionId::next();
        Self::spawn_with(id, None, opts, event_tx, waker)
    }

    /// Spawn, optionally reusing an existing `Term` (shell-fallback respawn
    /// keeps the grid so the warning text stays visible).
    pub fn spawn_with(
        id: SessionId,
        reuse_term: Option<Arc<FairMutex<Term<EventProxy>>>>,
        opts: SpawnOptions,
        event_tx: crossbeam_channel::Sender<SessionEvent>,
        waker: Arc<dyn Waker>,
    ) -> io::Result<Session> {
        let proxy = EventProxy::new(id, event_tx, waker);

        let shell = opts.shell.clone().unwrap_or_else(default_shell);

        // NOTE: `tty::Options.env` is *added* to the inherited environment, so
        // removing a key here cannot unset one the app itself inherited — it
        // only drops additions this session asked for. An earlier
        // `env.remove("SSH_ASKPASS_REQUIRE")` here silently deleted the
        // variable `build_ssh_command` sets, which is what makes ssh consult
        // the askpass helper: without it ssh has a TTY and prompts inline,
        // ignoring the saved password.
        let env = base_env(&opts.env);

        let tty_options = tty::Options {
            shell: Some(tty::Shell::new(shell.clone(), opts.shell_args.clone())),
            working_directory: opts.working_directory.clone(),
            drain_on_exit: true,
            env,
        };

        let pty = tty::new(&tty_options, opts.window_size, id.0)?;
        let child_pid = pty.child().id();

        let term = match reuse_term {
            Some(term) => {
                term.lock().resize(TermSize::new(
                    opts.window_size.num_cols as usize,
                    opts.window_size.num_lines as usize,
                ));
                term
            }
            None => {
                let size = TermSize::new(
                    opts.window_size.num_cols as usize,
                    opts.window_size.num_lines as usize,
                );
                Arc::new(FairMutex::new(Term::new(
                    term_config(&opts),
                    &size,
                    proxy.clone(),
                )))
            }
        };

        let event_loop = EventLoop::new(term.clone(), proxy, pty, true, false)?;
        let sender = event_loop.channel();
        let io_thread = event_loop.spawn();

        Ok(Session {
            id,
            term,
            sender,
            child_pid: Some(child_pid),
            profile: opts.profile,
            spawned_shell: shell,
            spawned_shell_args: opts.shell_args,
            started: Instant::now(),
            title: String::new(),
            cleanup_paths: opts.cleanup_paths,
            io_thread: Some(io_thread),
        })
    }

    /// Queue bytes for the PTY (user input, paste, query responses).
    pub fn write(&self, data: impl Into<Cow<'static, [u8]>>) {
        let _ = self.sender.send(Msg::Input(data.into()));
    }

    /// Resize both the PTY and the terminal grid.
    pub fn resize(&self, size: WindowSize) {
        self.term.lock().resize(TermSize::new(
            size.num_cols as usize,
            size.num_lines as usize,
        ));
        let _ = self.sender.send(Msg::Resize(size));
    }

    /// Current working directory of the shell process (preserveCWD).
    pub fn cwd(&self) -> Option<PathBuf> {
        self.child_pid.and_then(crate::cwd::cwd_of_pid)
    }

    /// Ask the PTY to close. Never blocks: the IO thread owns the `Pty`, and
    /// dropping it runs `kill(SIGHUP)` followed by a blocking `child.wait()`
    /// (alacritty's `Pty::drop`). Joining here would run that wait on the UI
    /// thread and freeze the whole app whenever a child ignores SIGHUP, so
    /// the thread is detached and tears the PTY down on its own — the
    /// Electron build's `pty.kill()` was likewise fire-and-forget.
    pub fn shutdown(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
        self.io_thread.take();
        self.clean_up_files();
    }

    /// Remove the session's secret scratch files (ssh askpass password).
    fn clean_up_files(&mut self) {
        for path in self.cleanup_paths.drain(..) {
            if let Err(err) = std::fs::remove_file(&path) {
                if err.kind() != io::ErrorKind::NotFound {
                    log::warn!("couldn't remove {}: {err}", path.display());
                }
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
        // Same reasoning as `shutdown`: never join on the UI thread.
        self.clean_up_files();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::{Event, EventListener};
    use alacritty_terminal::term::TermMode;
    use alacritty_terminal::vte::ansi::Processor;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<Event>>);

    impl EventListener for &Recorder {
        fn send_event(&self, event: Event) {
            self.0.lock().unwrap().push(event);
        }
    }

    /// The kitty keyboard protocol round trip apps rely on to detect
    /// shift+enter support: a query gets answered, a push sets the
    /// TermMode flag the key encoder reads.
    #[test]
    fn kitty_keyboard_protocol_round_trip() {
        let recorder = Recorder::default();
        let size = TermSize::new(80, 24);
        let mut term = Term::new(term_config(&SpawnOptions::default()), &size, &recorder);
        let mut parser: Processor = Processor::new();

        parser.advance(&mut term, b"\x1b[?u");
        let replies: Vec<String> = recorder
            .0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                Event::PtyWrite(text) => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(replies, vec!["\x1b[?0u".to_string()]);

        parser.advance(&mut term, b"\x1b[>1u");
        assert!(term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));

        parser.advance(&mut term, b"\x1b[<u");
        assert!(!term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));
    }
}

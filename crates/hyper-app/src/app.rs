use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::Event as TermEvent;
use alacritty_terminal::grid::Scroll;
use crossbeam_channel::{Receiver, Sender};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, ViewportCommand};
use hyper_config::{Command, KeyChord};
use hyper_term::{Session, SessionEvent, SessionId, SpawnOptions};

use hyper_config::{ConfigSnapshot, ConfigWatcher};

use crate::fonts::{self, CellMetrics};
use crate::grid_paint::GridPainter;
use crate::input;
use crate::term_view::{self, TermBehavior, TermViewState};
use crate::theme::TermTheme;
use crate::workspace::{NodeId, PaneNode, SplitDir, Workspace};

fn cursor_style_from(cfg: &hyper_config::ResolvedConfig) -> alacritty_terminal::vte::ansi::CursorStyle {
    use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle};
    let shape = match cfg.cursor_shape.to_uppercase().as_str() {
        "BEAM" => CursorShape::Beam,
        "UNDERLINE" => CursorShape::Underline,
        _ => CursorShape::Block,
    };
    CursorStyle {
        shape,
        blinking: cfg.cursor_blink,
    }
}

const TAB_BAR_HEIGHT: f32 = 34.0;
const SEPARATOR_WIDTH: f32 = 4.0;

/// `{shell, shellArgs}` as the fallback warning shows it.
fn describe_shell(shell: &str, args: &[String]) -> String {
    serde_json::json!({ "shell": shell, "shellArgs": args }).to_string()
}

/// Wakes the UI thread when a PTY event loop produces output.
struct EguiWaker(egui::Context);
impl hyper_term::Waker for EguiWaker {
    fn wake(&self) {
        self.0.request_repaint();
    }
}

struct SessionEntry {
    session: Session,
    view: TermViewState,
    bell_until: Option<Instant>,
    /// Directory the session was spawned in; a shell-fallback respawn reuses
    /// it rather than landing in the app process's cwd.
    spawn_cwd: Option<std::path::PathBuf>,
}

pub struct HyperApp {
    workspace: Workspace,
    sessions: HashMap<SessionId, SessionEntry>,
    event_rx: Receiver<SessionEvent>,
    event_tx: Sender<SessionEvent>,
    waker: Arc<EguiWaker>,
    config: ConfigSnapshot,
    watcher: Option<ConfigWatcher>,
    /// Profile whose resolved options currently drive theme/metrics.
    theme_profile: String,
    theme: TermTheme,
    padding: (f32, f32, f32, f32),
    metrics: Option<CellMetrics>,
    painter: GridPainter,
    clipboard: Option<arboard::Clipboard>,
    behavior: TermBehavior,
    key_options: input::KeyOptions,
    keymap: Vec<(KeyChord, Command)>,
    font_zoom: f32,
    pending_commands: Vec<Command>,
    search: crate::search_ui::SearchState,
    settings: crate::settings::SettingsWindow,
    palette: crate::palette::PaletteState,
    ipc: Option<crate::ipc::IpcServer>,
    menu: Option<crate::menu::AppMenu>,
    menu_generation: Option<u64>,
    /// (focused, blurred) window alpha from the resolved opacity option.
    opacity: (f32, f32),
    last_alpha: f32,
    bell_sound: bool,
    /// Last known window placement, persisted to windows-state.json on exit.
    last_geometry: Option<crate::session_restore::WindowGeometry>,
    /// Displays, captured by `MonitorProbe` once the event loop is running.
    monitors: crate::Monitors,
    /// The restored position has been checked against `monitors` (once).
    position_checked: bool,
    /// AltGr text egui never delivered, tapped from winit.
    altgr_rx: Receiver<String>,
    /// ssh:// URLs delivered via Apple Events.
    url_rx: Receiver<String>,
    /// Tab key events hidden from egui (see `raw_input_hook`) that still
    /// need to reach the palette or the PTY.
    stashed_tab_events: Vec<egui::Event>,
    /// A ctrl+click press was rewritten to a secondary click; its release
    /// must be rewritten too, even if ctrl was lifted in between.
    #[cfg(target_os = "macos")]
    ctrl_click_secondary: bool,
}

impl HyperApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        initial_paths: Vec<String>,
        ipc: Option<crate::ipc::IpcServer>,
        taps: crate::PlatformTaps,
    ) -> Self {
        fonts::install(&cc.egui_ctx);
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let waker = Arc::new(EguiWaker(cc.egui_ctx.clone()));

        let config = ConfigSnapshot::load(0);
        if let Some(err) = &config.parse_error {
            notify_parse_error(err);
        }
        let watcher = hyper_config::watch();

        let (url_tx, url_rx) = crossbeam_channel::unbounded();
        crate::macos::install_url_handler(url_tx);
        // `defaultSSHApp` defaults to true, as in config-default.json.
        crate::macos::sync_ssh_url_handler(config.root.default_ssh_app.unwrap_or(true));

        let mut app = HyperApp {
            workspace: Workspace::default(),
            sessions: HashMap::new(),
            event_rx,
            event_tx,
            waker,
            theme_profile: config.default_profile_name(),
            config,
            watcher,
            theme: TermTheme::default(),
            padding: (12.0, 14.0, 12.0, 14.0),
            metrics: None,
            painter: GridPainter::default(),
            clipboard: arboard::Clipboard::new().ok(),
            behavior: TermBehavior::default(),
            key_options: input::KeyOptions::default(),
            keymap: Vec::new(),
            font_zoom: 0.0,
            pending_commands: Vec::new(),
            search: crate::search_ui::SearchState::default(),
            settings: crate::settings::SettingsWindow::default(),
            palette: crate::palette::PaletteState::default(),
            ipc,
            menu: None,
            menu_generation: None,
            opacity: (1.0, 1.0),
            last_alpha: -1.0,
            bell_sound: true,
            last_geometry: None,
            monitors: taps.monitors,
            position_checked: false,
            altgr_rx: taps.altgr_rx,
            url_rx,
            stashed_tab_events: Vec::new(),
            #[cfg(target_os = "macos")]
            ctrl_click_secondary: false,
        };
        app.apply_config();

        // Initial tabs: CLI paths win, then restored session, else one default
        // tab (created lazily in logic()).
        for path in &initial_paths {
            app.open_path(std::path::Path::new(path));
        }
        if initial_paths.is_empty() && app.config.root.restore_session.unwrap_or(false) {
            if let Some(state) = crate::session_restore::read_session_state() {
                for tab in state.windows.iter().flat_map(|w| &w.tabs) {
                    // Skip profiles that no longer exist rather than spawning
                    // a default-config shell under a stale name.
                    if app.config.profile(&tab.profile).is_none() {
                        continue;
                    }
                    app.new_tab_at(
                        Some(&tab.profile.clone()),
                        tab.cwd.as_ref().map(std::path::PathBuf::from),
                    );
                }
                // Consumed — the state describes the last clean exit, so
                // leaving it in place means a crash (which never gets to write
                // fresh state) resurrects this same stale set of tabs forever.
                crate::session_restore::clear_session_state();
            }
        }

        // Debug affordances: open the settings window / palette at startup.
        if std::env::var_os("HYPER_OPEN_SETTINGS").is_some() {
            app.pending_commands.push(Command::Preferences);
        }
        if std::env::var_os("HYPER_OPEN_PALETTE").is_some() {
            app.palette.open = true;
        }
        app
    }

    /// (Re)derive everything view-related from the config snapshot for the
    /// current theme profile. Called at startup, on hot reload, and when the
    /// active tab's profile changes.
    fn apply_config(&mut self) {
        let resolved = self.config.resolved(Some(&self.theme_profile));
        self.theme = TermTheme::from_resolved(&resolved);
        self.padding = resolved.padding_px();
        self.behavior = TermBehavior {
            copy_on_select: resolved.copy_on_select,
            quick_edit: resolved.quick_edit,
            cursor_blink: resolved.cursor_blink,
            web_links_activation: resolved.web_links_activation_key.clone(),
        };
        self.key_options = input::KeyOptions {
            alt_is_meta: resolved.modifier_keys.alt_is_meta,
        };
        self.keymap = self.config.keymap.clone();
        self.opacity = (resolved.opacity.focused(), resolved.opacity.blurred());
        self.last_alpha = -1.0; // force re-apply
        self.bell_sound = resolved.bell.is_sound();
        self.metrics = None;
        self.painter.clear_cache();
        for entry in self.sessions.values_mut() {
            entry.view.last_grid = (0, 0);
        }
    }

    /// Hot reload: rebuild the snapshot and push option changes into every
    /// live terminal (scrollback, cursor style).
    fn reload_config(&mut self) {
        let generation = self.config.generation + 1;
        self.config = ConfigSnapshot::load(generation);
        if let Some(err) = &self.config.parse_error {
            notify_parse_error(err);
        }
        for entry in self.sessions.values_mut() {
            let resolved = self.config.resolved(Some(&entry.session.profile));
            let opts = SpawnOptions {
                scrollback: resolved.scrollback,
                cursor_style: Some(cursor_style_from(&resolved)),
                ..Default::default()
            };
            entry
                .session
                .term
                .lock()
                .set_options(hyper_term::session::term_config(&opts));
        }
        self.apply_config();
        log::info!("config reloaded (generation {generation})");
    }

    fn set_clipboard(&mut self, text: String) {
        if let Some(clipboard) = &mut self.clipboard {
            let _ = clipboard.set_text(text);
        }
    }

    fn clipboard_text(&mut self) -> String {
        self.clipboard
            .as_mut()
            .and_then(|c| c.get_text().ok())
            .unwrap_or_default()
    }

    /// Spawn a session for a profile (None ⇒ default). Working directory:
    /// active session's live cwd when preserveCWD and same profile, else the
    /// profile's workingDirectory, else home (`app/ui/window.ts` parity).
    fn spawn_session(&mut self, profile_name: Option<&str>) -> Option<SessionId> {
        self.spawn_session_at(profile_name, None)
    }

    /// Build spawn options for a profile: shell/args/env/scrollback, the ssh
    /// command for ssh profiles, and the working directory
    /// (`app/ui/window.ts` parity). Shared by new sessions and shell-fallback
    /// respawns so a respawn keeps the profile's env, cwd, and scrollback.
    fn spawn_options_for(
        &self,
        profile_name: &str,
        cwd: Option<std::path::PathBuf>,
    ) -> SpawnOptions {
        let resolved = self.config.resolved(Some(profile_name));
        let profile = self.config.profile(profile_name);

        let mut opts = SpawnOptions {
            shell: (!resolved.shell.is_empty()).then(|| resolved.shell.clone()),
            shell_args: resolved.shell_args.clone(),
            env: resolved.env.clone(),
            scrollback: resolved.scrollback,
            cursor_style: Some(cursor_style_from(&resolved)),
            profile: profile_name.to_string(),
            ..Default::default()
        };

        // SSH profiles spawn the system ssh binary inside the PTY.
        if let Some(profile) = profile {
            if profile.is_ssh() {
                if let Some(ssh) = &profile.ssh {
                    let ssh_opts = hyper_term::ssh::SshOptions {
                        host: ssh.host.clone(),
                        user: ssh.user.clone(),
                        port: ssh.port,
                        identity_file: ssh.identity_file.clone(),
                        forward_agent: ssh.forward_agent.unwrap_or(false),
                        extra_args: ssh.extra_args.clone().unwrap_or_default(),
                        // A stored password implies password auth even with no
                        // explicit authType, as in `window.ts`.
                        password_auth: ssh.uses_password_auth(),
                        password: ssh.password.clone(),
                    };
                    let askpass = std::env::current_exe().ok();
                    match hyper_term::ssh::build_ssh_command(&ssh_opts, askpass.as_deref()) {
                        Ok(cmd) => {
                            opts.shell = Some(cmd.program);
                            opts.shell_args = cmd.args;
                            opts.env.extend(cmd.env);
                            opts.cleanup_paths.extend(cmd.password_file);
                        }
                        Err(err) => log::error!("ssh command build failed: {err}"),
                    }
                }
            }
        }

        opts.working_directory = cwd.filter(|p| p.is_absolute() && p.is_dir());
        if opts.working_directory.is_none() {
            let same_profile_cwd = resolved.preserve_cwd.then(|| {
                self.workspace
                    .active_session()
                    .and_then(|id| self.sessions.get(&id))
                    .filter(|e| e.session.profile == profile_name)
                    .and_then(|e| e.session.cwd())
            });
            opts.working_directory = same_profile_cwd
                .flatten()
                .or_else(|| resolved.working_directory_path())
                .or_else(dirs::home_dir);
        }
        opts
    }

    fn spawn_session_at(
        &mut self,
        profile_name: Option<&str>,
        cwd: Option<std::path::PathBuf>,
    ) -> Option<SessionId> {
        let profile_name =
            profile_name.map(str::to_string).unwrap_or_else(|| self.config.default_profile_name());
        let opts = self.spawn_options_for(&profile_name, cwd);
        let spawn_cwd = opts.working_directory.clone();

        match Session::spawn(opts, self.event_tx.clone(), self.waker.clone()) {
            Ok(session) => {
                let id = session.id;
                self.sessions.insert(
                    id,
                    SessionEntry {
                        session,
                        view: TermViewState::default(),
                        bell_until: None,
                        spawn_cwd,
                    },
                );
                Some(id)
            }
            Err(err) => {
                log::error!("failed to spawn session: {err}");
                None
            }
        }
    }

    fn new_tab(&mut self) {
        self.new_tab_with_profile(None);
    }

    fn new_tab_with_profile(&mut self, profile: Option<&str>) {
        self.new_tab_at(profile, None);
    }

    fn new_tab_at(&mut self, profile: Option<&str>, cwd: Option<std::path::PathBuf>) {
        if let Some(id) = self.spawn_session_at(profile, cwd) {
            self.workspace.new_tab(id);
        }
    }

    /// Open a path handed over by the CLI, a second instance, or macOS
    /// `open-file`, with the same three behaviors as the Electron build's
    /// `openFile` action:
    ///
    /// - a directory opens a tab already in it (the original typed `cd <dir>`;
    ///   spawning there directly lands in the same place without the noise),
    /// - an executable file opens a tab and runs it,
    /// - any other file is typed at the prompt *without* a newline, so the
    ///   user decides what to do with it.
    ///
    /// A path that doesn't exist is reported rather than ignored.
    fn open_path(&mut self, path: &std::path::Path) {
        let Ok(meta) = std::fs::metadata(path) else {
            notify_open_failed(path);
            return;
        };

        if meta.is_dir() {
            self.new_tab_at(None, Some(path.to_path_buf()));
            return;
        }

        let mut command = escape_shell_cmd(&path.to_string_lossy());
        if is_executable(&meta) {
            command.push('\n');
        }
        self.new_tab();
        if let Some(entry) = self
            .workspace
            .active_session()
            .and_then(|id| self.sessions.get_mut(&id))
        {
            entry.session.write(command.into_bytes());
        }
    }

    fn split_active(&mut self, dir: SplitDir, profile: Option<&str>) {
        let Some(tab) = self.workspace.tabs.get(self.workspace.active_tab) else {
            return;
        };
        let at_leaf = tab.active_leaf;
        let profile = profile
            .map(str::to_string)
            .or_else(|| self.workspace.active_session().and_then(|id| {
                self.sessions.get(&id).map(|e| e.session.profile.clone())
            }));
        if let Some(id) = self.spawn_session(profile.as_deref()) {
            self.workspace.split(at_leaf, dir, id);
        }
    }

    /// Remove a pane and its session. Closes the window when nothing is left.
    fn close_pane(&mut self, leaf: NodeId, ctx: &egui::Context) {
        let session_id = match self.workspace.nodes.get(leaf) {
            Some(PaneNode::Leaf { session }) => Some(*session),
            _ => None,
        };
        self.workspace.remove(leaf);
        if let Some(sid) = session_id {
            if let Some(mut entry) = self.sessions.remove(&sid) {
                entry.session.shutdown();
            }
        }
        if self.workspace.tabs.is_empty() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn close_active_pane(&mut self, ctx: &egui::Context) {
        if let Some(tab) = self.workspace.tabs.get(self.workspace.active_tab) {
            self.close_pane(tab.active_leaf, ctx);
        }
    }

    /// Close a whole tab and every session in it (`userExitTermGroup`).
    ///
    /// Panes are re-read from the tree each round rather than snapshotted:
    /// removing one pane merges single-child parents and deletes those node
    /// ids, so a pre-collected list goes stale after the first close and the
    /// tab's last pane would survive with its shell still running. The tab is
    /// tracked by its root id because indices shift as tabs close.
    fn close_tab(&mut self, idx: usize, ctx: &egui::Context) {
        let Some(root) = self.workspace.tabs.get(idx).map(|tab| tab.root) else {
            return;
        };
        while self.workspace.tabs.iter().any(|tab| tab.root == root) {
            let Some(&leaf) = self.workspace.leaves_of(root).first() else {
                break;
            };
            self.close_pane(leaf, ctx);
        }
    }

    fn active_entry_mut(&mut self) -> Option<&mut SessionEntry> {
        let id = self.workspace.active_session()?;
        self.sessions.get_mut(&id)
    }

    fn zoom(&mut self, delta: Option<f32>) {
        self.font_zoom = match delta {
            Some(d) => (self.font_zoom + d).clamp(-8.0, 24.0),
            None => 0.0,
        };
        self.metrics = None; // re-measure next frame
        self.painter.clear_cache();
        for entry in self.sessions.values_mut() {
            entry.view.last_grid = (0, 0); // force PTY re-size
        }
    }

    fn execute(&mut self, cmd: Command, ctx: &egui::Context) {
        match cmd {
            Command::NewTab | Command::NewWindow => {
                // Multi-window lands in M7; treat window:new as tab:new.
                self.new_tab();
            }
            Command::NewTabProfile(name) | Command::NewWindowProfile(name) => {
                self.new_tab_with_profile(Some(&name));
            }
            Command::NextTab => {
                let n = self.workspace.tabs.len();
                if n > 0 {
                    self.workspace.active_tab = (self.workspace.active_tab + 1) % n;
                }
            }
            Command::PrevTab => {
                let n = self.workspace.tabs.len();
                if n > 0 {
                    self.workspace.active_tab = (self.workspace.active_tab + n - 1) % n;
                }
            }
            Command::JumpTab(n) => {
                let idx = (n as usize).saturating_sub(1);
                if idx < self.workspace.tabs.len() {
                    self.workspace.active_tab = idx;
                }
            }
            Command::JumpTabLast => {
                if !self.workspace.tabs.is_empty() {
                    self.workspace.active_tab = self.workspace.tabs.len() - 1;
                }
            }
            Command::SplitRight => self.split_active(SplitDir::Horizontal, None),
            Command::SplitRightProfile(name) => {
                self.split_active(SplitDir::Horizontal, Some(&name));
            }
            Command::SplitDown => self.split_active(SplitDir::Vertical, None),
            Command::SplitDownProfile(name) => {
                self.split_active(SplitDir::Vertical, Some(&name));
            }
            Command::ClosePane => self.close_active_pane(ctx),
            Command::NextPane => self.workspace.cycle_pane(true),
            Command::PrevPane => self.workspace.cycle_pane(false),
            Command::CloseWindow => ctx.send_viewport_cmd(ViewportCommand::Close),
            Command::MinimizeWindow => {
                ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
            }
            Command::ZoomWindow | Command::ToggleFullScreen => {
                let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fullscreen));
            }
            Command::ZoomIn => self.zoom(Some(1.0)),
            Command::ZoomOut => self.zoom(Some(-1.0)),
            Command::ZoomReset => self.zoom(None),
            Command::Copy => {
                if let Some(entry) = self.active_entry_mut() {
                    let text = entry
                        .view
                        .selection_snapshot
                        .clone()
                        .or_else(|| entry.session.term.lock().selection_to_string());
                    if let Some(text) = text {
                        if !text.is_empty() {
                            self.set_clipboard(text);
                        }
                    }
                }
            }
            Command::Paste => {
                let text = self.clipboard_text();
                if let Some(entry) = self.active_entry_mut() {
                    if !text.is_empty() {
                        let mode = *entry.session.term.lock().mode();
                        entry.session.write(input::encode_paste(&text, mode));
                    }
                }
            }
            Command::SelectAll => {
                if let Some(entry) = self.active_entry_mut() {
                    use alacritty_terminal::grid::Dimensions;
                    use alacritty_terminal::index::{Column, Point, Side};
                    use alacritty_terminal::selection::{Selection, SelectionType};

                    let mut term = entry.session.term.lock();
                    // Span the whole buffer: from the top of the scrollback
                    // (`topmost_line` is negative by the history size) to the
                    // last column of the last screen line. `include_all` only
                    // adjusts the start/end *sides* — it does not grow the
                    // region — so anchoring at Line(0) selected just the first
                    // visible line.
                    let start = Point::new(term.topmost_line(), Column(0));
                    let end = Point::new(
                        term.bottommost_line(),
                        Column(term.columns().saturating_sub(1)),
                    );
                    let mut selection = Selection::new(SelectionType::Lines, start, Side::Left);
                    selection.update(end, Side::Right);
                    selection.include_all();
                    term.selection = Some(selection);
                }
            }
            Command::ClearBuffer => {
                if let Some(entry) = self.active_entry_mut() {
                    entry
                        .session
                        .term
                        .lock()
                        .grid_mut()
                        .clear_history();
                    entry.session.write(b"\x0c".to_vec());
                }
            }
            Command::PtyBytes(bytes) => {
                if let Some(entry) = self.active_entry_mut() {
                    entry.session.write(bytes.to_vec());
                }
            }
            Command::Search => self.search.open(),
            Command::SearchClose => {
                let session = self
                    .workspace
                    .active_session()
                    .and_then(|id| self.sessions.get(&id))
                    .map(|e| &e.session);
                self.search.close(session);
            }
            Command::Preferences => self.settings.open_window(&self.config),
            Command::OpenConfigFile => {
                let _ = open::that(hyper_config::paths::cfg_path());
            }
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        while let Ok(SessionEvent { id, event }) = self.event_rx.try_recv() {
            match event {
                TermEvent::Wakeup => {}
                TermEvent::Title(title) => {
                    if let Some(entry) = self.sessions.get_mut(&id) {
                        entry.session.title = title;
                    }
                }
                TermEvent::ResetTitle => {
                    if let Some(entry) = self.sessions.get_mut(&id) {
                        entry.session.title.clear();
                    }
                }
                TermEvent::PtyWrite(text) => {
                    if let Some(entry) = self.sessions.get_mut(&id) {
                        entry.session.write(text.into_bytes());
                    }
                }
                TermEvent::ClipboardStore(_, text) => {
                    self.set_clipboard(text);
                }
                TermEvent::ClipboardLoad(_, format) => {
                    let text = self.clipboard_text();
                    if let Some(entry) = self.sessions.get_mut(&id) {
                        entry.session.write(format(&text).into_bytes());
                    }
                }
                TermEvent::ColorRequest(index, format) => {
                    if let Some(entry) = self.sessions.get_mut(&id) {
                        // A program that has set a color with OSC 4/10/11 must
                        // read that color back, not the theme's. Answering from
                        // the static palette tells anything that saves and
                        // restores colors — vim, tmux — to restore the wrong
                        // ones, which is how a theme "leaks" between sessions.
                        let overridden = entry.session.term.lock().colors()[index];
                        let rgb = overridden.unwrap_or_else(|| {
                            let color = self.theme.palette[index.min(268)];
                            alacritty_terminal::vte::ansi::Rgb {
                                r: color.r(),
                                g: color.g(),
                                b: color.b(),
                            }
                        });
                        entry.session.write(format(rgb).into_bytes());
                    }
                }
                TermEvent::TextAreaSizeRequest(format) => {
                    if let Some(entry) = self.sessions.get_mut(&id) {
                        let (cols, rows) = entry.view.last_grid;
                        let metrics = self.metrics.as_ref();
                        let size = alacritty_terminal::event::WindowSize {
                            num_lines: rows.max(1),
                            num_cols: cols.max(1),
                            cell_width: metrics.map_or(8, |m| m.width as u16),
                            cell_height: metrics.map_or(16, |m| m.height as u16),
                        };
                        entry.session.write(format(size).into_bytes());
                    }
                }
                TermEvent::Bell => {
                    if let Some(entry) = self.sessions.get_mut(&id) {
                        entry.bell_until = Some(Instant::now() + Duration::from_millis(150));
                    }
                    if self.bell_sound {
                        crate::macos::beep();
                    }
                    ctx.request_repaint();
                }
                TermEvent::MouseCursorDirty | TermEvent::CursorBlinkingChange => {}
                TermEvent::ChildExit(status) => {
                    self.handle_child_exit(id, status.code(), ctx);
                }
                TermEvent::Exit => {}
            }
        }
    }

    /// Shell fallback (port of `session.ts`'s `onExit` + `shell-fallback.ts`):
    /// *any* shell that exits non-zero within 1s is retried with a safer
    /// configuration — first the same shell without its args, then the system
    /// default shell — instead of the pane vanishing. This is what surfaces a
    /// broken shell config rather than an instantly-closing window, so it must
    /// fire for the default shell too (a `~/.zprofile` that exits non-zero
    /// would otherwise make every new tab close immediately).
    fn handle_child_exit(&mut self, id: SessionId, code: Option<i32>, ctx: &egui::Context) {
        let died_fast = self.sessions.get(&id).is_some_and(|entry| {
            code.is_some_and(|c| c > 0) && entry.session.started.elapsed() < Duration::from_secs(1)
        });

        if died_fast {
            let entry = self.sessions.get(&id).unwrap();
            let elapsed = entry.session.started.elapsed().as_millis();
            let bad_shell = entry.session.spawned_shell.clone();
            let bad_args = entry.session.spawned_shell_args.clone();
            let term = entry.session.term.clone();
            let profile = entry.session.profile.clone();
            let cwd = entry.spawn_cwd.clone();
            let (cols, rows) = entry.view.last_grid;
            let exit_code = code.unwrap_or(0);

            let fallback = hyper_term::session::fallback_shell_config(
                &bad_shell,
                &bad_args,
                &hyper_term::env::default_shell(),
            );

            let Some((shell, shell_args)) = fallback else {
                // Nothing left to try: report and leave the pane open, as the
                // Electron build did (it never emitted 'exit' in this branch).
                let msg = format!(
                    "\r\n\x1b[33mshell exited in {elapsed} ms with exit code {exit_code}\r\n\
                     No fallback available, please check the shell config.\x1b[0m\r\n\r\n"
                );
                self.write_to_term(&term, &msg);
                return;
            };

            let msg = format!(
                "\r\n\x1b[33mshell exited in {elapsed} ms with exit code {exit_code}\r\n\
                 please check the shell config: {}\r\n\
                 using fallback shell config: {}\x1b[0m\r\n\r\n",
                describe_shell(&bad_shell, &bad_args),
                describe_shell(&shell, &shell_args),
            );
            self.write_to_term(&term, &msg);

            let mut opts = self.spawn_options_for(&profile, cwd.clone());
            opts.shell = Some(shell);
            opts.shell_args = shell_args;
            opts.window_size = alacritty_terminal::event::WindowSize {
                num_lines: rows.max(2),
                num_cols: cols.max(2),
                cell_width: self.metrics.as_ref().map_or(8, |m| m.width as u16),
                cell_height: self.metrics.as_ref().map_or(16, |m| m.height as u16),
            };
            let spawn_cwd = opts.working_directory.clone();

            match Session::spawn_with(id, Some(term), opts, self.event_tx.clone(), self.waker.clone())
            {
                Ok(new_session) => {
                    let entry = self.sessions.get_mut(&id).unwrap();
                    let mut old = std::mem::replace(&mut entry.session, new_session);
                    entry.spawn_cwd = spawn_cwd;
                    old.shutdown();
                    return;
                }
                Err(err) => log::error!("shell fallback respawn failed: {err}"),
            }
        }

        if let Some(leaf) = self.workspace.find_leaf(id) {
            self.close_pane(leaf, ctx);
        } else if let Some(mut entry) = self.sessions.remove(&id) {
            entry.session.shutdown();
        }
    }

    /// Feed text straight into a terminal grid (warnings the app itself
    /// prints, never sent to the PTY).
    fn write_to_term(&self, term: &hyper_term::SharedTerm, text: &str) {
        let mut processor: alacritty_terminal::vte::ansi::Processor =
            alacritty_terminal::vte::ansi::Processor::new();
        let mut term = term.lock();
        processor.advance(&mut *term, text.as_bytes());
    }

    /// Keyboard: app keymap first (consuming), then terminal encoding.
    fn handle_input(&mut self, ctx: &egui::Context) {
        // 1. App commands.
        let keymap = self.keymap.clone();
        for (chord, cmd) in &keymap {
            // Esc must reach the terminal unless the search overlay is open.
            if *cmd == Command::SearchClose && !self.search.open {
                continue;
            }
            for _ in 0..input::consume_chord(ctx, chord) {
                self.pending_commands.push(cmd.clone());
            }
        }
        for cmd in std::mem::take(&mut self.pending_commands) {
            self.execute(cmd, ctx);
        }

        // An egui widget (search field, settings…) owns the keyboard.
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }

        // Tab events hidden from egui in raw_input_hook: navigation while the
        // palette is open, PTY completion otherwise.
        let stashed_tabs = std::mem::take(&mut self.stashed_tab_events);
        let tab_pressed = stashed_tabs.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key {
                    key: egui::Key::Tab,
                    pressed: true,
                    modifiers,
                    ..
                } if !modifiers.any()
            )
        });
        let palette_had_tab = self.palette.open && tab_pressed;

        // Command palette: `/` at the start of a prompt line. Runs before
        // terminal encoding; consumes only its navigation keys.
        let profile_names: Vec<String> = self
            .config
            .root
            .profiles
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let action = self.palette.handle_input(ctx, &profile_names, true, tab_pressed);
        if let Some(cmd) = action.run {
            self.run_palette_command(cmd, ctx);
        }

        // 2. Terminal input for the focused pane.
        let alt_is_meta = self.key_options.alt_is_meta;
        let Some(active) = self.workspace.active_session() else {
            return;
        };
        let Some(entry) = self.sessions.get_mut(&active) else {
            return;
        };

        let mode = *entry.session.term.lock().mode();
        let mut events = ctx.input(|i| i.events.clone());
        if !palette_had_tab {
            events.extend(stashed_tabs);
        }
        // AltGr characters egui dropped before we ever saw them, recovered
        // from winit by `MonitorProbe` and rejoined here as ordinary text.
        let altgr: Vec<String> = self.altgr_rx.try_iter().collect();
        // Whether this frame's ctrl+alt was AltGr composing a character. A
        // layout that composes nothing (ctrl+alt+A on US) sends no text, so
        // real ctrl+alt chords still reach the encoder below.
        let altgr_composed = !altgr.is_empty();
        events.extend(altgr.into_iter().map(egui::Event::Text));
        let frame_mods = ctx.input(|i| i.modifiers);
        let mut wrote = false;
        let mut copy_requested = false;

        for event in events {
            match event {
                egui::Event::Text(text) => {
                    // Testing `command` here would mean ctrl off macOS, which
                    // is half of AltGr — and egui-winit has already dropped
                    // the text for every real ctrl/cmd chord, so anything
                    // arriving with ctrl held is AltGr text recovered above
                    // and must pass through.
                    let cmd_held = frame_mods.mac_cmd;
                    // Likewise `altIsMeta` claims plain alt, not ctrl+alt:
                    // xterm.js takes its meta branch on `altKey && !ctrlKey`.
                    let alt_consumed = alt_is_meta && frame_mods.alt && !frame_mods.ctrl;
                    if !cmd_held && !alt_consumed {
                        entry.session.write(text.into_bytes());
                        wrote = true;
                    }
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    // shift+PageUp/PageDown scroll the scrollback rather than
                    // reaching the shell, as in xterm.js (only ctrl-modified
                    // page keys are encoded and sent).
                    if modifiers.shift
                        && !modifiers.ctrl
                        && !modifiers.alt
                        && matches!(key, egui::Key::PageUp | egui::Key::PageDown)
                    {
                        let scroll = if key == egui::Key::PageUp {
                            Scroll::PageUp
                        } else {
                            Scroll::PageDown
                        };
                        entry.session.term.lock().scroll_display(scroll);
                        continue;
                    }
                    // AltGr just composed a character that is already on its
                    // way as text. Encoding the same chord again would type
                    // e.g. German AltGr+8 as "[" *and* ctrl+[ — an ESC.
                    if altgr_composed && modifiers.ctrl && modifiers.alt {
                        continue;
                    }
                    if alt_is_meta && modifiers.alt && !modifiers.mac_cmd && !modifiers.ctrl {
                        if let Some(bytes) = input::alt_meta_bytes(key, modifiers, mode) {
                            entry.session.write(bytes);
                            wrote = true;
                            continue;
                        }
                    }
                    if let Some(bytes) = input::encode_key(key, modifiers, mode) {
                        if std::env::var_os("HYPER_DEBUG_INPUT").is_some() {
                            eprintln!(
                                "[input] encode {key:?} -> {} (kitty={})",
                                String::from_utf8_lossy(&bytes).escape_debug(),
                                mode.contains(
                                    alacritty_terminal::term::TermMode::DISAMBIGUATE_ESC_CODES
                                )
                            );
                        }
                        entry.session.write(bytes);
                        wrote = true;
                    }
                }
                egui::Event::Paste(text) => {
                    entry.session.write(input::encode_paste(&text, mode));
                    wrote = true;
                }
                egui::Event::Copy => copy_requested = true,
                _ => {}
            }
        }

        if wrote {
            entry.session.term.lock().scroll_display(Scroll::Bottom);
        }

        if copy_requested {
            let text = entry
                .view
                .selection_snapshot
                .clone()
                .or_else(|| entry.session.term.lock().selection_to_string());
            if let Some(text) = text {
                if !text.is_empty() {
                    self.set_clipboard(text);
                }
            }
        }
    }

    /// Palette runner: wipe the typed `/xyz` from the prompt line (\x15),
    /// then execute the command.
    fn run_palette_command(&mut self, cmd: Command, ctx: &egui::Context) {
        if let Some(entry) = self
            .workspace
            .active_session()
            .and_then(|id| self.sessions.get_mut(&id))
        {
            entry.session.write(b"\x15".to_vec());
        }
        self.execute(cmd, ctx);
    }

    fn sync_window_title(&self, ctx: &egui::Context) {
        let title = self
            .workspace
            .active_session()
            .and_then(|id| self.sessions.get(&id))
            .map(|e| e.session.title.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "Hyper Revamp".to_string());
        ctx.send_viewport_cmd(ViewportCommand::Title(title));
    }

    // ---- rendering ----

    fn tab_bar(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let theme = &self.theme;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme.background);
        painter.line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0, theme.border),
        );

        // With the native titlebar hidden, the bar is the window's drag
        // handle; tabs and buttons interact after this and win hit-testing.
        let drag_resp = ui.interact(rect, ui.id().with("tabbar-drag"), Sense::click_and_drag());
        if drag_resp.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if drag_resp.double_clicked() {
            let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }

        let n = self.workspace.tabs.len();
        let mut clicked_tab = None;
        let mut closed_tab = None;
        let mut new_tab_clicked = false;

        // macOS traffic-light inset.
        let left_inset: f32 = if cfg!(target_os = "macos") { 76.0 } else { 8.0 };
        let plus_w = 34.0;
        let avail = rect.width() - left_inset - plus_w;
        let tab_w = (avail / n.max(1) as f32).clamp(60.0, 220.0);

        for idx in 0..n {
            let tab_rect = Rect::from_min_size(
                Pos2::new(rect.min.x + left_inset + idx as f32 * tab_w, rect.min.y),
                Vec2::new(tab_w, rect.height()),
            );
            let id = ui.id().with(("tab", idx));
            let response = ui.interact(tab_rect, id, Sense::click());
            let active = idx == self.workspace.active_tab;

            if active {
                painter.rect_filled(
                    Rect::from_min_max(
                        tab_rect.min,
                        Pos2::new(tab_rect.max.x, tab_rect.max.y - 1.0),
                    ),
                    0.0,
                    theme.background.gamma_multiply(0.85).to_opaque(),
                );
                painter.line_segment(
                    [
                        Pos2::new(tab_rect.min.x, tab_rect.max.y - 1.5),
                        Pos2::new(tab_rect.max.x, tab_rect.max.y - 1.5),
                    ],
                    Stroke::new(2.0, theme.cursor),
                );
            }
            if idx > 0 {
                painter.line_segment(
                    [
                        Pos2::new(tab_rect.min.x, tab_rect.min.y + 8.0),
                        Pos2::new(tab_rect.min.x, tab_rect.max.y - 8.0),
                    ],
                    Stroke::new(1.0, theme.border),
                );
            }

            // The tab's *active* pane names it (`activeSessions[rootGroup.uid]`
            // in `lib/containers/header.ts`), not whichever pane happens to be
            // first — otherwise splitting a tab freezes its title forever.
            let tab_session = self
                .workspace
                .tabs
                .get(idx)
                .and_then(|tab| match self.workspace.nodes.get(tab.active_leaf) {
                    Some(PaneNode::Leaf { session }) => Some(*session),
                    _ => self.workspace.sessions_of_tab(idx).first().copied(),
                })
                .and_then(|sid| self.sessions.get(&sid));

            let title = tab_session
                .map(|e| {
                    if e.session.title.is_empty() {
                        "Shell".to_string()
                    } else {
                        e.session.title.clone()
                    }
                })
                .unwrap_or_else(|| "Shell".into());

            // Per-profile accent strip (`profiles[].color`), as drawn by
            // `lib/components/tab.tsx`.
            let accent = tab_session
                .map(|e| e.session.profile.as_str())
                .and_then(|profile| self.config.profile(profile))
                .and_then(|profile| profile.color.as_deref())
                .and_then(hyper_config::parse_color)
                .map(|[r, g, b, a]| Color32::from_rgba_unmultiplied(r, g, b, a));
            if let Some(accent) = accent {
                let strip = Rect::from_min_max(
                    Pos2::new(tab_rect.min.x, tab_rect.min.y),
                    Pos2::new(tab_rect.max.x, tab_rect.min.y + 2.0),
                );
                painter.rect_filled(
                    strip,
                    0.0,
                    if active {
                        accent
                    } else {
                        accent.gamma_multiply(0.55)
                    },
                );
            }

            let text_color = if active {
                theme.foreground
            } else {
                theme.foreground.gamma_multiply(0.55)
            };
            painter.text(
                tab_rect.center(),
                egui::Align2::CENTER_CENTER,
                truncate_title(&title, tab_w),
                egui::FontId::proportional(12.0),
                text_color,
            );

            // Close button while the pointer is anywhere over the tab (not
            // response.hovered(): the close widget itself steals hover, which
            // would make the button vanish as the pointer reaches it).
            if ui.rect_contains_pointer(tab_rect) {
                let close_rect = Rect::from_center_size(
                    Pos2::new(tab_rect.min.x + 14.0, tab_rect.center().y),
                    Vec2::splat(16.0),
                );
                let close_resp =
                    ui.interact(close_rect, id.with("close"), Sense::click());
                let color = if close_resp.hovered() {
                    theme.foreground
                } else {
                    theme.foreground.gamma_multiply(0.5)
                };
                // Drawn, not a glyph: the bundled fonts have no ✕.
                let cross = Rect::from_center_size(close_rect.center(), Vec2::splat(7.0));
                painter.line_segment(
                    [cross.left_top(), cross.right_bottom()],
                    Stroke::new(1.3, color),
                );
                painter.line_segment(
                    [cross.right_top(), cross.left_bottom()],
                    Stroke::new(1.3, color),
                );
                if close_resp.clicked() {
                    closed_tab = Some(idx);
                }
            }

            if response.clicked() && closed_tab.is_none() {
                clicked_tab = Some(idx);
            }
        }

        // "+" button.
        let plus_rect = Rect::from_min_size(
            Pos2::new(rect.min.x + left_inset + n as f32 * tab_w, rect.min.y),
            Vec2::new(plus_w, rect.height()),
        );
        let plus_resp = ui.interact(plus_rect, ui.id().with("tab-plus"), Sense::click());
        painter.text(
            plus_rect.center(),
            egui::Align2::CENTER_CENTER,
            "+",
            egui::FontId::proportional(16.0),
            if plus_resp.hovered() {
                theme.foreground
            } else {
                theme.foreground.gamma_multiply(0.55)
            },
        );
        if plus_resp.clicked() {
            new_tab_clicked = true;
        }

        if let Some(idx) = clicked_tab {
            self.workspace.active_tab = idx;
        }
        if let Some(idx) = closed_tab {
            let ctx = ui.ctx().clone();
            self.close_tab(idx, &ctx);
        }
        if new_tab_clicked {
            self.new_tab();
        }
    }

    /// Compute leaf rects and separator rects for the active tab's tree.
    fn layout_tree(
        &self,
        node: NodeId,
        rect: Rect,
        leaves: &mut Vec<(NodeId, Rect)>,
        seps: &mut Vec<(NodeId, usize, Rect, SplitDir, f32)>,
    ) {
        match &self.workspace.nodes[node] {
            PaneNode::Leaf { .. } => leaves.push((node, rect)),
            PaneNode::Split {
                dir,
                children,
                sizes,
            } => {
                let n = children.len();
                let sep_total = SEPARATOR_WIDTH * (n - 1) as f32;
                let span = match dir {
                    SplitDir::Horizontal => rect.width(),
                    SplitDir::Vertical => rect.height(),
                } - sep_total;
                let mut offset = 0.0;
                for (i, (child, size)) in children.iter().zip(sizes).enumerate() {
                    let extent = span * size;
                    let child_rect = match dir {
                        SplitDir::Horizontal => Rect::from_min_size(
                            Pos2::new(rect.min.x + offset, rect.min.y),
                            Vec2::new(extent, rect.height()),
                        ),
                        SplitDir::Vertical => Rect::from_min_size(
                            Pos2::new(rect.min.x, rect.min.y + offset),
                            Vec2::new(rect.width(), extent),
                        ),
                    };
                    self.layout_tree(*child, child_rect, leaves, seps);
                    offset += extent;
                    if i + 1 < n {
                        let sep_rect = match dir {
                            SplitDir::Horizontal => Rect::from_min_size(
                                Pos2::new(rect.min.x + offset, rect.min.y),
                                Vec2::new(SEPARATOR_WIDTH, rect.height()),
                            ),
                            SplitDir::Vertical => Rect::from_min_size(
                                Pos2::new(rect.min.x, rect.min.y + offset),
                                Vec2::new(rect.width(), SEPARATOR_WIDTH),
                            ),
                        };
                        seps.push((node, i, sep_rect, *dir, span));
                        offset += SEPARATOR_WIDTH;
                    }
                }
            }
        }
    }
}

fn notify_parse_error(err: &str) {
    log::warn!("config parse error: {err}");
    let _ = notify_rust::Notification::new()
        .summary("Hyper Revamp")
        .body(&format!(
            "Couldn't parse the config file — using defaults.\n{err}"
        ))
        .show();
}

/// `notify('Unable to open path', '"<path>" doesn\'t exist.')`.
fn notify_open_failed(path: &std::path::Path) {
    log::warn!("can't open {}: no such path", path.display());
    let _ = notify_rust::Notification::new()
        .summary("Unable to open path")
        .body(&format!("\"{}\" doesn't exist.", path.display()))
        .show();
}

/// Whether a file would run if typed at a prompt, matching `utils/file.ts`:
/// any of the three execute bits, and always true on Windows.
fn is_executable(meta: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        true
    }
}

/// Port of PHP's `escapeshellcmd` (the `php-escape-shell` package the original
/// fed `openFile` paths through) plus its follow-up space escaping.
///
/// This backslash-escapes shell metacharacters so that opening a file called
/// `report;rm -rf ~.txt` types a harmless literal path rather than handing the
/// shell a second command to run. Quotes are escaped unconditionally, where
/// PHP leaves *balanced* pairs alone — for a path being typed at a prompt the
/// stricter rule is the correct one, and the pairing exemption is what makes
/// `escapeshellcmd` unsafe on its own in PHP.
fn escape_shell_cmd(path: &str) -> String {
    const METACHARACTERS: &[char] = &[
        '#', '&', ';', '`', '|', '*', '?', '~', '<', '>', '^', '(', ')', '[', ']', '{', '}', '$',
        '\\', '\x0a', '\u{ff}', '\'', '"', ' ',
    ];
    let mut out = String::with_capacity(path.len());
    for ch in path.chars() {
        if METACHARACTERS.contains(&ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn truncate_title(title: &str, width: f32) -> String {
    let max_chars = ((width - 40.0) / 7.0).max(4.0) as usize;
    if title.chars().count() > max_chars {
        let truncated: String = title.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{truncated}…")
    } else {
        title.to_string()
    }
}

impl eframe::App for HyperApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Diagnostics: HYPER_DEBUG_INPUT=1 dumps raw pointer/key events.
        if std::env::var_os("HYPER_DEBUG_INPUT").is_some() {
            for event in &raw_input.events {
                match event {
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        ..
                    } => eprintln!("[input] {button:?} pressed={pressed} at {pos:?}"),
                    egui::Event::Key {
                        key,
                        pressed,
                        repeat,
                        modifiers,
                        ..
                    } => eprintln!(
                        "[input] key {key:?} pressed={pressed} repeat={repeat} mods={modifiers:?}"
                    ),
                    egui::Event::Text(text) => eprintln!("[input] text {text:?}"),
                    _ => {}
                }
            }
        }

        // ctrl+click = right-click, per macOS convention. Skipped when links
        // are configured to open on ctrl+click.
        #[cfg(target_os = "macos")]
        if raw_input.viewport_id == egui::ViewportId::ROOT
            && self.behavior.web_links_activation != "ctrl"
        {
            input::rewrite_ctrl_click_as_secondary(
                &mut raw_input.events,
                &mut self.ctrl_click_secondary,
            );
        }
        // Tab must reach the shell (completion!), not egui's focus traversal —
        // otherwise a chrome button gains focus and the "widget owns the
        // keyboard" guard mutes terminal input for good. Hide Tab from egui
        // unless a widget (search field, settings editor) currently has focus;
        // the stashed events are fed to the palette / PTY in handle_input.
        //
        // Tab combinations the keymap binds (ctrl+tab / ctrl+shift+tab cycle
        // tabs, and are the *only* tab-switch bindings on Windows) stay in the
        // queue so `handle_input` can dispatch them — stashing those made
        // every Tab binding dead and typed a Tab into the shell instead.
        // egui's own focus traversal only reacts to unmodified/shift Tab, so
        // leaving modified ones in place is safe.
        if raw_input.viewport_id == egui::ViewportId::ROOT
            && ctx.memory(|m| m.focused().is_none())
        {
            let tab_chords: Vec<&KeyChord> = self
                .keymap
                .iter()
                .map(|(chord, _)| chord)
                .filter(|chord| chord.key == "tab" && input::chord_is_bindable(chord))
                .collect();
            let stash = &mut self.stashed_tab_events;
            raw_input.events.retain(|event| {
                let egui::Event::Key {
                    key: egui::Key::Tab,
                    modifiers,
                    ..
                } = event
                else {
                    return true;
                };
                if tab_chords
                    .iter()
                    .any(|chord| input::chord_matches(chord, *modifiers))
                {
                    return true;
                }
                stash.push(event.clone());
                false
            });
        }
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Config hot reload.
        if let Some(watcher) = &self.watcher {
            if watcher.rx.try_recv().is_ok() {
                while watcher.rx.try_recv().is_ok() {}
                self.reload_config();
            }
        }

        // Native menu bar: install once, rebuild when the config changes
        // (accelerators come from the keymap, profiles feed the Shell menu).
        if self.menu_generation != Some(self.config.generation) {
            self.menu_generation = Some(self.config.generation);
            let profiles: Vec<String> = self
                .config
                .root
                .profiles
                .iter()
                .map(|p| p.name.clone())
                .collect();
            self.menu = crate::menu::AppMenu::install(&self.keymap, &profiles);
        }
        if let Some(menu) = &self.menu {
            let cmds = menu.poll();
            self.pending_commands.extend(cmds);
        }

        // Second-instance handoff: open a tab per path, or just focus.
        let ipc_msgs: Vec<crate::ipc::IpcMessage> = self
            .ipc
            .as_ref()
            .map(|s| s.rx.try_iter().collect())
            .unwrap_or_default();
        for msg in ipc_msgs {
            if msg.paths.is_empty() {
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
            for path in msg.paths {
                self.open_path(std::path::Path::new(&path));
                ctx.send_viewport_cmd(ViewportCommand::Focus);
            }
        }

        // ssh:// URLs: open a fresh tab and type the ssh command into it
        // (Electron `openSSH` parity).
        let urls: Vec<String> = self.url_rx.try_iter().collect();
        for url in urls {
            let Some(command) = crate::macos::ssh_url_to_command(&url) else {
                log::warn!("ignoring malformed ssh url: {url}");
                continue;
            };
            self.new_tab();
            if let Some(entry) = self
                .workspace
                .active_session()
                .and_then(|id| self.sessions.get_mut(&id))
            {
                entry.session.write(command.into_bytes());
            }
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }

        // Window opacity (focus/blur pair) via NSWindow.alphaValue.
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let alpha = if focused {
            self.opacity.0
        } else {
            self.opacity.1
        };
        if (alpha - self.last_alpha).abs() > f32::EPSILON {
            self.last_alpha = alpha;
            crate::macos::set_window_alpha(alpha, "Hyper Revamp Settings");
        }

        // Track geometry for windows-state.json. The position is the outer
        // top-left (what `with_position` takes) but the size is the *inner*
        // one (what `with_inner_size` takes) — recording the outer size here
        // and restoring it as an inner size grows the window by the frame's
        // height on every launch.
        let (outer, inner) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
        if let (Some(outer), Some(inner)) = (outer, inner) {
            self.last_geometry = Some(crate::session_restore::WindowGeometry {
                window_position: [outer.min.x, outer.min.y],
                window_size: [inner.width(), inner.height()],
            });

            // The saved position may name a display that has since been
            // unplugged, leaving the window somewhere unreachable. winit only
            // lists displays once the event loop runs, so unlike Electron's
            // up-front `positionIsValid` this corrects a frame late.
            if !self.position_checked {
                if let Some(monitors) = self.monitors.get() {
                    self.position_checked = true;
                    if !crate::session_restore::position_is_valid(
                        [outer.min.x, outer.min.y],
                        monitors,
                    ) {
                        log::info!("restored window position is off every display; recentering");
                        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
                            crate::session_restore::DEFAULT_POSITION.into(),
                        ));
                    }
                }
            }
        }

        // Theme follows the active tab's profile (Hyper parity).
        if let Some(profile) = self
            .workspace
            .active_session()
            .and_then(|id| self.sessions.get(&id))
            .map(|e| e.session.profile.clone())
        {
            if profile != self.theme_profile {
                self.theme_profile = profile;
                self.apply_config();
            }
        }

        if self.metrics.is_none() {
            self.metrics = Some(fonts::measure(
                ctx,
                (self.theme.font_size + self.font_zoom).max(6.0),
                self.theme.line_height,
                self.theme.letter_spacing,
            ));
        }

        if self.workspace.tabs.is_empty() && self.sessions.is_empty() {
            self.new_tab();
        }

        self.drain_events(ctx);
        self.handle_input(ctx);
        self.sync_window_title(ctx);
        self.settings.show(ctx, &self.config);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let Some(metrics) = self.metrics.clone() else {
            return;
        };
        let theme = self.theme.clone();
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, theme.background);

        // Tab bar.
        let bar_rect = Rect::from_min_size(full.min, Vec2::new(full.width(), TAB_BAR_HEIGHT));
        self.tab_bar(ui, bar_rect);

        let content = Rect::from_min_max(
            Pos2::new(full.min.x, full.min.y + TAB_BAR_HEIGHT),
            full.max,
        );

        let Some(tab) = self.workspace.tabs.get(self.workspace.active_tab) else {
            return;
        };
        let root = tab.root;
        let active_leaf = tab.active_leaf;

        let mut leaves = Vec::new();
        let mut seps = Vec::new();
        self.layout_tree(root, content, &mut leaves, &mut seps);

        // Separators: paint + drag-resize.
        for (split, index, sep_rect, dir, span) in seps {
            let id = ui.id().with(("sep", split, index));
            // `click()` as well as `drag()`: a double-click is a click pair,
            // and a drag-only sense never reports one.
            let response = ui.interact(sep_rect, id, Sense::click_and_drag());
            let hovered = response.hovered() || response.dragged();
            ui.painter().rect_filled(
                sep_rect,
                0.0,
                if hovered {
                    theme.cursor.gamma_multiply(0.6)
                } else {
                    theme.border
                },
            );
            if hovered {
                ui.ctx().set_cursor_icon(match dir {
                    SplitDir::Horizontal => egui::CursorIcon::ResizeHorizontal,
                    SplitDir::Vertical => egui::CursorIcon::ResizeVertical,
                });
            }
            if response.double_clicked() {
                self.workspace.equalize_split(split, index);
            } else if response.dragged() && span > 0.0 {
                let delta = match dir {
                    SplitDir::Horizontal => response.drag_delta().x,
                    SplitDir::Vertical => response.drag_delta().y,
                } / span;
                self.workspace.resize_split(split, index, delta);
            }
        }

        // Panes.
        let behavior = self.behavior.clone();
        let padding = self.padding;
        let multiple_panes = leaves.len() > 1;
        let mut focus_leaf = None;
        let mut clipboard_set = None;
        let mut paste_to = None;
        let mut open_url = None;
        let mut context_commands: Vec<Command> = Vec::new();
        let context_profiles: Vec<String> = self
            .config
            .root
            .profiles
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let mut active_rect = content;

        for (leaf, rect) in leaves {
            let PaneNode::Leaf { session } = self.workspace.nodes[leaf] else {
                continue;
            };
            let focused = leaf == active_leaf;
            if focused {
                active_rect = rect;
            }
            let Some(entry) = self.sessions.get_mut(&session) else {
                continue;
            };
            let out = term_view::show(
                ui,
                rect,
                &entry.session,
                &mut entry.view,
                &mut self.painter,
                &metrics,
                &theme,
                padding,
                focused,
                &behavior,
            );

            if out.response.is_pointer_button_down_on() {
                if !focused {
                    focus_leaf = Some(leaf);
                }
                // Clicking a terminal reclaims the keyboard from any egui
                // widget (e.g. a chrome button that grabbed focus).
                ui.ctx().memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
            }

            // Diagnostics for HYPER_DEBUG_INPUT.
            if std::env::var_os("HYPER_DEBUG_INPUT").is_some()
                && out.response.secondary_clicked()
            {
                eprintln!(
                    "[input] pane secondary_clicked quick_edit={} mouse_mode={}",
                    behavior.quick_edit,
                    out.mode
                        .intersects(alacritty_terminal::term::TermMode::MOUSE_MODE)
                );
            }

            // Right-click context menu (unless quickEdit gave the right
            // button a job, or the app in the terminal owns the mouse —
            // shift+right-click bypasses mouse reporting, like selection
            // does). Keep attaching while the popup is open so it survives
            // the shift key being released before an item is clicked.
            let app_owns_mouse = out
                .mode
                .intersects(alacritty_terminal::term::TermMode::MOUSE_MODE)
                && !ui.input(|i| i.modifiers.shift);
            if !behavior.quick_edit
                && (!app_owns_mouse || egui::Popup::is_any_open(ui.ctx()))
            {
                out.response.context_menu(|ui| {
                    ui.set_min_width(180.0);
                    let mut item = |ui: &mut egui::Ui, label: &str, command: Command| {
                        if ui.button(label).clicked() {
                            context_commands.push(command);
                            ui.close();
                        }
                    };
                    item(ui, "Copy", Command::Copy);
                    item(ui, "Paste", Command::Paste);
                    item(ui, "Select All", Command::SelectAll);
                    ui.separator();
                    item(ui, "New Tab", Command::NewTab);
                    item(ui, "Split Right", Command::SplitRight);
                    item(ui, "Split Down", Command::SplitDown);
                    ui.separator();
                    // Per-profile submenus (Electron shell-menu parity).
                    for name in &context_profiles {
                        ui.menu_button(name, |ui| {
                            ui.set_min_width(140.0);
                            item(ui, "New Tab", Command::NewTabProfile(name.clone()));
                            item(
                                ui,
                                "Split Right",
                                Command::SplitRightProfile(name.clone()),
                            );
                            item(ui, "Split Down", Command::SplitDownProfile(name.clone()));
                        });
                    }
                    ui.separator();
                    item(ui, "Clear Buffer", Command::ClearBuffer);
                    item(ui, "Close Pane", Command::ClosePane);
                });
            }
            if let Some(copied) = out.copied {
                clipboard_set = Some(copied);
            }
            if out.wants_paste {
                paste_to = Some((session, out.mode));
            }
            if let Some(url) = out.open_url {
                open_url = Some(url);
            }

            // Dim border around unfocused panes; bell flash.
            if multiple_panes && !focused {
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    Color32::from_black_alpha(64),
                );
            }
            if let Some(until) = entry.bell_until {
                if Instant::now() < until {
                    ui.painter().rect_stroke(
                        rect.shrink(1.0),
                        0.0,
                        Stroke::new(2.0, theme.foreground),
                        egui::StrokeKind::Inside,
                    );
                    ui.ctx().request_repaint();
                } else {
                    entry.bell_until = None;
                }
            }
        }

        if let Some(leaf) = focus_leaf {
            if let Some(tab_idx) = self.workspace.tab_of(leaf) {
                self.workspace.tabs[tab_idx].active_leaf = leaf;
            }
        }
        self.pending_commands.extend(context_commands);
        if let Some(text) = clipboard_set {
            self.set_clipboard(text);
        }
        if let Some((session, mode)) = paste_to {
            let text = self.clipboard_text();
            if !text.is_empty() {
                if let Some(entry) = self.sessions.get_mut(&session) {
                    entry.session.write(input::encode_paste(&text, mode));
                }
            }
        }
        if let Some(url) = open_url {
            if crate::links::is_safe_url(&url) {
                let _ = open::that(url);
            } else {
                log::warn!("blocked non-allowlisted url: {url}");
            }
        }

        // Search overlay over the active pane.
        if self.search.open {
            if let Some(entry) = self
                .workspace
                .active_session()
                .and_then(|id| self.sessions.get(&id))
            {
                self.search.show(ui, active_rect, &entry.session, &theme);
            }
        }

        // Command palette overlay over the active pane.
        if self.palette.open {
            let profile_names: Vec<String> = self
                .config
                .root
                .profiles
                .iter()
                .map(|p| p.name.clone())
                .collect();
            if let Some(cmd) = self.palette.show(ui, active_rect, &profile_names, &theme) {
                self.run_palette_command(cmd, ui.ctx());
            }
        }
    }

    fn on_exit(&mut self) {
        // Persist session state (same file format as the Electron build) and
        // window geometry.
        //
        // The capture is gated on `restoreSession` exactly as the Electron
        // build's `before-quit` was: with the option off nothing ever reads
        // this file, and writing it anyway leaves a list of every profile and
        // working directory on disk for a feature the user declined.
        if self.config.root.restore_session.unwrap_or(false) {
            let mut state = crate::session_restore::StoredSessionState::default();
            let mut window = crate::session_restore::StoredWindow { tabs: Vec::new() };
            for tab in &self.workspace.tabs {
                for leaf in self.workspace.leaves_of(tab.root) {
                    let PaneNode::Leaf { session } = self.workspace.nodes[leaf] else {
                        continue;
                    };
                    if let Some(entry) = self.sessions.get(&session) {
                        window.tabs.push(crate::session_restore::StoredTab {
                            profile: entry.session.profile.clone(),
                            cwd: entry
                                .session
                                .cwd()
                                .map(|p| p.to_string_lossy().into_owned()),
                        });
                    }
                }
            }
            if !window.tabs.is_empty() {
                state.windows.push(window);
            }
            crate::session_restore::write_session_state(&state);
        }

        if let Some(geometry) = &self.last_geometry {
            crate::session_restore::write_window_geometry(geometry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paths handed over by `open`/the CLI are typed at a live shell prompt,
    /// so a filename carrying shell syntax must arrive inert.
    #[test]
    fn opened_paths_are_escaped_before_being_typed() {
        // The motivating case: a file whose name is a second command.
        assert_eq!(
            escape_shell_cmd("/tmp/report;rm -rf ~.txt"),
            "/tmp/report\\;rm\\ -rf\\ \\~.txt"
        );
        // Spaces and command substitution.
        assert_eq!(
            escape_shell_cmd("/My Files/$(id).txt"),
            "/My\\ Files/\\$\\(id\\).txt"
        );
        assert_eq!(escape_shell_cmd("/tmp/`whoami`"), "/tmp/\\`whoami\\`");
        // An ordinary path is left exactly as it was.
        assert_eq!(escape_shell_cmd("/Users/me/code"), "/Users/me/code");
    }

    /// Drive the real app headlessly: a right-click over the pane must open
    /// the context menu.
    #[test]
    fn right_click_over_pane_opens_context_menu() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(800.0, 600.0))
            .build_eframe(|cc| {
                let mut app = HyperApp::new(cc, Vec::new(), None, Default::default());
                // muda menus require the process main thread; skip in tests.
                app.menu_generation = Some(app.config.generation);
                app
            });

        // Let the first tab spawn and the layout settle.
        for _ in 0..5 {
            harness.step();
        }

        let pos = egui::Pos2::new(400.0, 300.0);
        harness.event(egui::Event::PointerMoved(pos));
        harness.step();
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: Default::default(),
        });
        harness.step();
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers: Default::default(),
        });
        harness.step();
        harness.step();

        assert!(
            egui::Popup::is_any_open(&harness.ctx),
            "right-click over the pane should open the context menu"
        );
    }
}

#[cfg(test)]
mod shift_enter_tests {
    use super::*;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line, Point};
    use alacritty_terminal::term::TermMode;
    use alacritty_terminal::vte::ansi::Processor;

    fn grid_text(term: &alacritty_terminal::Term<hyper_term::event::EventProxy>) -> String {
        let grid = term.grid();
        let mut s = String::new();
        for line in 0..grid.screen_lines() {
            for col in 0..grid.columns() {
                s.push(grid[Point::new(Line(line as i32), Column(col))].c);
            }
            s.push('\n');
        }
        s
    }

    /// End-to-end: after an app pushes the kitty keyboard protocol (as
    /// Claude Code does with CSI > 1 u), shift+enter must reach the PTY as
    /// CSI 13;2u. `cat` keeps the tty in canonical mode so the bytes are
    /// echoed back visibly into the grid.
    #[test]
    fn shift_enter_reaches_pty_as_kitty_csi_u() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(800.0, 600.0))
            .build_eframe(|cc| {
                let mut app = HyperApp::new(cc, Vec::new(), None, Default::default());
                app.menu_generation = Some(app.config.generation);
                app
            });
        for _ in 0..5 {
            harness.step();
        }

        let session_id = harness
            .state()
            .workspace
            .active_session()
            .expect("a session should exist");
        let term = harness.state().sessions[&session_id].session.term.clone();

        // Run `cat` so unknown escapes are echoed by the tty instead of
        // being eaten by zle.
        harness.state().sessions[&session_id]
            .session
            .write(b"cat\r".to_vec());
        std::thread::sleep(std::time::Duration::from_millis(800));

        // What Claude Code sends at startup.
        let mut parser: Processor = Processor::new();
        parser.advance(&mut *term.lock(), b"\x1b[>1u");
        assert!(
            term.lock().mode().contains(TermMode::DISAMBIGUATE_ESC_CODES),
            "kitty push must set the mode flag"
        );

        harness.event(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                shift: true,
                ..Default::default()
            },
        });
        harness.step();

        let mut content = String::new();
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            content = grid_text(&term.lock());
            if content.contains("13;2u") {
                break;
            }
            harness.step();
        }
        assert!(
            content.contains("13;2u"),
            "shift+enter did not reach the pty as CSI 13;2u; grid:\n{content}"
        );
    }

    /// End-to-end: AltGr+Space types a non-breaking space on many European
    /// layouts. Windows and X11 report AltGr as ctrl+alt, so egui-winit drops
    /// the text and `MonitorProbe` recovers it from winit. It must reach the
    /// PTY — and the same press must not *also* encode as ctrl+alt+Space,
    /// which under the kitty protocol is a CSI u report.
    #[test]
    fn altgr_text_reaches_the_pty_without_a_stray_escape() {
        let (altgr_tx, altgr_rx) = crossbeam_channel::unbounded();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(800.0, 600.0))
            .build_eframe(move |cc| {
                let mut app = HyperApp::new(
                    cc,
                    Vec::new(),
                    None,
                    crate::PlatformTaps {
                        monitors: Default::default(),
                        altgr_rx,
                    },
                );
                app.menu_generation = Some(app.config.generation);
                app
            });
        for _ in 0..5 {
            harness.step();
        }

        let session_id = harness
            .state()
            .workspace
            .active_session()
            .expect("a session should exist");
        let term = harness.state().sessions[&session_id].session.term.clone();

        // `cat` echoes what it receives, so an ESC would show up as `^[`.
        harness.state().sessions[&session_id]
            .session
            .write(b"cat\r".to_vec());
        std::thread::sleep(std::time::Duration::from_millis(800));

        // Under the kitty protocol every ctrl-modified key is encodable, so
        // this is where an unguarded ctrl+alt would encode `[` a second time.
        let mut parser: Processor = Processor::new();
        parser.advance(&mut *term.lock(), b"\x1b[>1u");
        assert!(term.lock().mode().contains(TermMode::DISAMBIGUATE_ESC_CODES));

        // One AltGr+Space press: the character winit composed, plus the key
        // event egui delivers for it.
        altgr_tx.send("\u{a0}".into()).unwrap();
        harness.event(egui::Event::Key {
            key: egui::Key::Space,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: true,
                alt: true,
                ..Default::default()
            },
        });
        harness.step();

        let mut content = String::new();
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            content = grid_text(&term.lock());
            if content.contains('\u{a0}') {
                break;
            }
            harness.step();
        }
        assert!(
            content.contains('\u{a0}'),
            "AltGr text never reached the pty; grid:\n{content}"
        );
        // A CSI u report for the same press echoes as `^[[32;7u`.
        assert!(
            !content.contains("^["),
            "AltGr+Space sent an escape sequence as well as the character; grid:\n{}",
            content.escape_debug()
        );
    }
}

/// Repro for the report: open an ssh split and, while ssh sits at its
/// password prompt, close the pane — the whole app froze.
///
/// `#[ignore]` because it rewrites the process-global XDG_CONFIG_HOME and
/// PATH, which races the other harness tests. Run it in its own process:
/// `cargo test -p hyper-app ssh_pane_close -- --ignored`.
/// Set HYPER_TEST_SSH_PORT to a local throwaway sshd's port to drive the
/// real `/usr/bin/ssh` instead of the bundled fake.
#[cfg(test)]
mod ssh_pane_close_tests {
    use super::*;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line, Point};

    fn grid_text(term: &alacritty_terminal::Term<hyper_term::event::EventProxy>) -> String {
        let grid = term.grid();
        let mut s = String::new();
        for line in 0..grid.screen_lines() {
            for col in 0..grid.columns() {
                s.push(grid[Point::new(Line(line as i32), Column(col))].c);
            }
            s.push('\n');
        }
        s
    }

    #[test]
    #[ignore = "mutates process-global env; run alone: cargo test ssh_pane_close -- --ignored"]
    fn closing_an_ssh_pane_stuck_at_a_password_prompt_does_not_freeze() {
        use std::os::unix::fs::PermissionsExt;

        // Scratch config with an ssh profile carrying a stored password, and
        // a fake `ssh` on PATH that reaches a password prompt and blocks on
        // the tty with echo off, like the real client.
        let dir = tempfile::tempdir().unwrap();
        let cfg_dir = dir.path().join("config/hyper-revamp");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        // HYPER_TEST_SSH_PORT set ⇒ drive the REAL /usr/bin/ssh against a
        // local throwaway sshd; the empty stored password means no askpass,
        // so ssh prompts inline on the tty exactly like the report.
        let real_port: Option<String> = std::env::var("HYPER_TEST_SSH_PORT").ok();
        let port_field = real_port
            .as_deref()
            .map(|p| format!("\"port\": {p},"))
            .unwrap_or_default();
        let host = if real_port.is_some() { "127.0.0.1" } else { "testhost" };
        std::fs::write(
            cfg_dir.join("hyper-revamp.json"),
            format!(
                r#"{{
              "config": {{
                "profiles": [
                  {{"name": "default", "config": {{}}}},
                  {{"name": "ssh", "config": {{}}, "type": "ssh",
                   "ssh": {{"authType": "password", "host": "{host}", {port_field}
                           "extraArgs": ["-o", "UserKnownHostsFile=/dev/null"],
                           "user": "kate", "password": ""}}}}
                ]
              }}
            }}"#
            ),
        )
        .unwrap();
        if real_port.is_none() {
            let bin = dir.path().join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let fake_ssh = bin.join("ssh");
            // `trap '' HUP` mirrors real ssh, which survives SIGHUP while
            // blocked at its password prompt (verified against OpenSSH on
            // macOS): the pane's child never dies, alacritty's `Pty::drop`
            // blocks forever in `child.wait()`, and any shutdown path that
            // joins the IO thread would hang the UI thread here.
            std::fs::write(
                &fake_ssh,
                "#!/bin/sh\ntrap '' HUP\nstty -echo 2>/dev/null\nprintf \"kate@testhost's password: \"\nread -r _line\n",
            )
            .unwrap();
            std::fs::set_permissions(&fake_ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::env::set_var(
                "PATH",
                format!(
                    "{}:{}",
                    bin.display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            );
        }
        std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config"));

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(800.0, 600.0))
            .build_eframe(|cc| {
                let mut app = HyperApp::new(cc, Vec::new(), None, Default::default());
                // muda menus require the process main thread; skip in tests.
                app.menu_generation = Some(app.config.generation);
                app
            });
        for _ in 0..5 {
            harness.step();
        }

        // Open the ssh split and wait for the password prompt.
        harness
            .state_mut()
            .pending_commands
            .push(Command::SplitRightProfile("ssh".into()));
        harness.step();
        let ssh_id = harness
            .state()
            .workspace
            .active_session()
            .expect("the split should have spawned a session");
        let term = harness.state().sessions[&ssh_id].session.term.clone();
        let mut content = String::new();
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            harness.step();
            content = grid_text(&term.lock());
            if content.contains("password") {
                break;
            }
        }
        assert!(
            content.contains("password"),
            "fake ssh never reached its prompt; grid:\n{content}"
        );

        // Close the pane while ssh is blocked at the prompt. A regression
        // here hangs the step and the test times out.
        harness.state_mut().pending_commands.push(Command::ClosePane);
        harness.step();
        assert!(
            !harness.state().sessions.contains_key(&ssh_id),
            "the ssh session should be gone"
        );
        for _ in 0..10 {
            harness.step();
        }
    }
}

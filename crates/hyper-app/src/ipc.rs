//! Single-instance handoff over a unix socket at `<cfgDir>/hyper-revamp.sock`.
//!
//! A second `hyper-revamp [paths…]` invocation connects and hands its paths
//! to the running instance (which opens a tab per path), then exits.

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct IpcMessage {
    /// Directories to open as new tabs ([] ⇒ just focus the window).
    pub paths: Vec<String>,
}

/// Try handing `paths` to a running instance. Returns true on success
/// (the caller should exit).
#[cfg(unix)]
pub fn try_handoff(paths: &[String]) -> bool {
    handoff_to(&hyper_config::paths::socket_path(), paths)
}

#[cfg(unix)]
fn handoff_to(socket: &std::path::Path, paths: &[String]) -> bool {
    use std::io::Write;
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return false;
    };
    let msg = IpcMessage {
        paths: paths.to_vec(),
    };
    let Ok(json) = serde_json::to_string(&msg) else {
        return false;
    };
    stream.write_all(json.as_bytes()).is_ok() && stream.write_all(b"\n").is_ok()
}

#[cfg(not(unix))]
pub fn try_handoff(_paths: &[String]) -> bool {
    false
}

/// Bind the instance socket and forward incoming messages on a channel.
/// The returned guard removes the socket file on drop.
pub struct IpcServer {
    pub rx: Receiver<IpcMessage>,
    #[cfg(unix)]
    socket: std::path::PathBuf,
    /// `(dev, ino)` of the socket this instance bound, so that on exit it can
    /// tell its own socket from one another instance has since put in its
    /// place.
    #[cfg(unix)]
    socket_id: Option<(u64, u64)>,
}

/// Outcome of claiming the instance socket.
pub enum Serve {
    /// This process owns the socket.
    Server(IpcServer),
    /// Another instance owns it — it should be handed the paths instead.
    AlreadyRunning,
    /// No IPC on this platform, or the socket could not be bound. The app
    /// still runs, just without single-instance handoff.
    Unavailable,
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            // Only unlink the socket if it is still the one we bound. Removing
            // the path unconditionally deletes whatever is there now — and if
            // another instance has taken over in the meantime, that cuts off a
            // live listener rather than cleaning up after this one.
            use std::os::unix::fs::MetadataExt;
            let still_ours = std::fs::metadata(&self.socket)
                .map(|meta| Some((meta.dev(), meta.ino())) == self.socket_id)
                .unwrap_or(false);
            if still_ours {
                let _ = std::fs::remove_file(&self.socket);
            }
        }
    }
}

/// Bind `socket`, taking over only a socket no one is listening on.
///
/// The obvious `if socket.exists() { remove_file(socket) }` is a race: the
/// earlier `try_handoff` connect having failed does not mean the path is stale
/// *now*, and two instances starting together would each delete the other's
/// live socket and both keep running. Binding first and only clearing the path
/// once a connect proves nobody is home closes that window.
#[cfg(unix)]
fn bind_socket(socket: &std::path::Path) -> Result<UnixListener, Serve> {
    use std::io::ErrorKind;

    let err = match UnixListener::bind(socket) {
        Ok(listener) => return Ok(listener),
        Err(err) if err.kind() == ErrorKind::AddrInUse => err,
        Err(err) => {
            log::warn!("couldn't bind instance socket: {err}");
            return Err(Serve::Unavailable);
        }
    };

    // The path is taken. If someone answers, they are a live instance and we
    // lost the race between handing off and binding.
    if UnixStream::connect(socket).is_ok() {
        return Err(Serve::AlreadyRunning);
    }

    // Nobody answered: a dead instance left the file behind.
    log::info!("removing stale instance socket at {}", socket.display());
    if let Err(err) = std::fs::remove_file(socket) {
        log::warn!("couldn't remove stale instance socket: {err}");
        return Err(Serve::Unavailable);
    }
    UnixListener::bind(socket).map_err(|retry_err| {
        log::warn!("couldn't bind instance socket after clearing a stale one: {retry_err} (first: {err})");
        Serve::Unavailable
    })
}

#[cfg(unix)]
pub fn serve() -> Serve {
    use std::io::BufRead;
    use std::os::unix::fs::MetadataExt;

    let socket = hyper_config::paths::socket_path();
    if let Some(dir) = socket.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let listener = match bind_socket(&socket) {
        Ok(listener) => listener,
        Err(outcome) => return outcome,
    };
    let socket_id = std::fs::metadata(&socket)
        .map(|meta| (meta.dev(), meta.ino()))
        .ok();

    let (tx, rx): (Sender<IpcMessage>, Receiver<IpcMessage>) = crossbeam_channel::unbounded();
    if std::thread::Builder::new()
        .name("ipc-server".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let reader = std::io::BufReader::new(stream);
                for line in reader.lines().map_while(Result::ok) {
                    if let Ok(msg) = serde_json::from_str::<IpcMessage>(&line) {
                        let _ = tx.send(msg);
                    }
                }
            }
        })
        .is_err()
    {
        return Serve::Unavailable;
    }

    Serve::Server(IpcServer {
        rx,
        socket,
        socket_id,
    })
}

#[cfg(not(unix))]
pub fn serve() -> Serve {
    Serve::Unavailable
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A socket left behind by a crashed instance is taken over; one a live
    /// instance is listening on is not.
    #[test]
    fn stale_sockets_are_reclaimed_but_live_ones_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("hyper-revamp.sock");

        // Fresh path: binds.
        let listener = bind_socket(&socket).ok().expect("fresh bind");

        // Held by a live listener: reported as already running, and — the
        // point of the test — the live socket survives.
        assert!(matches!(
            bind_socket(&socket),
            Err(Serve::AlreadyRunning)
        ));
        assert!(socket.exists());
        assert!(handoff_to(&socket, &[]), "the first listener still answers");

        // Simulate a crash: the listener goes away without unlinking. Dropping
        // a UnixListener does not remove the file, which is exactly the state
        // a killed process leaves behind.
        drop(listener);
        assert!(socket.exists());
        bind_socket(&socket).ok().expect("stale socket is reclaimed");
    }

    /// An instance that has been superseded must not unlink the socket its
    /// successor is now listening on.
    #[test]
    fn dropping_a_superseded_server_leaves_the_new_socket_alone() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("hyper-revamp.sock");

        let (_tx, rx) = crossbeam_channel::unbounded();
        let old = IpcServer {
            rx,
            socket: socket.clone(),
            // Bound a socket that has since been replaced.
            socket_id: Some((0, 0)),
        };

        // A successor binds the same path.
        let _successor = UnixListener::bind(&socket).unwrap();
        drop(old);

        assert!(
            socket.exists(),
            "the superseded instance unlinked its successor's socket"
        );
    }
}

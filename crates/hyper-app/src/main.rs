#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod fonts;
mod grid_paint;
mod input;
mod ipc;
mod links;
mod macos;
mod menu;
mod mouse;
mod palette;
mod search_ui;
mod session_restore;
mod settings;
mod term_view;
mod theme;
mod workspace;

use clap::Parser;
use std::sync::{Arc, OnceLock};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::window::WindowId;

/// The display list, which winit only exposes once the event loop is running.
pub type Monitors = Arc<OnceLock<Vec<session_restore::MonitorBounds>>>;

/// State the app can only get from the winit layer, handed over at startup.
pub struct PlatformTaps {
    pub monitors: Monitors,
    /// AltGr characters recovered from winit — see [`MonitorProbe::window_event`].
    pub altgr_rx: crossbeam_channel::Receiver<String>,
}

impl Default for PlatformTaps {
    /// For tests: a receiver whose sender is already gone, so it simply never
    /// yields anything.
    fn default() -> Self {
        let (_tx, altgr_rx) = crossbeam_channel::unbounded();
        PlatformTaps {
            monitors: Monitors::default(),
            altgr_rx,
        }
    }
}

/// Wraps eframe's winit application purely to capture that display list on
/// `resumed`, before the window is shown.
///
/// The saved window position can name a monitor that has since been unplugged,
/// which puts the window somewhere the user can neither see nor drag back.
/// Electron checked this up front with `windowUtils.positionIsValid`; winit has
/// no monitor list until the loop is active, so `HyperApp` makes the same check
/// against these bounds on its first frame instead.
struct MonitorProbe<'a> {
    inner: eframe::EframeWinitApplication<'a>,
    monitors: Monitors,
    altgr_tx: crossbeam_channel::Sender<String>,
    /// Live modifier state, needed to spot AltGr (see `window_event`).
    modifiers: winit::keyboard::ModifiersState,
}

impl ApplicationHandler<eframe::UserEvent> for MonitorProbe<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let bounds = event_loop
            .available_monitors()
            .map(|monitor| {
                // winit reports physical pixels; the saved position is in the
                // logical points egui reports and `with_position` consumes.
                let scale = monitor.scale_factor() as f32;
                let position = monitor.position();
                let size = monitor.size();
                [
                    position.x as f32 / scale,
                    position.y as f32 / scale,
                    size.width as f32 / scale,
                    size.height as f32 / scale,
                ]
            })
            .collect();
        let _ = self.monitors.set(bounds);
        self.inner.resumed(event_loop);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.inner.new_events(event_loop, cause);
    }
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: eframe::UserEvent) {
        self.inner.user_event(event_loop, event);
    }
    /// Recover AltGr characters on the way past.
    ///
    /// Windows and X11 report AltGr as ctrl+alt, and egui-winit drops
    /// `Event::Text` whenever ctrl is held (to swallow the stray text that
    /// comes with Cmd+C, Ctrl+W and friends). On a German or Spanish layout
    /// that silently eats `@`, `\`, `€`, `[`, `]`, `{` and `}` — and egui has
    /// no key to offer in their place either, since those characters aren't
    /// `egui::Key` variants. This is the last point where the composed
    /// character still exists, so it is picked up here and handed to the app.
    ///
    /// Only a layout that actually produces a printable character for the
    /// chord sends anything: on a US layout ctrl+alt+E composes nothing, so
    /// `text` is empty and a real ctrl+alt binding still reaches the keymap.
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match &event {
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput {
                event: key_event,
                is_synthetic: false,
                ..
            } if key_event.state.is_pressed()
                && self.modifiers.control_key()
                && self.modifiers.alt_key() =>
            {
                if let Some(text) = &key_event.text {
                    if !text.is_empty() && !text.chars().any(char::is_control) {
                        let _ = self.altgr_tx.send(text.to_string());
                    }
                }
            }
            _ => {}
        }
        self.inner.window_event(event_loop, id, event);
    }
    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        self.inner.device_event(event_loop, id, event);
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.about_to_wait(event_loop);
    }
    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.suspended(event_loop);
    }
    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.exiting(event_loop);
    }
    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.memory_warning(event_loop);
    }
}

/// A fast, native terminal — drop-in replacement for Hyper Revamp.
#[derive(Parser)]
#[command(name = "hyper-revamp", version, about)]
struct Cli {
    /// Directories to open as tabs.
    paths: Vec<String>,
}

fn main() -> eframe::Result {
    // ssh SSH_ASKPASS helper mode: print the handed-over password and exit.
    if std::env::var("HYPER_ASKPASS_FILE").is_ok() && hyper_term::ssh::run_askpass_helper() {
        return Ok(());
    }

    env_logger::init();
    let cli = Cli::parse();

    // Canonicalize path args relative to the invoking shell's cwd. Files are
    // kept alongside directories — the app opens each per `openFile`'s rules —
    // and a path that doesn't exist is an error, exactly as `cli/index.ts` had
    // it, rather than a silent no-op that opens an empty window.
    let mut paths = Vec::with_capacity(cli.paths.len());
    for arg in &cli.paths {
        match std::fs::canonicalize(arg) {
            Ok(path) => paths.push(path.to_string_lossy().into_owned()),
            Err(err) => {
                eprintln!("Error! Directory or file does not exist: {arg} ({err})");
                std::process::exit(1);
            }
        }
    }

    // Single instance: hand paths to a running app and exit.
    if ipc::try_handoff(&paths) {
        return Ok(());
    }
    let ipc_server = match ipc::serve() {
        ipc::Serve::Server(server) => Some(server),
        // Another instance bound the socket between the handoff attempt above
        // and this bind. Hand off to it rather than opening a second window.
        ipc::Serve::AlreadyRunning => {
            ipc::try_handoff(&paths);
            return Ok(());
        }
        ipc::Serve::Unavailable => None,
    };

    let geometry = session_restore::read_window_geometry();
    // Without an explicit icon eframe swaps the Dock icon to its own default
    // logo at runtime, overriding the bundle's .icns.
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))
        .expect("bundled icon is a valid png");
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Hyper Revamp")
        .with_icon(icon)
        .with_position(geometry.window_position)
        .with_inner_size([
            geometry.window_size[0].max(370.0),
            geometry.window_size[1].max(190.0),
        ])
        .with_min_inner_size([370.0, 190.0]);
    if cfg!(target_os = "macos") {
        // Compact look: no titlebar, traffic lights float over the tab bar
        // (which reserves an inset for them and acts as the drag handle).
        viewport = viewport
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_fullsize_content_view(true);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let event_loop = winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event()
        .build()
        .map_err(eframe::Error::from)?;
    let monitors: Monitors = Arc::default();
    let (altgr_tx, altgr_rx) = crossbeam_channel::unbounded();
    let taps = PlatformTaps {
        monitors: monitors.clone(),
        altgr_rx,
    };
    let inner = eframe::create_native(
        "hyper-revamp",
        options,
        Box::new(move |cc| Ok(Box::new(app::HyperApp::new(cc, paths, ipc_server, taps)))),
        &event_loop,
    );
    let mut winit_app = MonitorProbe {
        inner,
        monitors,
        altgr_tx,
        modifiers: Default::default(),
    };
    event_loop.run_app(&mut winit_app).map_err(Into::into)
}

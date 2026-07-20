//! Terminal engine: PTY sessions driven by alacritty_terminal's event loop.
//!
//! This crate has no GUI dependency; the app layer supplies a [`event::Waker`]
//! that schedules repaints and drains the session event channel.

pub mod cwd;
pub mod env;
pub mod event;
pub mod session;
pub mod ssh;

pub use alacritty_terminal;
pub use event::{EventProxy, SessionEvent, SessionId, Waker};
pub use session::{Session, SharedTerm, SpawnOptions};

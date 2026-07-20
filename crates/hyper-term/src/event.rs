use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener};

/// Unique identifier for a terminal session, stable for its lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u64);

impl SessionId {
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        SessionId(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

/// An alacritty terminal event tagged with the session it came from.
pub struct SessionEvent {
    pub id: SessionId,
    pub event: Event,
}

/// Schedules a repaint on the UI thread. Implemented app-side with
/// `egui::Context::request_repaint_after`; a no-op impl works for tests.
pub trait Waker: Send + Sync {
    fn wake(&self);
}

pub struct NoopWaker;
impl Waker for NoopWaker {
    fn wake(&self) {}
}

/// Fans terminal events out of the PTY event loop to the app.
#[derive(Clone)]
pub struct EventProxy {
    id: SessionId,
    tx: crossbeam_channel::Sender<SessionEvent>,
    waker: Arc<dyn Waker>,
}

impl EventProxy {
    pub fn new(
        id: SessionId,
        tx: crossbeam_channel::Sender<SessionEvent>,
        waker: Arc<dyn Waker>,
    ) -> Self {
        Self { id, tx, waker }
    }
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        let _ = self.tx.send(SessionEvent { id: self.id, event });
        self.waker.wake();
    }
}

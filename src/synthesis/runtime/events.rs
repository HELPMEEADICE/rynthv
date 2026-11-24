use super::worker::Event;
use crate::synthesis::notify::Notify;
use std::sync::mpsc::{SendError, Sender};

pub(super) struct Events {
    sender: Sender<Event>,
    repaint: Notify,
}

impl Events {
    pub fn new(sender: Sender<Event>, repaint: Notify) -> Self {
        Self { sender, repaint }
    }

    pub fn send(&self, event: Event) -> Result<(), SendError<Event>> {
        self.sender.send(event)?;
        self.repaint.wake();
        Ok(())
    }
}

/// Kept outside `run` so its event sender is already dropped before the final
/// wakeup. This also exposes an unexpected worker exit without periodic polls.
pub(super) struct WakeOnExit(pub Notify);

impl Drop for WakeOnExit {
    fn drop(&mut self) {
        self.0.wake();
    }
}

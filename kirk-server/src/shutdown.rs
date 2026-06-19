//! Broadcast-based shutdown plumbing.

use std::time::Duration;
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct ShutdownHandle {
    tx: broadcast::Sender<()>,
}

impl Default for ShutdownHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl ShutdownHandle {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(16);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.tx.subscribe()
    }

    pub fn fire(&self) {
        let _ = self.tx.send(());
    }
}

pub const DRAIN_DEADLINE: Duration = Duration::from_secs(10);

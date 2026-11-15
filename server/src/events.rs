use std::sync::Arc;

use chrono::{DateTime, Local};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::at::LogDir;
use crate::status::{ConnectionState, Status};
use crate::store::Sms;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Status {
        status: Arc<Status>,
    },
    Connection {
        connection: ConnectionState,
    },
    Sms {
        sms: Sms,
    },
    /// Messages were read from the modem; their storage state may have changed.
    SmsSync {
        new: usize,
    },
    AtLog {
        dir: LogDir,
        line: String,
        ts: DateTime<Local>,
    },
}

#[derive(Clone)]
pub struct EventBus(broadcast::Sender<Event>);

impl EventBus {
    pub fn new() -> Self {
        Self(broadcast::channel(256).0)
    }

    pub fn send(&self, ev: Event) {
        let _ = self.0.send(ev);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.0.subscribe()
    }
}

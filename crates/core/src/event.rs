//! In-process event bus.
//!
//! Events are broadcast to live subscribers and retained in a bounded history
//! so late subscribers (UI, agents) can catch up on recent activity.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

/// A single application event.
#[derive(Debug, Clone, Serialize)]
pub struct AppEvent {
    pub kind: String,
    pub payload: serde_json::Value,
    pub ts: DateTime<Utc>,
}

/// Maximum number of events buffered for broadcast subscribers.
const CHANNEL_CAPACITY: usize = 1024;

/// Maximum number of events retained in history.
const HISTORY_CAPACITY: usize = 256;

/// A cheaply-cloneable handle to the shared event bus.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Arc<AppEvent>>,
    history: Arc<RwLock<VecDeque<Arc<AppEvent>>>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            tx,
            history: Arc::new(RwLock::new(VecDeque::new())),
        }
    }

    /// Publish an event to all subscribers and append it to history.
    pub fn publish(&self, kind: impl Into<String>, payload: serde_json::Value) {
        let event = Arc::new(AppEvent {
            kind: kind.into(),
            payload,
            ts: Utc::now(),
        });

        {
            let mut history = self.history.write().unwrap_or_else(|e| e.into_inner());
            history.push_back(event.clone());
            while history.len() > HISTORY_CAPACITY {
                history.pop_front();
            }
        }

        let _ = self.tx.send(event);
    }

    /// Subscribe to live events. Events published before subscribing are
    /// only available through [`EventBus::recent`].
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<AppEvent>> {
        self.tx.subscribe()
    }

    /// Snapshot of recent history, oldest first.
    pub fn recent(&self) -> Vec<Arc<AppEvent>> {
        let history = self.history.read().unwrap_or_else(|e| e.into_inner());
        history.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast::error::RecvError;

    #[test]
    fn subscriber_receives_published_events() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        bus.publish("transfer.added", serde_json::json!({"id": "1"}));
        bus.publish(
            "transfer.progress",
            serde_json::json!({"id": "1", "pct": 50}),
        );

        let first = rx.blocking_recv().unwrap();
        assert_eq!(first.kind, "transfer.added");
        assert_eq!(first.payload["id"], "1");

        let second = rx.blocking_recv().unwrap();
        assert_eq!(second.kind, "transfer.progress");
        assert_eq!(second.payload["pct"], 50);
    }

    #[test]
    fn recent_returns_published_events_oldest_first() {
        let bus = EventBus::new();
        bus.publish("a", serde_json::json!(1));
        bus.publish("b", serde_json::json!(2));

        let recent = bus.recent();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].kind, "a");
        assert_eq!(recent[1].kind, "b");
    }

    #[test]
    fn history_stays_bounded() {
        let bus = EventBus::new();
        for i in 0..(HISTORY_CAPACITY + 100) {
            bus.publish(format!("evt-{i}"), serde_json::json!(i));
        }

        let recent = bus.recent();
        assert_eq!(recent.len(), HISTORY_CAPACITY);
        assert_eq!(recent[0].kind, format!("evt-{}", 100));
        assert_eq!(
            recent[HISTORY_CAPACITY - 1].kind,
            format!("evt-{}", HISTORY_CAPACITY + 99)
        );

        // A subscriber attached after the burst: publishing more events than
        // the fixed channel ring-buffer capacity makes the receiver lag (it
        // skipped the earliest messages) instead of blocking forever.
        let mut rx = bus.subscribe();
        for i in 0..(CHANNEL_CAPACITY + 100) {
            bus.publish(format!("evt2-{i}"), serde_json::json!(i));
        }
        assert_eq!(rx.blocking_recv().unwrap_err(), RecvError::Lagged(100));
    }
}

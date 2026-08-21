//! Server-Sent Events (SSE) fan-out from the [`EventBus`].
//!
//! Subscribers receive a replay of recent history followed by live events.
//! High-frequency progress events are throttled per event kind so the UI and
//! agents are not flooded.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agpeer_core::event::{AppEvent, EventBus};
use axum::response::sse::{Event, KeepAlive};
use axum::response::Sse;
use futures::Stream;
use tokio::sync::broadcast;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

/// Minimum interval between two progress events of the same kind.
const PROGRESS_THROTTLE: Duration = Duration::from_millis(1000);

/// Build the underlying event stream (history replay followed by live events
/// with per-kind throttling for `*.progress` kinds). Split out so tests can
/// iterate the raw stream without the `Sse` response wrapper.
fn event_stream(bus: EventBus) -> ReceiverStream<Arc<AppEvent>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Arc<AppEvent>>(512);

    // Replay recent history first.
    let history = bus.recent();
    let history_tx = tx.clone();
    tokio::spawn(async move {
        for ev in history {
            if history_tx.send(ev).await.is_err() {
                break;
            }
        }
    });

    // Live events with per-kind throttling for progress events.
    tokio::spawn(async move {
        let mut sub = bus.subscribe();
        let mut last_emit: HashMap<String, Instant> = HashMap::new();
        loop {
            match sub.recv().await {
                Ok(ev) => {
                    let is_progress = ev.kind.ends_with(".progress");
                    if is_progress {
                        let ok = match last_emit.get(&ev.kind) {
                            Some(t) => t.elapsed() >= PROGRESS_THROTTLE,
                            None => true,
                        };
                        if ok {
                            last_emit.insert(ev.kind.clone(), Instant::now());
                        } else {
                            continue;
                        }
                    }
                    if tx.send(ev).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    ReceiverStream::new(rx)
}

/// Build an SSE stream that replays recent history and then streams live
/// events, throttling `.progress` events per kind.
pub fn sse_stream(bus: EventBus) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = event_stream(bus).map(|ev| {
        Ok::<_, Infallible>(Event::default().data(serde_json::to_string(&ev).unwrap_or_default()))
    });

    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::StreamExt;

    /// Collect stream items until `deadline`, returning each event kind.
    async fn drain(
        stream: &mut ReceiverStream<Arc<AppEvent>>,
        deadline: tokio::time::Instant,
    ) -> Vec<String> {
        let mut kinds = Vec::new();
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                break;
            }
            match tokio::time::timeout(deadline - now, stream.next()).await {
                Ok(Some(ev)) => kinds.push(ev.kind.clone()),
                Ok(None) | Err(_) => break,
            }
        }
        kinds
    }

    #[tokio::test]
    async fn publishing_before_subscribing_is_replayed() {
        let bus = EventBus::new();
        bus.publish("transfer.added", serde_json::json!({"id": "1"}));
        bus.publish("transfer.completed", serde_json::json!({"id": "1"}));

        let mut stream = event_stream(bus);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);

        let kinds = drain(&mut stream, deadline).await;
        assert_eq!(kinds, vec!["transfer.added", "transfer.completed"]);
    }

    #[tokio::test]
    async fn progress_events_are_throttled_but_others_pass() {
        let bus = EventBus::new();
        let mut stream = event_stream(bus.clone());

        // Give the live subscriber task time to attach so the events below
        // are observed through the throttling path rather than history replay.
        tokio::time::sleep(Duration::from_millis(100)).await;

        for i in 0..5 {
            bus.publish(
                "transfer.progress",
                serde_json::json!({"id": "1", "pct": i * 20}),
            );
        }
        bus.publish("transfer.added", serde_json::json!({"id": "1"}));
        bus.publish("transfer.completed", serde_json::json!({"id": "1"}));

        let deadline = tokio::time::Instant::now() + Duration::from_millis(1300);
        let kinds = drain(&mut stream, deadline).await;

        let progress = kinds.iter().filter(|k| k.ends_with(".progress")).count();
        let other = kinds.iter().filter(|k| !k.ends_with(".progress")).count();
        assert!(progress < 5, "expected throttled progress, got {progress}");
        assert_eq!(other, 2, "non-progress events must all arrive");
    }
}

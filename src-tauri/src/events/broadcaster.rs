//! Broadcast-based event dispatcher for the standalone proxy process.
//!
//! Unlike `EventDispatcher` (which requires a Tauri `AppHandle`),
//! `BroadcastDispatcher` uses a `tokio::sync::broadcast` channel.
//! SSE consumers subscribe via `subscribe()` and receive a stream of events.

use tokio::sync::broadcast;

use super::dispatcher::DynDispatcher;
use super::types::ApertureEvent;

/// Default broadcast channel capacity.
/// Events are dropped for slow consumers once the buffer is full.
const BROADCAST_CAPACITY: usize = 256;

/// Event broadcaster that feeds SSE streams and produces a `DynDispatcher`.
#[derive(Clone)]
pub struct BroadcastDispatcher {
    sender: broadcast::Sender<ApertureEvent>,
}

impl BroadcastDispatcher {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self { sender }
    }

    /// Create a `DynDispatcher` whose `emit()` sends events into this broadcast channel.
    pub fn to_dyn_dispatcher(&self) -> DynDispatcher {
        let sender = self.sender.clone();
        DynDispatcher::from_fn(move |event: &ApertureEvent| {
            // send() fails only when there are no receivers — that's fine.
            let _ = sender.send(event.clone());
        })
    }

    /// Subscribe to the event stream. Returns a receiver that yields events.
    pub fn subscribe(&self) -> broadcast::Receiver<ApertureEvent> {
        self.sender.subscribe()
    }

    /// Number of active subscribers.
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for BroadcastDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for BroadcastDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BroadcastDispatcher")
            .field("receivers", &self.sender.receiver_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_broadcast_dispatcher_sends_events() {
        let broadcaster = BroadcastDispatcher::new();
        let mut rx = broadcaster.subscribe();
        let dispatcher = broadcaster.to_dyn_dispatcher();

        dispatcher.emit(&ApertureEvent::ContextUpdated {
            block_count: 5,
            total_tokens: 1000,
        });

        let event = rx.recv().await.unwrap();
        match event {
            ApertureEvent::ContextUpdated {
                block_count,
                total_tokens,
            } => {
                assert_eq!(block_count, 5);
                assert_eq!(total_tokens, 1000);
            }
            _ => panic!("unexpected event type"),
        }
    }

    #[tokio::test]
    async fn test_broadcast_no_receivers_does_not_panic() {
        let broadcaster = BroadcastDispatcher::new();
        let dispatcher = broadcaster.to_dyn_dispatcher();

        // Should not panic even with no receivers
        dispatcher.emit(&ApertureEvent::ContextUpdated {
            block_count: 0,
            total_tokens: 0,
        });
    }

    #[test]
    fn test_broadcast_receiver_count() {
        let broadcaster = BroadcastDispatcher::new();
        assert_eq!(broadcaster.receiver_count(), 0);

        let _rx1 = broadcaster.subscribe();
        assert_eq!(broadcaster.receiver_count(), 1);

        let _rx2 = broadcaster.subscribe();
        assert_eq!(broadcaster.receiver_count(), 2);

        drop(_rx1);
        assert_eq!(broadcaster.receiver_count(), 1);
    }
}

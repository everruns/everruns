// Event notification broadcaster for push-based SSE delivery
// Decision: Uses PostgreSQL NOTIFY/LISTEN to replace SSE polling (100-500ms) with
// push notifications (<20ms). Mirrors TaskNotificationBroadcaster pattern.

use sqlx::PgPool;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Payload sent when a new event is inserted
#[derive(Debug, Clone)]
pub struct EventNotificationPayload {
    pub session_id: Uuid,
}

/// Broadcaster that listens for PostgreSQL NOTIFY on 'event_available' channel
/// and broadcasts to SSE subscribers per session.
pub struct EventNotificationBroadcaster {
    sender: broadcast::Sender<EventNotificationPayload>,
    shutdown_tx: mpsc::Sender<()>,
}

impl EventNotificationBroadcaster {
    /// Create a new broadcaster and start listening for PostgreSQL NOTIFY events
    pub async fn new(pool: PgPool) -> Self {
        let (sender, _) = broadcast::channel(4096);
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        let sender_clone = sender.clone();
        tokio::spawn(async move {
            Self::listen_loop(pool, sender_clone, shutdown_rx).await;
        });

        Self {
            sender,
            shutdown_tx,
        }
    }

    /// Subscribe to event notifications. Returns a broadcast receiver that
    /// yields payloads for all sessions — callers filter by session_id.
    pub fn subscribe(&self) -> broadcast::Receiver<EventNotificationPayload> {
        self.sender.subscribe()
    }

    /// Signal shutdown of the LISTEN loop
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(()).await;
    }

    /// Background task that listens for PostgreSQL NOTIFY events
    async fn listen_loop(
        pool: PgPool,
        sender: broadcast::Sender<EventNotificationPayload>,
        mut shutdown_rx: mpsc::Receiver<()>,
    ) {
        info!("Starting PostgreSQL NOTIFY listener for event notifications");

        loop {
            let mut listener = match sqlx::postgres::PgListener::connect_with(&pool).await {
                Ok(listener) => listener,
                Err(e) => {
                    error!("Failed to create PostgreSQL listener for events: {}", e);
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => continue,
                        _ = shutdown_rx.recv() => {
                            info!("Event notification listener shutting down");
                            return;
                        }
                    }
                }
            };

            if let Err(e) = listener.listen("event_available").await {
                error!("Failed to LISTEN on event_available channel: {}", e);
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => continue,
                    _ = shutdown_rx.recv() => {
                        info!("Event notification listener shutting down");
                        return;
                    }
                }
            }

            info!("Listening for event_available notifications");

            loop {
                tokio::select! {
                    notification = listener.recv() => {
                        match notification {
                            Ok(notification) => {
                                let payload_str = notification.payload();
                                let session_id = match Uuid::parse_str(payload_str) {
                                    Ok(id) => id,
                                    Err(_) => {
                                        warn!(payload = %payload_str, "Invalid session_id in event notification");
                                        continue;
                                    }
                                };

                                debug!(%session_id, "Received event_available notification");
                                let _ = sender.send(EventNotificationPayload { session_id });
                            }
                            Err(e) => {
                                warn!("Error receiving event notification: {}", e);
                                break;
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Event notification listener shutting down");
                        return;
                    }
                }
            }
        }
    }
}

impl Drop for EventNotificationBroadcaster {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.try_send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_payload_debug() {
        let payload = EventNotificationPayload {
            session_id: Uuid::nil(),
        };
        let debug_str = format!("{:?}", payload);
        assert!(debug_str.contains("session_id"));
    }
}

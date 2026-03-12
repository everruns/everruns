// Notification broadcaster for push-based user notification SSE delivery.
// Decision: Mirrors event/task broadcasters so notification bell updates work
// across instances without falling back to short-polling.

use serde::Deserialize;
use sqlx::PgPool;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct NotificationNotificationPayload {
    pub org_id: i64,
    pub user_id: Uuid,
    pub action: String,
}

pub struct NotificationNotificationBroadcaster {
    sender: broadcast::Sender<NotificationNotificationPayload>,
    shutdown_tx: mpsc::Sender<()>,
}

impl NotificationNotificationBroadcaster {
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

    pub fn subscribe(&self) -> broadcast::Receiver<NotificationNotificationPayload> {
        self.sender.subscribe()
    }

    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(()).await;
    }

    async fn listen_loop(
        pool: PgPool,
        sender: broadcast::Sender<NotificationNotificationPayload>,
        mut shutdown_rx: mpsc::Receiver<()>,
    ) {
        info!("Starting PostgreSQL NOTIFY listener for notifications");

        loop {
            let mut listener = match sqlx::postgres::PgListener::connect_with(&pool).await {
                Ok(listener) => listener,
                Err(e) => {
                    error!(
                        "Failed to create PostgreSQL listener for notifications: {}",
                        e
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => continue,
                        _ = shutdown_rx.recv() => {
                            info!("Notification listener shutting down");
                            return;
                        }
                    }
                }
            };

            if let Err(e) = listener.listen("notification_available").await {
                error!("Failed to LISTEN on notification_available channel: {}", e);
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => continue,
                    _ = shutdown_rx.recv() => {
                        info!("Notification listener shutting down");
                        return;
                    }
                }
            }

            info!("Listening for notification_available notifications");

            loop {
                tokio::select! {
                    notification = listener.recv() => {
                        match notification {
                            Ok(notification) => {
                                match serde_json::from_str::<NotificationNotificationPayload>(notification.payload()) {
                                    Ok(payload) => {
                                        debug!(org_id = payload.org_id, user_id = %payload.user_id, action = %payload.action, "Received notification_available payload");
                                        let _ = sender.send(payload);
                                    }
                                    Err(e) => {
                                        warn!(payload = %notification.payload(), error = %e, "Invalid notification payload");
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("Error receiving notification payload: {}", e);
                                break;
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Notification listener shutting down");
                        return;
                    }
                }
            }
        }
    }
}

impl Drop for NotificationNotificationBroadcaster {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.try_send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_deserializes() {
        let payload = serde_json::from_str::<NotificationNotificationPayload>(
            r#"{"org_id":1,"user_id":"00000000-0000-0000-0000-000000000000","action":"insert"}"#,
        )
        .unwrap();
        assert_eq!(payload.org_id, 1);
        assert_eq!(payload.user_id, Uuid::nil());
        assert_eq!(payload.action, "insert");
    }
}

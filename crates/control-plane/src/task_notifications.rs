// Task notification broadcaster for push-based task notifications
// Decision: Uses PostgreSQL NOTIFY/LISTEN for low-latency task availability signaling
// Decision: Broadcasts to all subscribed workers matching the activity type

use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, mpsc};
use tracing::{debug, error, info, warn};

/// Task notification payload sent to workers
#[derive(Debug, Clone)]
pub struct TaskNotificationPayload {
    pub activity_type: String,
    pub pending_count: Option<i32>,
}

/// Handle for a worker subscription
pub struct WorkerSubscription {
    pub worker_id: String,
    pub activity_types: Vec<String>,
    pub receiver: broadcast::Receiver<TaskNotificationPayload>,
}

/// Broadcaster that listens for PostgreSQL NOTIFY events and broadcasts to workers
pub struct TaskNotificationBroadcaster {
    /// Broadcast sender for task notifications
    sender: broadcast::Sender<TaskNotificationPayload>,
    /// Currently subscribed workers (worker_id -> activity_types)
    subscribers: Arc<RwLock<HashMap<String, Vec<String>>>>,
    /// Shutdown signal
    shutdown_tx: mpsc::Sender<()>,
}

impl TaskNotificationBroadcaster {
    /// Create a new broadcaster and start listening for PostgreSQL NOTIFY events
    pub async fn new(pool: PgPool) -> Self {
        // Create broadcast channel with reasonable capacity
        // If workers are slow to consume, older notifications are dropped (acceptable since
        // workers will poll as fallback)
        let (sender, _) = broadcast::channel(1024);
        let subscribers = Arc::new(RwLock::new(HashMap::new()));
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        // Start the LISTEN loop in a background task
        let sender_clone = sender.clone();
        let pool_clone = pool.clone();
        tokio::spawn(async move {
            Self::listen_loop(pool_clone, sender_clone, shutdown_rx).await;
        });

        Self {
            sender,
            subscribers,
            shutdown_tx,
        }
    }

    /// Subscribe a worker to task notifications
    pub async fn subscribe(
        &self,
        worker_id: String,
        activity_types: Vec<String>,
    ) -> WorkerSubscription {
        // Register worker
        {
            let mut subs = self.subscribers.write().await;
            subs.insert(worker_id.clone(), activity_types.clone());
        }

        debug!(
            worker_id = %worker_id,
            activity_types = ?activity_types,
            "Worker subscribed to task notifications"
        );

        WorkerSubscription {
            worker_id,
            activity_types,
            receiver: self.sender.subscribe(),
        }
    }

    /// Unsubscribe a worker from task notifications
    pub async fn unsubscribe(&self, worker_id: &str) {
        let mut subs = self.subscribers.write().await;
        subs.remove(worker_id);
        debug!(worker_id = %worker_id, "Worker unsubscribed from task notifications");
    }

    /// Get the current number of subscribers
    pub async fn subscriber_count(&self) -> usize {
        self.subscribers.read().await.len()
    }

    /// Signal shutdown of the LISTEN loop
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(()).await;
    }

    /// Background task that listens for PostgreSQL NOTIFY events
    async fn listen_loop(
        pool: PgPool,
        sender: broadcast::Sender<TaskNotificationPayload>,
        mut shutdown_rx: mpsc::Receiver<()>,
    ) {
        info!("Starting PostgreSQL NOTIFY listener for task notifications");

        loop {
            // Acquire a dedicated connection for LISTEN
            let mut listener = match sqlx::postgres::PgListener::connect_with(&pool).await {
                Ok(listener) => listener,
                Err(e) => {
                    error!("Failed to create PostgreSQL listener: {}", e);
                    // Wait before retry
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => continue,
                        _ = shutdown_rx.recv() => {
                            info!("Task notification listener shutting down");
                            return;
                        }
                    }
                }
            };

            // Subscribe to the task_available channel
            if let Err(e) = listener.listen("task_available").await {
                error!("Failed to LISTEN on task_available channel: {}", e);
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => continue,
                    _ = shutdown_rx.recv() => {
                        info!("Task notification listener shutting down");
                        return;
                    }
                }
            }

            info!("Listening for task_available notifications");

            // Process notifications
            loop {
                tokio::select! {
                    notification = listener.recv() => {
                        match notification {
                            Ok(notification) => {
                                let activity_type = notification.payload().to_string();
                                debug!(
                                    activity_type = %activity_type,
                                    "Received task_available notification"
                                );

                                // Broadcast to all subscribers
                                let payload = TaskNotificationPayload {
                                    activity_type,
                                    pending_count: None, // Could query count, but adds latency
                                };

                                // Ignore send errors (no receivers is fine)
                                let _ = sender.send(payload);
                            }
                            Err(e) => {
                                warn!("Error receiving notification: {}", e);
                                // Connection might be broken, restart the loop
                                break;
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Task notification listener shutting down");
                        return;
                    }
                }
            }
        }
    }
}

impl Drop for TaskNotificationBroadcaster {
    fn drop(&mut self) {
        // Signal shutdown (best effort)
        let _ = self.shutdown_tx.try_send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_payload_debug() {
        let payload = TaskNotificationPayload {
            activity_type: "reason".to_string(),
            pending_count: Some(5),
        };
        let debug_str = format!("{:?}", payload);
        assert!(debug_str.contains("reason"));
        assert!(debug_str.contains("5"));
    }
}

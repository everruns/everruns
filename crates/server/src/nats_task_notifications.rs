// NATS-backed task notification broadcaster
//
// Decision: When NATS_URL is set, task notifications flow through NATS pub/sub
// instead of PostgreSQL NOTIFY/LISTEN. Workers are transparent to this — they
// still receive notifications via gRPC streaming. Only the server-side broadcaster
// implementation changes.
//
// NATS subjects: task.available.{activity_type}
// Each task enqueue publishes a notification to the activity type's subject.
// The broadcaster subscribes to task.available.> and forwards matching
// notifications to the broadcast channel (same as PG NOTIFY path).

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, mpsc};
use tracing::{debug, error, info, warn};

use crate::task_notifications::{TaskNotificationPayload, WorkerSubscription};

/// NATS subject prefix for task availability notifications
const NATS_TASK_SUBJECT_PREFIX: &str = "task.available";

/// NATS-backed task notification broadcaster.
/// Same public API as `TaskNotificationBroadcaster` — drop-in replacement.
pub struct NatsTaskNotificationBroadcaster {
    sender: broadcast::Sender<TaskNotificationPayload>,
    subscribers: Arc<RwLock<HashMap<String, Vec<String>>>>,
    shutdown_tx: mpsc::Sender<()>,
    /// NATS client for publishing task notifications (used by enqueue path)
    nats_client: async_nats::Client,
}

impl NatsTaskNotificationBroadcaster {
    /// Connect to NATS and start listening for task notifications.
    pub async fn new(nats_url: &str) -> Result<Self> {
        let client = async_nats::connect(nats_url)
            .await
            .context("Failed to connect to NATS for task notifications")?;

        let (sender, _) = broadcast::channel(1024);
        let subscribers = Arc::new(RwLock::new(HashMap::new()));
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        // Start NATS subscription loop
        let sender_clone = sender.clone();
        let client_clone = client.clone();
        tokio::spawn(async move {
            Self::listen_loop(client_clone, sender_clone, shutdown_rx).await;
        });

        info!("NATS task notification broadcaster ready");

        Ok(Self {
            sender,
            subscribers,
            shutdown_tx,
            nats_client: client,
        })
    }

    /// Subscribe a worker to task notifications (same API as PG broadcaster)
    pub async fn subscribe(
        &self,
        worker_id: String,
        activity_types: Vec<String>,
    ) -> WorkerSubscription {
        {
            let mut subs = self.subscribers.write().await;
            subs.insert(worker_id.clone(), activity_types.clone());
        }

        debug!(
            worker_id = %worker_id,
            activity_types = ?activity_types,
            "Worker subscribed to NATS task notifications"
        );

        WorkerSubscription {
            worker_id,
            activity_types,
            receiver: self.sender.subscribe(),
        }
    }

    /// Unsubscribe a worker
    pub async fn unsubscribe(&self, worker_id: &str) {
        let mut subs = self.subscribers.write().await;
        subs.remove(worker_id);
        debug!(worker_id = %worker_id, "Worker unsubscribed from NATS task notifications");
    }

    /// Get subscriber count
    pub async fn subscriber_count(&self) -> usize {
        self.subscribers.read().await.len()
    }

    /// Publish a task notification to NATS (called when a task is enqueued)
    pub async fn notify_task_available(&self, activity_type: &str) {
        let subject = format!("{NATS_TASK_SUBJECT_PREFIX}.{activity_type}");
        if let Err(e) = self
            .nats_client
            .publish(subject, activity_type.as_bytes().to_vec().into())
            .await
        {
            warn!(
                error = %e,
                activity_type,
                "Failed to publish task notification to NATS"
            );
        }
    }

    /// Signal shutdown
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(()).await;
        let _ = self.nats_client.drain().await;
    }

    /// Background loop: subscribe to task.available.> and broadcast to workers
    async fn listen_loop(
        client: async_nats::Client,
        sender: broadcast::Sender<TaskNotificationPayload>,
        mut shutdown_rx: mpsc::Receiver<()>,
    ) {
        use futures::StreamExt;

        info!("Starting NATS subscription for task notifications");

        let subject = format!("{NATS_TASK_SUBJECT_PREFIX}.>");
        let mut subscription = match client.subscribe(subject.clone()).await {
            Ok(sub) => sub,
            Err(e) => {
                error!(error = %e, "Failed to subscribe to NATS task notifications");
                return;
            }
        };

        info!(subject = %subject, "Listening for NATS task notifications");

        let prefix = format!("{NATS_TASK_SUBJECT_PREFIX}.");

        loop {
            tokio::select! {
                msg = subscription.next() => {
                    match msg {
                        Some(msg) => {
                            let activity_type = msg.subject
                                .strip_prefix(&prefix)
                                .unwrap_or(&msg.subject)
                                .to_string();

                            debug!(activity_type = %activity_type, "NATS task notification received");

                            let payload = TaskNotificationPayload {
                                activity_type,
                                pending_count: 0,
                            };
                            let _ = sender.send(payload);
                        }
                        None => {
                            warn!("NATS subscription closed, restarting...");
                            break;
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("NATS task notification listener shutting down");
                    return;
                }
            }
        }
    }
}

impl Drop for NatsTaskNotificationBroadcaster {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.try_send(());
    }
}

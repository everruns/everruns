// Task notification broadcaster for push-based task notifications
//
// Decision: Uses PostgreSQL NOTIFY/LISTEN by default. When NATS_URL is set,
// uses NATS pub/sub instead for lower latency and multi-instance support.
// Workers are transparent to the backend — they receive notifications via gRPC.
//
// The TaskBroadcaster enum wraps both backends with the same API.

use crate::live_updates::nats_task_notifications::NatsTaskNotificationBroadcaster;
use crate::live_updates::pg_listener_config::resolve_pg_listener_database_url;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, mpsc};
use tracing::{debug, error, info, warn};

/// Task notification payload sent to workers
#[derive(Debug, Clone)]
pub struct TaskNotificationPayload {
    pub activity_type: String,
    pub pending_count: i32,
    /// The one worker this wake-up is for (sticky sessions); `None` wakes
    /// every subscriber of the activity type.
    pub target_worker: Option<String>,
}

/// Separates an activity type from the worker a wake-up names, on the wire
/// (`session_actor@worker-…`, in a NOTIFY payload or a NATS message body).
const TARGET_SEPARATOR: char = '@';

impl TaskNotificationPayload {
    /// Parse a raw wake-up: an activity type, optionally `@` a worker id.
    pub fn parse(raw: &str) -> Self {
        let (activity_type, target_worker) = match raw.split_once(TARGET_SEPARATOR) {
            Some((activity_type, worker)) if !worker.is_empty() => {
                (activity_type.to_string(), Some(worker.to_string()))
            }
            Some((activity_type, _)) => (activity_type.to_string(), None),
            None => (raw.to_string(), None),
        };
        Self {
            activity_type,
            pending_count: 0,
            target_worker,
        }
    }

    /// The wire form [`Self::parse`] reads.
    pub fn encode(activity_type: &str, target_worker: Option<&str>) -> String {
        match target_worker {
            Some(worker) => format!("{activity_type}{TARGET_SEPARATOR}{worker}"),
            None => activity_type.to_string(),
        }
    }

    /// Whether the subscription `subscriber_id` should hear this wake-up. A
    /// worker subscribes under its id, or its id and a `:suffix` per loop
    /// (`worker-…:actors`); a targeted wake-up reaches only those.
    pub fn is_for(&self, subscriber_id: &str) -> bool {
        match &self.target_worker {
            None => true,
            Some(worker) => subscriber_id
                .strip_prefix(worker.as_str())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(':')),
        }
    }
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
    pub async fn new(database_url: String) -> Self {
        // Create broadcast channel with reasonable capacity
        // If workers are slow to consume, older notifications are dropped (acceptable since
        // workers will poll as fallback)
        let (sender, _) = broadcast::channel(1024);
        let subscribers = Arc::new(RwLock::new(HashMap::new()));
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        // Start the LISTEN loop in a background task
        let sender_clone = sender.clone();
        tokio::spawn(async move {
            Self::listen_loop(database_url, sender_clone, shutdown_rx).await;
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
        database_url: String,
        sender: broadcast::Sender<TaskNotificationPayload>,
        mut shutdown_rx: mpsc::Receiver<()>,
    ) {
        info!("Starting PostgreSQL NOTIFY listener for task notifications");

        loop {
            let mut listener = match sqlx::postgres::PgListener::connect(&database_url).await {
                Ok(listener) => listener,
                Err(e) => {
                    everruns_core::report_repeated_failure!(error, error = %e, "Failed to create PostgreSQL listener");
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
                                let payload = TaskNotificationPayload::parse(notification.payload());
                                debug!(
                                    activity_type = %payload.activity_type,
                                    target_worker = ?payload.target_worker,
                                    "Received task_available notification"
                                );

                                // Broadcast to all subscribers; each stream filters.

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

// ============================================================================
// TaskBroadcaster enum — unified API over PG and NATS backends
// ============================================================================

/// Unified task notification broadcaster.
/// Wraps either PG NOTIFY or NATS pub/sub with the same API.
/// Selected at startup based on NATS_URL environment variable.
pub enum TaskBroadcaster {
    /// PostgreSQL NOTIFY/LISTEN (default, no NATS required)
    Postgres(TaskNotificationBroadcaster),
    /// NATS pub/sub (when NATS_URL is set)
    Nats(NatsTaskNotificationBroadcaster),
}

impl TaskBroadcaster {
    /// Create from environment: NATS if `NATS_URL` is set, otherwise PG NOTIFY.
    /// Returns None if neither is available (dev mode without PG).
    pub async fn from_env(
        database_url: Option<&str>,
        database_unpooled_url: Option<&str>,
    ) -> Option<Self> {
        // Try NATS first (preferred when configured)
        if let Ok(nats_url) = std::env::var("NATS_URL") {
            match NatsTaskNotificationBroadcaster::new(&nats_url).await {
                Ok(nats) => {
                    info!("Task notifications: NATS pub/sub");
                    return Some(Self::Nats(nats));
                }
                Err(e) => {
                    warn!(
                        error = %crate::live_updates::nats::error_chain(&e),
                        "Failed to connect NATS for task notifications, falling back to PG NOTIFY"
                    );
                }
            }
        }

        // Fall back to PG NOTIFY
        let listener_database_url = match database_url {
            Some(database_url) => {
                match resolve_pg_listener_database_url(database_url, database_unpooled_url) {
                    Ok(listener_database_url) => Some(listener_database_url),
                    Err(error) => {
                        error!(
                            error = %error,
                            "Task notifications unavailable: listener database URL is not usable"
                        );
                        None
                    }
                }
            }
            None => None,
        };

        if let Some(database_url) = listener_database_url {
            let broadcaster = TaskNotificationBroadcaster::new(database_url).await;
            info!("Task notifications: PostgreSQL NOTIFY");
            return Some(Self::Postgres(broadcaster));
        }

        warn!("Task notification broadcaster not available (no NATS or PostgreSQL)");
        None
    }

    pub async fn subscribe(
        &self,
        worker_id: String,
        activity_types: Vec<String>,
    ) -> WorkerSubscription {
        match self {
            Self::Postgres(b) => b.subscribe(worker_id, activity_types).await,
            Self::Nats(b) => b.subscribe(worker_id, activity_types).await,
        }
    }

    pub async fn unsubscribe(&self, worker_id: &str) {
        match self {
            Self::Postgres(b) => b.unsubscribe(worker_id).await,
            Self::Nats(b) => b.unsubscribe(worker_id).await,
        }
    }

    pub async fn subscriber_count(&self) -> usize {
        match self {
            Self::Postgres(b) => b.subscriber_count().await,
            Self::Nats(b) => b.subscriber_count().await,
        }
    }

    /// Publish a task notification (NATS only — PG uses NOTIFY triggers).
    /// No-op for PG backend since NOTIFY is handled by database triggers.
    pub async fn notify_task_available(&self, activity_type: &str) {
        self.notify_worker(activity_type, None).await;
    }

    /// [`Self::notify_task_available`] for one worker's subscriptions only
    /// when `target_worker` names one.
    pub async fn notify_worker(&self, activity_type: &str, target_worker: Option<&str>) {
        if let Self::Nats(b) = self {
            b.notify_task_available_to(activity_type, target_worker)
                .await;
        }
    }

    pub async fn shutdown(&self) {
        match self {
            Self::Postgres(b) => b.shutdown().await,
            Self::Nats(b) => b.shutdown().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TaskNotificationPayload;

    #[test]
    fn a_plain_wake_up_reaches_every_subscriber() {
        let payload = TaskNotificationPayload::parse("session_actor");
        assert_eq!(payload.activity_type, "session_actor");
        assert!(payload.is_for("worker-1:actors"));
        assert!(payload.is_for("worker-2"));
    }

    #[test]
    fn a_targeted_wake_up_reaches_only_its_worker() {
        let raw = TaskNotificationPayload::encode("session_actor", Some("worker-1"));
        let payload = TaskNotificationPayload::parse(&raw);
        assert_eq!(payload.activity_type, "session_actor");
        assert!(payload.is_for("worker-1"));
        assert!(payload.is_for("worker-1:actors"));
        assert!(!payload.is_for("worker-2:actors"));
        // A worker whose id merely starts with the target's is another worker.
        assert!(!payload.is_for("worker-10:actors"));
    }

    #[test]
    fn an_empty_target_is_no_target() {
        let payload = TaskNotificationPayload::parse("session_actor@");
        assert_eq!(payload.activity_type, "session_actor");
        assert!(payload.is_for("worker-2:actors"));
    }
}

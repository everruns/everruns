// Push wake-ups for the task worker poll loop.
//
// Decision: the poll loop backs off to `poll_backoff_max` (5s by default) while
// the queue is empty, and every turn phase (input -> reason -> act -> reason)
// is its own queued task. Without a wake-up, a new message or the next phase
// waited for the backed-off poll, which added up to seconds per tool call.
// Two sources wake the loop now:
// - the control plane's task-notification stream (NATS or PG NOTIFY behind
//   the server), for work enqueued anywhere;
// - this worker finishing a task, since that usually enqueued the next phase.
// Polling stays as the fallback, so a lost notification costs at most one
// backoff interval, never a stuck turn.

use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
use tokio::sync::mpsc;
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::durable::StoreError;

use crate::unified_worker::TaskStore;

/// Receiver side of a push channel: each item means "new work may be claimable".
/// The channel closing means the push stream ended and the caller should resubscribe.
pub use everruns_durable_engine::task_store::TaskWakeups;

/// First resubscribe delay after the stream fails or ends.
const RESUBSCRIBE_BASE: Duration = Duration::from_secs(1);
/// Resubscribe delay cap, also used while the control plane reports push as unavailable.
const RESUBSCRIBE_MAX: Duration = Duration::from_secs(30);

/// Keep a push subscription open for the worker's lifetime and turn each
/// notification into a wake-up of the poll loop.
///
/// Returns immediately (the task ends) when the store has no push channel.
pub(crate) fn spawn_wakeup_listener<S: TaskStore>(
    store: Arc<S>,
    worker_id: String,
    activity_types: Vec<String>,
    wake: Arc<Notify>,
    shutdown_rx: watch::Receiver<bool>,
) -> JoinHandle<()> {
    let subscribe = move || {
        let store = store.clone();
        let worker_id = worker_id.clone();
        let activity_types = activity_types.clone();
        async move {
            store
                .subscribe_task_wakeups(&worker_id, &activity_types)
                .await
        }
    };
    tokio::spawn(listen(subscribe, wake, shutdown_rx, RESUBSCRIBE_BASE))
}

async fn listen<F, Fut>(
    subscribe: F,
    wake: Arc<Notify>,
    mut shutdown_rx: watch::Receiver<bool>,
    resubscribe_base: Duration,
) where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Option<TaskWakeups>, StoreError>>,
{
    let mut retry = resubscribe_base;
    let mut warned = false;
    loop {
        match subscribe().await {
            Ok(None) => {
                debug!("Task store has no push channel; polling only");
                return;
            }
            Ok(Some(mut wakeups)) => {
                info!("Subscribed to task notifications");
                retry = resubscribe_base;
                warned = false;
                // Work enqueued while we were disconnected sent no notification.
                wake.notify_one();
                loop {
                    tokio::select! {
                        item = wakeups.recv() => match item {
                            Some(()) => wake.notify_one(),
                            None => break,
                        },
                        _ = shutdown_rx.changed() => return,
                    }
                }
                warn!("Task notification stream ended; polling until resubscribed");
            }
            Err(error) => {
                // Logged once per outage: a control plane without a
                // broadcaster answers `unavailable` on every retry.
                if !warned {
                    warn!(error = %error, "Task notifications unavailable; polling until resubscribed");
                    warned = true;
                }
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(retry) => {}
            _ = shutdown_rx.changed() => return,
        }
        retry = (retry * 2).min(RESUBSCRIBE_MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const WAIT: Duration = Duration::from_secs(2);

    async fn woken(wake: &Notify) -> bool {
        tokio::time::timeout(WAIT, wake.notified()).await.is_ok()
    }

    #[tokio::test]
    async fn notification_wakes_the_poll_loop() {
        let (tx, rx) = mpsc::channel(1);
        let rx = Mutex::new(Some(rx));
        let wake = Arc::new(Notify::new());
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let subscribe = move || {
            let rx = rx.lock().unwrap().take();
            async move { Ok(rx) }
        };
        let handle = tokio::spawn(listen(subscribe, wake.clone(), shutdown_rx, WAIT));

        // Subscribing wakes once, for work enqueued while disconnected.
        assert!(woken(&wake).await, "subscribe should wake");
        tx.send(()).await.unwrap();
        assert!(woken(&wake).await, "notification should wake");
        handle.abort();
    }

    #[tokio::test]
    async fn ended_stream_resubscribes() {
        let subscriptions = Arc::new(AtomicUsize::new(0));
        let wake = Arc::new(Notify::new());
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let counter = subscriptions.clone();
        let subscribe = move || {
            counter.fetch_add(1, Ordering::SeqCst);
            // Sender dropped at once: the stream ends right after subscribing.
            let (_tx, rx) = mpsc::channel(1);
            async move { Ok(Some(rx)) }
        };
        let handle = tokio::spawn(listen(
            subscribe,
            wake.clone(),
            shutdown_rx,
            Duration::from_millis(10),
        ));
        assert!(woken(&wake).await);
        assert!(woken(&wake).await, "resubscribe should wake again");
        assert!(subscriptions.load(Ordering::SeqCst) >= 2);
        handle.abort();
    }

    #[tokio::test]
    async fn store_without_push_stops_listening() {
        let wake = Arc::new(Notify::new());
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let subscribe = || async { Ok(None) };
        tokio::time::timeout(WAIT, listen(subscribe, wake, shutdown_rx, WAIT))
            .await
            .expect("listener should return when the store has no push channel");
    }

    #[tokio::test]
    async fn shutdown_stops_a_failing_listener() {
        let wake = Arc::new(Notify::new());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let subscribe = || async { Err(StoreError::Database("unavailable".into())) };
        let handle = tokio::spawn(listen(subscribe, wake, shutdown_rx, WAIT));
        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(WAIT, handle)
            .await
            .expect("listener should stop on shutdown")
            .unwrap();
    }
}

//! Per-task heartbeat for a claimed durable task.
//!
//! It yields two signals (EVE-1134). Task cancellation fires on ownership loss
//! (the claim was reclaimed), on an explicit cancel, and on a failed heartbeat:
//! the activity must stop. The turn cancel fires only for an explicit cancel,
//! a heartbeat this worker still owns (`accepted`) whose workflow was
//! cancelled. Provider work a new owner could resume, such as an OpenAI
//! background response, stops on the turn cancel alone.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{oneshot, watch};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::unified_worker::TaskStore;

/// `(task cancellation, turn cancel)`.
pub(crate) type CancelSignals = (watch::Receiver<bool>, watch::Receiver<bool>);

pub(crate) fn spawn_task_heartbeat<S: TaskStore>(
    store: Arc<S>,
    task_id: Uuid,
    worker_id: String,
    heartbeat_interval: Duration,
) -> (
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
    CancelSignals,
) {
    let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
    let (task_cancel_tx, task_cancel_rx) = watch::channel(false);
    let (turn_cancel_tx, turn_cancel_rx) = watch::channel(false);
    let handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(heartbeat_interval);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    match store.heartbeat_task(task_id, &worker_id, None).await {
                        Ok(response) => {
                            if response.should_cancel {
                                // Still owned (`accepted`) means the workflow was cancelled.
                                let _ = turn_cancel_tx.send(response.accepted);
                                let _ = task_cancel_tx.send(true);
                                warn!(task_id = %task_id, "Task cancellation requested via heartbeat");
                                break;
                            }
                            debug!(task_id = %task_id, "Task heartbeat sent");
                        }
                        Err(error) => {
                            let _ = task_cancel_tx.send(true);
                            warn!(task_id = %task_id, error = %error, "Failed to send task heartbeat");
                        }
                    }
                }
                _ = &mut cancel_rx => {
                    debug!(task_id = %task_id, "Task heartbeat loop cancelled");
                    break;
                }
            }
        }
    });
    (cancel_tx, handle, (task_cancel_rx, turn_cancel_rx))
}

//! Activity execution context

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;
use uuid::Uuid;

/// Payload sent with heartbeats
#[derive(Debug, Clone)]
pub struct HeartbeatPayload {
    /// Optional progress details
    pub details: Option<serde_json::Value>,
}

/// Error from heartbeat operations
#[derive(Debug, thiserror::Error)]
pub enum HeartbeatError {
    /// Heartbeat channel closed (activity cancelled or timed out)
    #[error("heartbeat channel closed")]
    ChannelClosed,

    /// Activity was cancelled
    #[error("activity was cancelled")]
    Cancelled,
}

/// Context provided to activities during execution
///
/// The context provides:
/// - Information about the current execution attempt
/// - Heartbeat functionality for long-running activities
/// - Cancellation detection
///
/// # Example
///
/// ```ignore
/// async fn execute(&self, ctx: &ActivityContext, input: Input) -> Result<Output, ActivityError> {
///     for i in 0..100 {
///         // Check for cancellation
///         if ctx.is_cancelled() {
///             return Err(ActivityError::non_retryable("cancelled"));
///         }
///
///         // Do work...
///         do_work(i).await?;
///
///         // Send heartbeat with progress
///         ctx.heartbeat(Some(json!({"progress": i}))).await?;
///     }
///
///     Ok(Output { ... })
/// }
/// ```
#[derive(Debug)]
pub struct ActivityContext {
    /// Unique execution attempt ID
    pub attempt_id: Uuid,

    /// Current attempt number (1-based)
    pub attempt: u32,

    /// Maximum attempts allowed
    pub max_attempts: u32,

    /// Workflow instance ID that owns this activity
    pub workflow_id: Uuid,

    /// Activity ID within the workflow
    pub activity_id: String,

    /// Heartbeat sender
    heartbeat_tx: Option<mpsc::Sender<HeartbeatPayload>>,

    /// Cancellation flag
    cancelled: Arc<AtomicBool>,
}

impl ActivityContext {
    /// Create a new activity context
    pub fn new(workflow_id: Uuid, activity_id: String, attempt: u32, max_attempts: u32) -> Self {
        Self {
            attempt_id: Uuid::now_v7(),
            attempt,
            max_attempts,
            workflow_id,
            activity_id,
            heartbeat_tx: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Create a context with heartbeat support
    pub fn with_heartbeat(mut self, tx: mpsc::Sender<HeartbeatPayload>) -> Self {
        self.heartbeat_tx = Some(tx);
        self
    }

    /// Get a handle that can be used to cancel this activity
    pub fn cancellation_handle(&self) -> CancellationHandle {
        CancellationHandle {
            cancelled: self.cancelled.clone(),
        }
    }

    /// Record a heartbeat
    ///
    /// Heartbeats serve two purposes:
    /// 1. Keep the activity alive (prevent heartbeat timeout)
    /// 2. Report progress to the workflow system
    ///
    /// # Errors
    ///
    /// Returns an error if the activity has been cancelled or the
    /// heartbeat channel is closed.
    pub async fn heartbeat(
        &self,
        details: Option<serde_json::Value>,
    ) -> Result<(), HeartbeatError> {
        // Check cancellation first
        if self.is_cancelled() {
            return Err(HeartbeatError::Cancelled);
        }

        // Send heartbeat if channel is available
        if let Some(tx) = &self.heartbeat_tx {
            tx.send(HeartbeatPayload { details })
                .await
                .map_err(|_| HeartbeatError::ChannelClosed)?;
        }

        Ok(())
    }

    /// Check if cancellation was requested
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Get a future that resolves when cancellation is requested
    ///
    /// This is useful for select! patterns:
    ///
    /// ```ignore
    /// tokio::select! {
    ///     result = do_work() => { ... }
    ///     _ = ctx.cancelled() => {
    ///         return Err(ActivityError::non_retryable("cancelled"));
    ///     }
    /// }
    /// ```
    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Check if this is the last retry attempt
    pub fn is_last_attempt(&self) -> bool {
        self.attempt >= self.max_attempts
    }
}

/// Handle to cancel an activity
#[derive(Debug, Clone)]
pub struct CancellationHandle {
    cancelled: Arc<AtomicBool>,
}

impl CancellationHandle {
    /// Cancel the activity
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activity_context_creation() {
        let workflow_id = Uuid::now_v7();
        let ctx = ActivityContext::new(workflow_id, "step-1".to_string(), 1, 3);

        assert_eq!(ctx.workflow_id, workflow_id);
        assert_eq!(ctx.activity_id, "step-1");
        assert_eq!(ctx.attempt, 1);
        assert_eq!(ctx.max_attempts, 3);
        assert!(!ctx.is_cancelled());
        assert!(!ctx.is_last_attempt());
    }

    #[test]
    fn test_is_last_attempt() {
        let ctx = ActivityContext::new(Uuid::now_v7(), "step-1".to_string(), 3, 3);
        assert!(ctx.is_last_attempt());

        let ctx = ActivityContext::new(Uuid::now_v7(), "step-1".to_string(), 2, 3);
        assert!(!ctx.is_last_attempt());
    }

    #[test]
    fn test_cancellation() {
        let ctx = ActivityContext::new(Uuid::now_v7(), "step-1".to_string(), 1, 3);
        let handle = ctx.cancellation_handle();

        assert!(!ctx.is_cancelled());

        handle.cancel();

        assert!(ctx.is_cancelled());
    }

    #[tokio::test]
    async fn test_heartbeat_when_cancelled() {
        let ctx = ActivityContext::new(Uuid::now_v7(), "step-1".to_string(), 1, 3);
        let handle = ctx.cancellation_handle();

        handle.cancel();

        let result = ctx.heartbeat(None).await;
        assert!(matches!(result, Err(HeartbeatError::Cancelled)));
    }

    #[tokio::test]
    async fn test_heartbeat_with_channel() {
        let (tx, mut rx) = mpsc::channel(10);
        let ctx =
            ActivityContext::new(Uuid::now_v7(), "step-1".to_string(), 1, 3).with_heartbeat(tx);

        ctx.heartbeat(Some(serde_json::json!({"progress": 50})))
            .await
            .unwrap();

        let payload = rx.recv().await.unwrap();
        assert!(payload.details.is_some());
    }
}

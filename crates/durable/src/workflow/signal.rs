//! Workflow signals for external communication

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// External signals that can be sent to running workflows
///
/// Signals allow external systems to communicate with running workflows.
/// They are processed asynchronously and trigger `on_signal` callbacks.
///
/// # Example
///
/// ```
/// use everruns_durable::{EventLog, InMemoryWorkflowEventStore, SignalStore, WorkflowSignal};
/// use serde_json::json;
///
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() -> Result<(), everruns_durable::StoreError> {
/// let store = InMemoryWorkflowEventStore::new();
/// let workflow_id = uuid::Uuid::now_v7();
/// store.create_workflow(workflow_id, "order", json!({}), None).await?;
///
/// store.send_signal(workflow_id, WorkflowSignal::cancel("user requested")).await?;
///
/// // The consumer drains pending signals on its next pass.
/// let pending = store.get_pending_signals(workflow_id).await?;
/// assert!(pending[0].is_cancel());
/// assert_eq!(pending[0].payload["reason"], "user requested");
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowSignal {
    /// Signal type identifier
    pub signal_type: String,

    /// Signal payload (JSON)
    pub payload: serde_json::Value,

    /// When the signal was sent
    pub sent_at: DateTime<Utc>,
}

impl WorkflowSignal {
    /// Create a new signal
    pub fn new(signal_type: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            signal_type: signal_type.into(),
            payload,
            sent_at: Utc::now(),
        }
    }

    /// Create a cancellation signal
    pub fn cancel(reason: impl Into<String>) -> Self {
        Self::new(
            signal_types::CANCEL,
            serde_json::json!({ "reason": reason.into() }),
        )
    }

    /// Create a shutdown signal (graceful)
    pub fn shutdown() -> Self {
        Self::new(signal_types::SHUTDOWN, serde_json::json!({}))
    }

    /// Create a custom signal
    pub fn custom(name: impl Into<String>, payload: serde_json::Value) -> Self {
        Self::new(name, payload)
    }

    /// Check if this is a cancellation signal
    pub fn is_cancel(&self) -> bool {
        self.signal_type == signal_types::CANCEL
    }

    /// Check if this is a shutdown signal
    pub fn is_shutdown(&self) -> bool {
        self.signal_type == signal_types::SHUTDOWN
    }
}

/// Signal types the engine itself interprets. Applications define their own
/// signal types (any string) on top of these.
pub mod signal_types {
    /// Request workflow cancellation (immediate)
    pub const CANCEL: &str = "cancel";

    /// Request graceful shutdown (complete current activity, then stop)
    pub const SHUTDOWN: &str = "shutdown";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cancel_signal() {
        let signal = WorkflowSignal::cancel("user cancelled");

        assert!(signal.is_cancel());
        assert!(!signal.is_shutdown());
        assert_eq!(signal.signal_type, signal_types::CANCEL);
    }

    #[test]
    fn test_shutdown_signal() {
        let signal = WorkflowSignal::shutdown();

        assert!(signal.is_shutdown());
        assert!(!signal.is_cancel());
    }

    #[test]
    fn test_custom_signal() {
        let signal =
            WorkflowSignal::custom("order_updated", serde_json::json!({"status": "shipped"}));

        assert_eq!(signal.signal_type, "order_updated");
        assert!(!signal.is_cancel());
    }

    #[test]
    fn test_signal_serialization() {
        let signal = WorkflowSignal::cancel("test");

        let json = serde_json::to_string(&signal).unwrap();
        let parsed: WorkflowSignal = serde_json::from_str(&json).unwrap();

        assert_eq!(signal.signal_type, parsed.signal_type);
        assert_eq!(signal.payload, parsed.payload);
    }
}

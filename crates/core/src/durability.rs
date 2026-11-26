//! Neutral contracts for durable tool results and stream recovery.

use crate::error::Result;
use crate::typed_id::{MessageId, SessionId};
use async_trait::async_trait;

pub use everruns_contracts::runtime::durable_tools::{
    DurableToolCallStatus, DurableToolResultStore, ToolCallClaimResult,
};

// ============================================================================
// PartialStreamStore — partial-stream recovery for Reason activity (EVE-532)
// ============================================================================

/// State of a partially-streamed assistant message detected in the event log.
#[derive(Debug, Clone)]
pub struct PartialStreamState {
    /// Prepared Astra effort recovered from the matching stream-start event.
    pub reasoning_state: Option<everruns_contracts::reasoning_updates::ReasoningState>,
    /// Stable public id from the latest `output.message.started` event.
    pub message_id: MessageId,

    /// Accumulated text from the last `output.message.delta` for the turn.
    /// Empty when `output.message.started` was emitted but no delta arrived.
    pub accumulated: String,

    /// Whether the attempt that opened this stream later recorded
    /// `reason.completed`: it failed and closed its own step (a transient
    /// provider error the engine retries) rather than dying mid-step.
    /// A settled attempt's partial text is never finalized, and the retry
    /// announces its step again; it still reuses `message_id`.
    pub attempt_settled: bool,
}

/// Consults the persisted event log to detect whether a `reason` activity
/// was interrupted after `output.message.started` but before
/// `output.message.completed` or `output.message.replaced`.
///
/// Used by `ReasonAtom` on re-entry to apply the ContinuePartial recovery
/// policy (EVE-532): finalize the partial text without a second provider call,
/// or restart clean if the partial is unusable. Either way the retry reuses
/// the partial's message id and does not announce it again, so every
/// `output.message.started` gets exactly one completion.
#[async_trait]
pub trait PartialStreamStore: Send + Sync {
    /// Return the partial-stream state for `(session_id, turn_id)` if an
    /// in-flight assistant message exists (started but not completed).
    async fn get_partial_stream(
        &self,
        session_id: SessionId,
        turn_id: &str,
    ) -> Result<Option<PartialStreamState>>;
}

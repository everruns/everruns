//! Neutral contracts for durable tool results and stream recovery.

use crate::error::Result;
use crate::typed_id::{MessageId, SessionId};
use async_trait::async_trait;

/// Result of a claim attempt on the per-tool-call idempotency store.
#[derive(Debug)]
pub enum ToolCallClaimResult {
    /// First claim for this (turn_id, tool_call_id); caller should execute the tool.
    /// `claim_token` must be passed to `settle_tool_call` to verify ownership.
    Claimed { claim_token: uuid::Uuid },
    /// A prior run already settled this call; replay the stored result.
    AlreadySettled {
        result_json: serde_json::Value,
        args_fingerprint: String,
    },
    /// A prior run started but never settled. For `AtMostOnce` tools the
    /// caller should NOT re-execute; for `Pure`/`Idempotent` tools the caller
    /// may re-execute and then try to settle (the settle CAS will be a no-op if
    /// a different claimer wins first).
    AlreadyRunning { args_fingerprint: String },
    /// A settled row exists but its `args_fingerprint` does not match the
    /// current call — this is a determinism violation (workflow replay with
    /// different inputs). The workflow should be failed loudly.
    DeterminismViolation {
        stored_fingerprint: String,
        current_fingerprint: String,
    },
}

/// Read-only status of a tool call in durable storage (EVE-533).
#[derive(Debug, Clone)]
pub enum DurableToolCallStatus {
    /// Tool completed successfully or with an error; result is stored.
    Settled { result_json: serde_json::Value },
    /// Tool was settled with `interrupted` status; result may contain error details.
    Interrupted {
        result_json: Option<serde_json::Value>,
    },
    /// A claim exists but the tool never finished.
    Running,
}

/// Durable per-tool-call idempotency store (EVE-530).
///
/// Implements the claim/settle CAS that prevents double-execution of
/// `AtMostOnce` tools on worker reclaim/replay.
#[async_trait]
pub trait DurableToolResultStore: Send + Sync + 'static {
    /// Atomically claim `(turn_id, tool_call_id)` before tool dispatch.
    ///
    /// - Inserts a `running` row if none exists → `Claimed`.
    /// - Finds an existing `settled` row → `AlreadySettled`.
    /// - Finds an existing `running` row → `AlreadyRunning`.
    /// - Finds a `settled` row with a mismatched `args_fingerprint`
    ///   (determinism violation) → `DeterminismViolation`.
    async fn try_claim_tool_call(
        &self,
        turn_id: &str,
        tool_call_id: &str,
        tool_name: &str,
        args_fingerprint: &str,
    ) -> Result<ToolCallClaimResult>;

    /// Settle a previously claimed tool call with its result.
    ///
    /// `claim_token` must match the token returned by `try_claim_tool_call`.
    /// Returns `Ok(true)` if the row was updated, `Ok(false)` if the claim
    /// token no longer matches (ownership lost — treat as a warning).
    async fn settle_tool_call(
        &self,
        turn_id: &str,
        tool_call_id: &str,
        result_json: serde_json::Value,
        status: &str,
        claim_token: uuid::Uuid,
    ) -> Result<bool>;

    /// Read-only lookup of a tool call's current status in durable storage (EVE-533).
    ///
    /// Used by transcript repair to decide whether to replay a stored result or
    /// synthesize an interrupted placeholder. Returns `None` if no row exists.
    async fn get_tool_call_status(
        &self,
        turn_id: &str,
        tool_call_id: &str,
    ) -> Result<Option<DurableToolCallStatus>>;
}

// ============================================================================
// StreamHeartbeater — per-stream liveness signal for Reason activity (EVE-531)
// ============================================================================

/// Progress snapshot carried in each stream heartbeat.
#[derive(Debug, Clone)]
pub struct StreamProgress {
    /// Accumulated text + thinking length (characters) at the time of heartbeat.
    pub accumulated_len: usize,
    /// Wall-clock time of the most recent received token (Unix seconds).
    pub last_delta_at: u64,
}

/// Heartbeater the Reason streaming loop calls on delta batches and a keepalive
/// timer, signalling that the provider connection is alive.
///
/// Implementations bridge to the durable-execution layer (e.g. gRPC).
#[async_trait]
pub trait StreamHeartbeater: Send + Sync {
    /// Signal stream liveness with current progress.
    ///
    /// Must be best-effort: errors must not propagate to the caller.
    /// Cancel-safety is critical — if the worker dies the heartbeat stops
    /// and the existing task-level reclaim takes over.
    async fn heartbeat(&self, progress: StreamProgress);
}

// ============================================================================
// PartialStreamStore — partial-stream recovery for Reason activity (EVE-532)
// ============================================================================

/// State of a partially-streamed assistant message detected in the event log.
#[derive(Debug, Clone)]
pub struct PartialStreamState {
    /// Prepared Astra effort recovered from the matching stream-start event.
    pub reasoning_state: Option<everruns_provider::reasoning_updates::ReasoningState>,
    /// Stable public id from the latest `output.message.started` event.
    pub message_id: MessageId,

    /// Accumulated text from the last `output.message.delta` for the turn.
    /// Empty when `output.message.started` was emitted but no delta arrived.
    pub accumulated: String,
}

/// Consults the persisted event log to detect whether a `reason` activity
/// was interrupted after `output.message.started` but before
/// `output.message.completed` or `output.message.replaced`.
///
/// Used by `ReasonAtom` on re-entry to apply the ContinuePartial recovery
/// policy (EVE-532): finalize the partial text without a second provider call,
/// or restart clean if the partial is unusable.
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

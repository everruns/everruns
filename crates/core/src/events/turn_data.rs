//! Payloads for turn and session lifecycle events.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::typed_id::{AgentId, HarnessId, MessageId, ModelId, TurnId};
use crate::user_facing_error::UserFacingErrorFields;

use super::*;

/// Data for turn.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TurnStartedData {
    /// Turn identifier
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Input message ID that triggered this turn
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "message_01933b5a00007000800000000000001"))]
    pub input_message_id: MessageId,

    /// Input message content (for observability)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_content: Option<String>,

    /// Agent the turn runs as, when the session is bound to one. Carried on
    /// the turn root so trace exporters can label the `invoke_agent` span
    /// without a store lookup (the Gen-AI conventions want the agent name in
    /// the span name and `gen_ai.agent.*` attributes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001"))]
    pub agent_id: Option<AgentId>,

    /// Human-readable agent name snapshot at turn start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,

    /// Agent description snapshot at turn start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_description: Option<String>,
}

/// Data for turn.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TurnCompletedData {
    /// Turn identifier
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Number of iterations in this turn
    pub iterations: u32,

    /// Duration in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,

    /// Aggregated token usage for all LLM calls in this turn
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,

    /// Input message content (for observability, passed through from turn.started)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_content: Option<String>,

    /// Canonical assistant message emitted by `output.message.completed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "message_01933b5a00007000800000000000001"))]
    pub final_message_id: Option<MessageId>,

    /// Bounded preview of the final visible assistant answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_answer_preview: Option<String>,

    /// First-token latency for the turn, usually from the first LLM generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<u64>,

    /// Number of tool calls completed during the turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_count: Option<u32>,

    /// Number of LLM generation calls executed during the turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_call_count: Option<u32>,

    /// Optional explicit completion status for consumers that summarize turns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// Data for turn.failed event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TurnFailedData {
    /// Turn identifier
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Error message
    pub error: String,

    /// Error code
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,

    /// Structured interpolation fields for localized error rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub error_fields: Option<UserFacingErrorFields>,

    /// Error-disclosure mode applied to `error_code`/`error_fields`
    /// ("generic" | "standard" | "detailed"). Full diagnostic detail remains
    /// available to operators via reason.completed failure events and tracing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_disclosure: Option<String>,
}

/// Data for turn.sealed event (EVE-534).
///
/// A sealed turn was deliberately stopped to prevent waste. It is observably
/// distinct from `turn.completed` (success) and `turn.failed` (error). The
/// `reason` is the durable engine's stable seal-reason wire value.
/// (`"no_progress"` or `"budget"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TurnSealedData {
    /// Turn identifier
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Why the turn was sealed: `"no_progress"` (crash-loop with no forward
    /// progress) or `"budget"` (work budget exhausted).
    pub reason: String,

    /// Human-readable detail for operators (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,

    /// Iterations completed before the turn was sealed (if known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,

    /// Aggregated token usage before sealing, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// Data for turn.cancelled event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TurnCancelledData {
    /// Turn identifier
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Reason for cancellation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Token usage before cancellation (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

// ============================================================================
// Session Event Data Types
// ============================================================================

/// Data for session.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionStartedData {
    /// Harness ID
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "harness_01933b5a00007000800000000000001"))]
    pub harness_id: HarnessId,

    /// Agent ID (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001"))]
    pub agent_id: Option<AgentId>,

    /// Model ID if specified
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001"))]
    pub model_id: Option<ModelId>,
}

/// Data for session.activated event (turn started, session now active)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionActivatedData {
    /// Turn ID that activated the session
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Input message ID that triggered the turn
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "message_01933b5a00007000800000000000001"))]
    pub input_message_id: MessageId,
}

/// Data for session.idled event (turn completed, session now idle)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionIdledData {
    /// Turn ID that just completed
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Number of iterations in the completed turn
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,

    /// Cumulative token usage for the session at this point
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

/// Data for `session.title.updated`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionTitleUpdatedData {
    /// Title before the mutation. `None` means the session was untitled.
    pub previous_title: Option<String>,

    /// New session title.
    pub title: String,
}

/// Data for `session.model.changed`.
///
/// Names are the provider's own model identifiers, captured at emission time so
/// the transcript stays readable after a model is renamed or removed from the
/// organization. Clients that still have the model may prefer its display name.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionModelChangedData {
    /// Model used before the switch. `None` when the previous turn ran on an
    /// inherited default that the emitter could not name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001"))]
    pub previous_model_id: Option<ModelId>,

    /// Name of the previous model, captured at emission time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_model_name: Option<String>,

    /// Model selected for the next turn.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "model_01933b5a00007000800000000000002"))]
    pub model_id: ModelId,

    /// Name of the selected model, captured at emission time.
    pub model_name: String,
}

// ============================================================================
// Session task event data
// ============================================================================

/// Data for task lifecycle events (`task.created`, `task.updated`).
///
/// Carries the full task snapshot so consumers never need a follow-up read;
/// UIs reconcile by `task.id` (snapshot-then-delta).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionTaskEventData {
    pub task: crate::session_task::SessionTask,
}

/// Data for task message events (`task.message.sent`, `task.message.received`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TaskMessageEventData {
    pub task_id: String,
    pub message: crate::session_task::TaskMessage,
}

// ============================================================================
// Context compaction event data
// ============================================================================

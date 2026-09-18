//! Payloads for reasoning events, including thinking deltas and items.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::typed_id::{AgentId, HarnessId, TurnId};

use super::*;

/// Data for reason.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonStartedData {
    /// Harness ID being used
    pub harness_id: HarnessId,

    /// Agent ID being used (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,

    /// Metadata about the model being used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ModelMetadata>,
}

/// Data for reason.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonCompletedData {
    /// Whether the LLM call succeeded
    pub success: bool,

    /// Text response preview (first 200 chars)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_preview: Option<String>,

    /// Whether tool calls were requested
    pub has_tool_calls: bool,

    /// Number of tool calls requested
    pub tool_call_count: u32,

    /// Error message if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Duration of the reason phase in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,

    /// Token usage from the LLM call
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

impl ReasonCompletedData {
    pub fn success(
        text: &str,
        has_tool_calls: bool,
        tool_call_count: u32,
        duration_ms: Option<u64>,
        usage: Option<TokenUsage>,
    ) -> Self {
        let text_preview = if text.is_empty() {
            None
        } else {
            Some(text.chars().take(200).collect())
        };

        Self {
            success: true,
            text_preview,
            has_tool_calls,
            tool_call_count,
            error: None,
            duration_ms,
            usage,
        }
    }

    pub fn failure(error: String, duration_ms: Option<u64>) -> Self {
        Self {
            success: false,
            text_preview: None,
            has_tool_calls: false,
            tool_call_count: 0,
            error: Some(error),
            duration_ms,
            usage: None,
        }
    }
}

/// Recovery mode chosen by the ContinuePartial classifier (EVE-532).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RecoveryMode {
    /// Persisted accumulated text was finalised as the assistant message;
    /// no second provider call was made.
    Finalize,
    /// Partial was unusable (empty accumulated); re-issued the provider call.
    Restart,
}

/// Data for the `reason.recovered` event (EVE-532).
///
/// Emitted by `ReasonAtom` when it detects an in-flight partial assistant
/// message from a previous worker execution and applies the ContinuePartial
/// recovery policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonRecoveredData {
    /// Turn ID the partial belonged to.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Recovery action taken.
    pub mode: RecoveryMode,

    /// Character length of the persisted accumulated text.
    pub accumulated_len: usize,
}

/// Data for reason.thinking.started event
///
/// Emitted when extended thinking begins during reasoning phase.
/// This signals the model is using chain-of-thought reasoning.
/// UI can show a "thinking" indicator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonThinkingStartedData {
    /// Turn ID this thinking belongs to
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Optional model name being used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Data for reason.thinking.delta event (extended thinking content from models like Claude)
///
/// This event streams incremental thinking/reasoning content from models that support
/// extended thinking mode (e.g., Claude with thinking enabled). The thinking content
/// represents the model's chain-of-thought reasoning before producing the final response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonThinkingDeltaData {
    /// Turn ID this delta belongs to (for correlation)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// The thinking delta (new thinking text since last delta)
    pub delta: String,

    /// Accumulated thinking text so far (convenience for UI)
    pub accumulated: String,
}

/// Data for reason.thinking.completed event
///
/// Emitted when extended thinking completes and the model transitions
/// to producing the final response. Contains the complete thinking content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonThinkingCompletedData {
    /// Turn ID this thinking belongs to
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Complete thinking content
    pub thinking: String,
}

/// Data for `reason.item` event.
///
/// Durable record of one provider reasoning artifact.
///
/// Carries identity and curated summary text only. Opaque replay state
/// (signatures, encrypted reasoning context) is deliberately absent: it is
/// state the driver hands back to the provider, not content, and it must not
/// reach an event stream or any API surface. Plaintext chain-of-thought is
/// likewise never persisted here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasonItemData {
    /// Turn ID this reasoning item belongs to.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Provider that produced the reasoning item (e.g., "openai").
    pub provider: String,

    /// Model identifier reported by the provider, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Provider-assigned identifier for the reasoning item.
    pub item_id: String,

    /// Safe summary text segments curated by the provider. Never includes
    /// plaintext reasoning content.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summary: Vec<String>,

    /// Per-item reasoning token count, when the provider reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_count: Option<u32>,
}

// ============================================================================
// Turn Event Data Types
// ============================================================================

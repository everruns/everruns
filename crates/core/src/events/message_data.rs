//! Payloads for the input and output message events.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::typed_id::{MessageId, TurnId};
use crate::user_facing_error::{UserFacingError, UserFacingErrorFields};

use super::*;

/// Data for input.message event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct InputMessageData {
    /// The user message
    pub message: RuntimeMessage,
}

impl InputMessageData {
    pub fn new(message: RuntimeMessage) -> Self {
        Self { message }
    }
}

// ============================================================================
// Output Event Data Types
// ============================================================================

/// Data for output.message.started event
///
/// Emitted when the LLM starts generating a response. UI can show a
/// "thinking" indicator until output.message.delta or output.message.completed events arrive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct OutputMessageStartedData {
    /// Prepared Astra effort state, persisted before the provider call so an
    /// interrupted worker can resume without changing the request baseline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_state: Option<everruns_provider::reasoning_updates::ReasoningState>,
    /// Turn ID this output belongs to
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Stable public ID for this assistant message across its streaming lifecycle.
    /// This is the same identifier as `OutputMessageCompletedData.message.id`.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "message_550e8400e29b41d4a716446655440000"))]
    pub message_id: MessageId,

    /// Optional model name being used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Current iteration number within this turn (1-based).
    /// Useful for UI to show progress during multi-step tool-calling flows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,

    /// Best-effort streamed phase hint (EVE-774).
    ///
    /// `None` means "not yet classified — treat as ordinary assistant text",
    /// NEVER "thinking". A missing/unknown phase must fall back to the
    /// assistant-text channel, never the reasoning channel. Only populated when
    /// the provider stream reveals a native phase before this event is emitted;
    /// today `output.message.started` is emitted before the LLM call, so this is
    /// generally `None` at start. The authoritative classification remains the
    /// completed `RuntimeMessage.phase`. See `knowledge/execution/events.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<ExecutionPhase>,
}

/// Data for output.message.delta event
///
/// Incremental text update during LLM generation. Events are batched (~100ms)
/// to reduce volume while providing real-time feedback.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct OutputMessageDeltaData {
    /// Turn ID this delta belongs to
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Stable public ID for this assistant message across its streaming lifecycle.
    /// This is the same identifier as `OutputMessageCompletedData.message.id`.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "message_550e8400e29b41d4a716446655440000"))]
    pub message_id: MessageId,

    /// The new text chunk
    pub delta: String,

    /// Accumulated text so far
    pub accumulated: String,

    /// Best-effort streamed phase hint (EVE-774).
    ///
    /// `None` means "not yet classified — treat as ordinary assistant text",
    /// NEVER "thinking". Refinement is monotonic: once a stream reveals a native
    /// phase (`Commentary` or `FinalAnswer`) it stays fixed for the rest of the
    /// message — it never flip-flops and never reverts to `None`
    /// (see `ExecutionPhase::refine_streamed_hint`). Providers without native
    /// mid-stream phase (Anthropic, Gemini, …) leave this `None` until
    /// completion. The authoritative classification remains the completed
    /// `RuntimeMessage.phase`. See `knowledge/execution/events.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<ExecutionPhase>,
}

/// Data for output.message.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct OutputMessageCompletedData {
    /// The agent message
    pub message: RuntimeMessage,

    /// Metadata about the model used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ModelMetadata>,

    /// Token usage
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,

    /// Stable error code for user-facing failures surfaced as assistant text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,

    /// Structured interpolation fields for localized error rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub error_fields: Option<UserFacingErrorFields>,

    /// Error-disclosure mode applied to `error_code`/`error_fields`
    /// ("generic" | "standard" | "detailed"). Tracking metadata; absent for
    /// non-error messages and for paths that predate disclosure modes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_disclosure: Option<String>,
}

impl OutputMessageCompletedData {
    pub fn new(message: RuntimeMessage) -> Self {
        Self {
            message,
            metadata: None,
            usage: None,
            error_code: None,
            error_fields: None,
            error_disclosure: None,
        }
    }

    pub fn with_metadata(mut self, metadata: ModelMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn with_usage(mut self, usage: TokenUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn with_user_facing_error(mut self, error: &UserFacingError) -> Self {
        error.apply_to_event_fields(&mut self.error_code, &mut self.error_fields);
        self
    }

    pub fn with_error_disclosure(mut self, mode: crate::ErrorDisclosure) -> Self {
        self.error_disclosure = Some(mode.as_str().to_string());
        self
    }
}

/// Data for `output.message.replaced` event.
///
/// Emitted between the last (suppressed) `output.message.delta` and the final
/// `output.message.completed`. Tells the client to discard the text accumulated
/// for `message_id` and use `replacement` as that assistant message's text. The
/// original model output is never persisted or replayed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct OutputMessageReplacedData {
    /// Turn ID this replacement belongs to.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Stable public ID for the assistant message whose streamed text is replaced.
    /// This is the same identifier as the subsequent completed `RuntimeMessage.id`.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "message_550e8400e29b41d4a716446655440000"))]
    pub message_id: MessageId,

    /// Stable ID of the capability that contributed the guardrail
    /// (e.g. `"prompt_canary_guardrail"`).
    pub guardrail_capability_id: String,

    /// Stable ID of the guardrail itself (e.g. `"prompt_canary"`).
    pub guardrail_id: String,

    /// Stable machine-readable reason code (e.g. `"system_prompt_leak"`).
    /// Clients localize their copy from this rather than the human text.
    pub reason_code: String,

    /// Replacement text shown to the user and stored as the assistant message.
    pub replacement: String,
}

// ============================================================================
// Atom Event Data Types
// ============================================================================

//! Payloads for file writes, budgets and voice sessions.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Data for file.written events emitted when files are written to the session filesystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FileWrittenData {
    /// File path within the session filesystem (normalized, e.g. "/reports/summary.md").
    pub path: String,
    /// Operation type (see `FILE_OP_*` constants).
    pub operation: String,
    /// File size in bytes after write.
    pub size_bytes: i64,
    /// Whether this is a new file (true) or an update to an existing file (false).
    pub created: bool,
}

pub const FILE_OP_UPDATE: &str = "update";

// ============================================================================
// Budget event data
// ============================================================================

/// Data for budget lifecycle events (warning, paused, exhausted, resumed).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct BudgetEventData {
    /// Budget that triggered this event.
    pub budget_id: String,
    /// Current remaining balance.
    pub balance: f64,
    /// Budget limit.
    pub limit: f64,
    /// Budget currency (e.g. "usd", "tokens").
    pub currency: String,
    /// Human-readable message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Soft limit threshold (present for warning/paused events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soft_limit: Option<f64>,
}

// ============================================================================
// Voice Event Data Types
// ============================================================================

/// Data for voice.session.started.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VoiceSessionStartedData {
    /// Prefixed voice connection identifier for the started session.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "voice_01933b5a00007000800000000000001")
    )]
    pub voice_connection_id: String,
    /// Provider-side realtime model identifier negotiated for this session.
    #[cfg_attr(feature = "openapi", schema(example = "gpt-realtime"))]
    pub model: String,
    /// Realtime voice preset selected for this session.
    #[cfg_attr(feature = "openapi", schema(example = "alloy"))]
    pub voice: String,
    /// Reasoning effort applied to the realtime model. One of `low`, `medium`, `high`.
    #[cfg_attr(feature = "openapi", schema(example = "medium"))]
    pub reasoning_effort: String,
    /// Transport carrying the audio stream. One of `webrtc`, `sip`, `websocket`.
    #[cfg_attr(feature = "openapi", schema(example = "webrtc"))]
    pub transport: String,
}

/// Data for voice transcript delta/completed events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VoiceTranscriptData {
    /// Prefixed voice connection identifier this transcript belongs to.
    pub voice_connection_id: String,
    /// Provider-specific identifier of the conversation item being transcribed. `None` when not yet assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    /// Provider-specific identifier of the response stream emitting this transcript. `None` for user-side transcripts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    /// Transcript phase: `user_partial`, `user_final`, `assistant_partial`, `assistant_final`. `None` when not yet classified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Newly transcribed text chunk delivered in this event. Empty for "final" events that only mark completion.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub delta: String,
    /// Full transcript accumulated for this item up to and including `delta`.
    pub accumulated: String,
}

/// Data for voice.session.ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VoiceSessionEndedData {
    /// Prefixed voice connection identifier for the ended session.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "voice_01933b5a00007000800000000000001")
    )]
    pub voice_connection_id: String,
    /// Free-text end reason captured from the client or server. `None` when no reason was supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(example = "User hung up after refund confirmed.")
    )]
    pub reason: Option<String>,
    /// Total wall-clock duration of the connection in milliseconds. `None` when the connection
    /// never completed an audio handshake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 184_500_u64))]
    pub duration_ms: Option<u64>,
}

/// Data for voice.session.failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VoiceSessionFailedData {
    /// Prefixed voice connection identifier for the failed session.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "voice_01933b5a00007000800000000000001")
    )]
    pub voice_connection_id: String,
    /// Error message captured at failure. Provider-formatted; not stable for parsing.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "realtime provider closed stream: 1011 internal_error")
    )]
    pub error: String,
}

// ============================================================================
// EventData Enum - Typed event payloads
// ============================================================================

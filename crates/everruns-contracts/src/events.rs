// AG-UI protocol event types for SSE streaming
//
// Runtime types are defined in everruns-core and re-exported here
// for backward compatibility. This file only defines DB/API entity types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// Re-export all AG-UI runtime types from core
pub use everruns_core::ag_ui::{
    AgUiEvent, AgUiMessageRole, CustomEvent, JsonPatchOp, MessagesSnapshotEvent, RunErrorEvent,
    RunFinishedEvent, RunStartedEvent, SnapshotMessage, StateDeltaEvent, StateSnapshotEvent,
    StepFinishedEvent, StepStartedEvent, TextMessageContentEvent, TextMessageEndEvent,
    TextMessageStartEvent, ToolCallArgsEvent, ToolCallEndEvent, ToolCallResultEvent,
    ToolCallStartEvent, EVENT_MESSAGE_ASSISTANT, EVENT_MESSAGE_SYSTEM, EVENT_MESSAGE_USER,
    EVENT_SESSION_ERROR, EVENT_SESSION_FINISHED, EVENT_SESSION_STARTED, EVENT_STATE_DELTA,
    EVENT_STATE_SNAPSHOT, EVENT_TEXT_DELTA, EVENT_TEXT_END, EVENT_TEXT_START, EVENT_TOOL_CALL_ARGS,
    EVENT_TOOL_CALL_END, EVENT_TOOL_CALL_START, EVENT_TOOL_RESULT, HITL_DECISION, HITL_REQUEST,
};

// ============================================
// Event DTOs for REST API (database entities)
// ============================================

/// Event - SSE notification record stored in database
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Event {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sequence: i32,
    pub event_type: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Request to create an event (mainly for internal use)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateEventRequest {
    pub event_type: String,
    pub data: serde_json::Value,
}

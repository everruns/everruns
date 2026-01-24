// AG-UI Protocol Support
//
// This module provides AG-UI protocol compatibility for Everruns.
// AG-UI is an open, lightweight, event-based protocol for AI agent UIs.
// See: https://docs.ag-ui.com
//
// Design decisions:
// - AG-UI is a SECONDARY API; the primary SSE endpoint remains unchanged
// - Events are translated at the SSE layer, not at storage
// - We define our own AG-UI types (not depending on community crate for stability)
// - SCREAMING_SNAKE_CASE serialization for event type field

use crate::api::common::ErrorResponse;
use crate::api::sse::SseStreamConfig;
use crate::auth::{OrgContext, middleware::FromRef};
use crate::services::{EventService, SessionService};
use crate::storage::StorageBackend;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
};
use everruns_core::typed_id::SessionId;
use everruns_core::{Event, EventData};
use futures::{
    StreamExt,
    stream::{self, Stream},
};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc, time::Duration};
use uuid::Uuid;

use super::events::EventsQuery;
use crate::auth::AuthState;

// ============================================================================
// AG-UI Event Types
// ============================================================================

/// Base fields for all AG-UI events
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseEvent {
    /// Event type in SCREAMING_SNAKE_CASE
    #[serde(rename = "type")]
    pub event_type: AgUiEventType,

    /// Optional timestamp (seconds since epoch)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

/// AG-UI event types (SCREAMING_SNAKE_CASE)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgUiEventType {
    // Lifecycle events
    RunStarted,
    RunFinished,
    RunError,

    // Text message events
    TextMessageStart,
    TextMessageContent,
    TextMessageEnd,

    // Tool call events
    ToolCallStart,
    ToolCallArgs,
    ToolCallEnd,
    ToolCallResult,

    // Thinking events (extended thinking/reasoning)
    ThinkingTextMessageStart,
    ThinkingTextMessageContent,
    ThinkingTextMessageEnd,

    // State events (for future use)
    StateSnapshot,
    StateDelta,
    MessagesSnapshot,

    // Step events
    StepStarted,
    StepFinished,

    // Extension events
    Raw,
    Custom,
}

impl AgUiEventType {
    /// Get the event type string for SSE event field
    pub fn as_str(&self) -> &'static str {
        match self {
            AgUiEventType::RunStarted => "RUN_STARTED",
            AgUiEventType::RunFinished => "RUN_FINISHED",
            AgUiEventType::RunError => "RUN_ERROR",
            AgUiEventType::TextMessageStart => "TEXT_MESSAGE_START",
            AgUiEventType::TextMessageContent => "TEXT_MESSAGE_CONTENT",
            AgUiEventType::TextMessageEnd => "TEXT_MESSAGE_END",
            AgUiEventType::ToolCallStart => "TOOL_CALL_START",
            AgUiEventType::ToolCallArgs => "TOOL_CALL_ARGS",
            AgUiEventType::ToolCallEnd => "TOOL_CALL_END",
            AgUiEventType::ToolCallResult => "TOOL_CALL_RESULT",
            AgUiEventType::ThinkingTextMessageStart => "THINKING_TEXT_MESSAGE_START",
            AgUiEventType::ThinkingTextMessageContent => "THINKING_TEXT_MESSAGE_CONTENT",
            AgUiEventType::ThinkingTextMessageEnd => "THINKING_TEXT_MESSAGE_END",
            AgUiEventType::StateSnapshot => "STATE_SNAPSHOT",
            AgUiEventType::StateDelta => "STATE_DELTA",
            AgUiEventType::MessagesSnapshot => "MESSAGES_SNAPSHOT",
            AgUiEventType::StepStarted => "STEP_STARTED",
            AgUiEventType::StepFinished => "STEP_FINISHED",
            AgUiEventType::Raw => "RAW",
            AgUiEventType::Custom => "CUSTOM",
        }
    }
}

/// Message role in AG-UI format
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgUiRole {
    User,
    Assistant,
    System,
    Tool,
}

// ============================================================================
// AG-UI Event Payloads
// ============================================================================

/// RUN_STARTED event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStartedEvent {
    #[serde(rename = "type")]
    pub event_type: AgUiEventType,
    pub thread_id: String,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

/// RUN_FINISHED event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFinishedEvent {
    #[serde(rename = "type")]
    pub event_type: AgUiEventType,
    pub thread_id: String,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

/// RUN_ERROR event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunErrorEvent {
    #[serde(rename = "type")]
    pub event_type: AgUiEventType,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

/// TEXT_MESSAGE_START event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMessageStartEvent {
    #[serde(rename = "type")]
    pub event_type: AgUiEventType,
    pub message_id: String,
    pub role: AgUiRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

/// TEXT_MESSAGE_CONTENT event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMessageContentEvent {
    #[serde(rename = "type")]
    pub event_type: AgUiEventType,
    pub message_id: String,
    pub delta: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

/// TEXT_MESSAGE_END event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMessageEndEvent {
    #[serde(rename = "type")]
    pub event_type: AgUiEventType,
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

/// TOOL_CALL_START event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallStartEvent {
    #[serde(rename = "type")]
    pub event_type: AgUiEventType,
    pub tool_call_id: String,
    pub tool_call_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

/// TOOL_CALL_ARGS event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallArgsEvent {
    #[serde(rename = "type")]
    pub event_type: AgUiEventType,
    pub tool_call_id: String,
    pub delta: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

/// TOOL_CALL_END event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallEndEvent {
    #[serde(rename = "type")]
    pub event_type: AgUiEventType,
    pub tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

/// TOOL_CALL_RESULT event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallResultEvent {
    #[serde(rename = "type")]
    pub event_type: AgUiEventType,
    pub tool_call_id: String,
    pub result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

/// THINKING_TEXT_MESSAGE_START event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingTextMessageStartEvent {
    #[serde(rename = "type")]
    pub event_type: AgUiEventType,
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

/// THINKING_TEXT_MESSAGE_CONTENT event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingTextMessageContentEvent {
    #[serde(rename = "type")]
    pub event_type: AgUiEventType,
    pub message_id: String,
    pub delta: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

/// THINKING_TEXT_MESSAGE_END event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingTextMessageEndEvent {
    #[serde(rename = "type")]
    pub event_type: AgUiEventType,
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

// ============================================================================
// Unified AG-UI Event Enum
// ============================================================================

/// All possible AG-UI events
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum AgUiEvent {
    RunStarted(RunStartedEvent),
    RunFinished(RunFinishedEvent),
    RunError(RunErrorEvent),
    TextMessageStart(TextMessageStartEvent),
    TextMessageContent(TextMessageContentEvent),
    TextMessageEnd(TextMessageEndEvent),
    ToolCallStart(ToolCallStartEvent),
    ToolCallArgs(ToolCallArgsEvent),
    ToolCallEnd(ToolCallEndEvent),
    ToolCallResult(ToolCallResultEvent),
    ThinkingTextMessageStart(ThinkingTextMessageStartEvent),
    ThinkingTextMessageContent(ThinkingTextMessageContentEvent),
    ThinkingTextMessageEnd(ThinkingTextMessageEndEvent),
}

impl AgUiEvent {
    /// Get the event type for SSE event field
    pub fn event_type(&self) -> AgUiEventType {
        match self {
            AgUiEvent::RunStarted(e) => e.event_type,
            AgUiEvent::RunFinished(e) => e.event_type,
            AgUiEvent::RunError(e) => e.event_type,
            AgUiEvent::TextMessageStart(e) => e.event_type,
            AgUiEvent::TextMessageContent(e) => e.event_type,
            AgUiEvent::TextMessageEnd(e) => e.event_type,
            AgUiEvent::ToolCallStart(e) => e.event_type,
            AgUiEvent::ToolCallArgs(e) => e.event_type,
            AgUiEvent::ToolCallEnd(e) => e.event_type,
            AgUiEvent::ToolCallResult(e) => e.event_type,
            AgUiEvent::ThinkingTextMessageStart(e) => e.event_type,
            AgUiEvent::ThinkingTextMessageContent(e) => e.event_type,
            AgUiEvent::ThinkingTextMessageEnd(e) => e.event_type,
        }
    }
}

// ============================================================================
// Translation State
// ============================================================================

/// State for tracking IDs across event translation
#[derive(Debug, Clone, Default)]
pub struct TranslationState {
    /// Current run/turn ID
    pub current_run_id: Option<String>,
    /// Current output message ID
    pub current_message_id: Option<String>,
    /// Current thinking message ID
    pub current_thinking_id: Option<String>,
}

// ============================================================================
// Event Translation
// ============================================================================

/// Translate an Everruns event to AG-UI events
///
/// Returns a vector because some Everruns events map to multiple AG-UI events
/// (e.g., tool.started -> TOOL_CALL_START + TOOL_CALL_ARGS)
pub fn translate_event(event: &Event, state: &mut TranslationState) -> Vec<AgUiEvent> {
    let timestamp =
        Some(event.ts.timestamp() as f64 + event.ts.timestamp_subsec_millis() as f64 / 1000.0);
    let thread_id = event.session_id.to_string();

    match &event.data {
        EventData::TurnStarted(data) => {
            let run_id = data.turn_id.to_string();
            state.current_run_id = Some(run_id.clone());
            vec![AgUiEvent::RunStarted(RunStartedEvent {
                event_type: AgUiEventType::RunStarted,
                thread_id,
                run_id,
                timestamp,
            })]
        }

        EventData::TurnCompleted(data) => {
            let run_id = data.turn_id.to_string();
            state.current_run_id = None;
            state.current_message_id = None;
            vec![AgUiEvent::RunFinished(RunFinishedEvent {
                event_type: AgUiEventType::RunFinished,
                thread_id,
                run_id,
                timestamp,
            })]
        }

        EventData::TurnFailed(data) => {
            state.current_run_id = None;
            state.current_message_id = None;
            vec![AgUiEvent::RunError(RunErrorEvent {
                event_type: AgUiEventType::RunError,
                message: data.error.clone(),
                code: data.error_code.clone(),
                timestamp,
            })]
        }

        EventData::TurnCancelled(data) => {
            state.current_run_id = None;
            state.current_message_id = None;
            vec![AgUiEvent::RunError(RunErrorEvent {
                event_type: AgUiEventType::RunError,
                message: data
                    .reason
                    .clone()
                    .unwrap_or_else(|| "Cancelled".to_string()),
                code: Some("cancelled".to_string()),
                timestamp,
            })]
        }

        EventData::OutputMessageStarted(data) => {
            // Generate a message ID based on turn_id
            let message_id = format!("msg_{}", data.turn_id);
            state.current_message_id = Some(message_id.clone());
            vec![AgUiEvent::TextMessageStart(TextMessageStartEvent {
                event_type: AgUiEventType::TextMessageStart,
                message_id,
                role: AgUiRole::Assistant,
                timestamp,
            })]
        }

        EventData::OutputMessageDelta(data) => {
            let message_id = state
                .current_message_id
                .clone()
                .unwrap_or_else(|| format!("msg_{}", data.turn_id));
            vec![AgUiEvent::TextMessageContent(TextMessageContentEvent {
                event_type: AgUiEventType::TextMessageContent,
                message_id,
                delta: data.delta.clone(),
                timestamp,
            })]
        }

        EventData::OutputMessageCompleted(_data) => {
            if let Some(message_id) = state.current_message_id.take() {
                vec![AgUiEvent::TextMessageEnd(TextMessageEndEvent {
                    event_type: AgUiEventType::TextMessageEnd,
                    message_id,
                    timestamp,
                })]
            } else {
                vec![]
            }
        }

        EventData::ToolStarted(data) => {
            let tool_call_id = data.tool_call.id.clone();
            let tool_name = data.tool_call.name.clone();
            let args_json = serde_json::to_string(&data.tool_call.arguments)
                .unwrap_or_else(|_| "{}".to_string());

            // Emit both TOOL_CALL_START and TOOL_CALL_ARGS (args sent immediately)
            vec![
                AgUiEvent::ToolCallStart(ToolCallStartEvent {
                    event_type: AgUiEventType::ToolCallStart,
                    tool_call_id: tool_call_id.clone(),
                    tool_call_name: tool_name,
                    parent_message_id: state.current_message_id.clone(),
                    timestamp,
                }),
                AgUiEvent::ToolCallArgs(ToolCallArgsEvent {
                    event_type: AgUiEventType::ToolCallArgs,
                    tool_call_id: tool_call_id.clone(),
                    delta: args_json,
                    timestamp,
                }),
                AgUiEvent::ToolCallEnd(ToolCallEndEvent {
                    event_type: AgUiEventType::ToolCallEnd,
                    tool_call_id,
                    timestamp,
                }),
            ]
        }

        EventData::ToolCompleted(data) => {
            let result = if data.success {
                // Convert result to string
                data.result
                    .as_ref()
                    .map(|parts| {
                        parts
                            .iter()
                            .map(|p| match p {
                                everruns_core::message::ContentPart::Text(text_part) => {
                                    text_part.text.clone()
                                }
                                _ => format!("{:?}", p),
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default()
            } else {
                data.error
                    .clone()
                    .unwrap_or_else(|| "Unknown error".to_string())
            };

            vec![AgUiEvent::ToolCallResult(ToolCallResultEvent {
                event_type: AgUiEventType::ToolCallResult,
                tool_call_id: data.tool_call_id.clone(),
                result,
                timestamp,
            })]
        }

        EventData::ReasonThinkingStarted(data) => {
            let message_id = format!("thinking_{}", data.turn_id);
            state.current_thinking_id = Some(message_id.clone());
            vec![AgUiEvent::ThinkingTextMessageStart(
                ThinkingTextMessageStartEvent {
                    event_type: AgUiEventType::ThinkingTextMessageStart,
                    message_id,
                    timestamp,
                },
            )]
        }

        EventData::ReasonThinkingDelta(data) => {
            let message_id = state
                .current_thinking_id
                .clone()
                .unwrap_or_else(|| format!("thinking_{}", data.turn_id));
            vec![AgUiEvent::ThinkingTextMessageContent(
                ThinkingTextMessageContentEvent {
                    event_type: AgUiEventType::ThinkingTextMessageContent,
                    message_id,
                    delta: data.delta.clone(),
                    timestamp,
                },
            )]
        }

        EventData::ReasonThinkingCompleted(_data) => {
            if let Some(message_id) = state.current_thinking_id.take() {
                vec![AgUiEvent::ThinkingTextMessageEnd(
                    ThinkingTextMessageEndEvent {
                        event_type: AgUiEventType::ThinkingTextMessageEnd,
                        message_id,
                        timestamp,
                    },
                )]
            } else {
                vec![]
            }
        }

        // Events not mapped to AG-UI (internal/observability)
        EventData::InputMessage(_)
        | EventData::ReasonStarted(_)
        | EventData::ReasonCompleted(_)
        | EventData::ActStarted(_)
        | EventData::ActCompleted(_)
        | EventData::LlmGeneration(_)
        | EventData::SessionStarted(_)
        | EventData::SessionActivated(_)
        | EventData::SessionIdled(_)
        | EventData::Unsupported { .. } => vec![],
    }
}

// ============================================================================
// App State and Routes
// ============================================================================

/// App state for AG-UI routes
#[derive(Clone)]
pub struct AppState {
    pub session_service: Arc<SessionService>,
    pub event_service: Arc<EventService>,
    pub auth: AuthState,
}

impl AppState {
    /// Create app state from storage backend
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            event_service: Arc::new(EventService::new(db)),
            auth,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Create AG-UI routes
pub fn routes(state: AppState) -> axum::Router {
    use axum::routing::get;
    axum::Router::new()
        .route(
            "/v1/orgs/:org/agents/:agent_id/sessions/:session_id/ag-ui/sse",
            get(stream_ag_ui_sse),
        )
        .with_state(state)
}

// ============================================================================
// SSE Endpoint Handler
// ============================================================================

/// GET /v1/orgs/{org}/agents/{agent_id}/sessions/{session_id}/ag-ui/sse
///
/// Stream session events in AG-UI protocol format.
/// This is a SECONDARY API - the primary SSE endpoint remains at /sse
#[utoipa::path(
    get,
    path = "/v1/orgs/{org}/agents/{agent_id}/sessions/{session_id}/ag-ui/sse",
    params(
        ("org" = String, Path, description = "Organization public ID"),
        ("agent_id" = String, Path, description = "Agent ID (prefixed, e.g., agt_...)"),
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)"),
        EventsQuery
    ),
    responses(
        (status = 200, description = "AG-UI event stream", content_type = "text/event-stream"),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "ag-ui"
)]
pub async fn stream_ag_ui_sse(
    org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, _agent_id, session_id)): Path<(String, String, String)>,
    Query(query): Query<EventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, (StatusCode, Json<ErrorResponse>)>
{
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid session ID: {}", e),
            }),
        )
    })?;

    // Verify session exists
    let _session = state
        .session_service
        .get(org.org_id, &org.public_id, session_id.uuid())
        .await
        .map_err(|e| {
            tracing::error!("Failed to get session: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Session not found".to_string(),
                }),
            )
        })?;

    let session_uuid = session_id.uuid();
    tracing::info!(session_id = %session_uuid, since_id = ?query.since_id, "Starting AG-UI event stream");

    let event_service = state.event_service.clone();
    let initial_since_id = query.since_id;

    // Use realtime config for session events
    let config = SseStreamConfig::realtime();

    // Stream state
    #[derive(Clone)]
    struct StreamState {
        last_id: Option<Uuid>,
        backoff_ms: u64,
        sent_connected: bool,
        config: SseStreamConfig,
        translation_state: TranslationState,
    }

    let initial_state = StreamState {
        last_id: initial_since_id,
        backoff_ms: config.min_backoff_ms,
        sent_connected: false,
        config,
        translation_state: TranslationState::default(),
    };

    // Create stream that translates events to AG-UI format
    let stream = stream::unfold(initial_state, move |mut state| {
        let event_service = event_service.clone();
        async move {
            // Send initial "connected" event
            if !state.sent_connected {
                tracing::debug!(session_id = %session_uuid, "AG-UI SSE: sending connected event");
                let connected_event = Ok(SseEvent::default()
                    .event("connected")
                    .data(r#"{"status":"connected"}"#));
                state.sent_connected = true;
                return Some((stream::iter(vec![connected_event]), state));
            }

            // Fetch events since last ID
            match event_service
                .list(session_uuid, None, state.last_id, &[])
                .await
            {
                Ok(events) if !events.is_empty() => {
                    let new_last_id = Some(events.last().unwrap().id.uuid());

                    tracing::debug!(
                        session_id = %session_uuid,
                        event_count = events.len(),
                        "AG-UI SSE: translating events"
                    );

                    // Translate events to AG-UI format
                    let mut sse_events: Vec<Result<SseEvent, Infallible>> = Vec::new();
                    for event in events {
                        let ag_ui_events = translate_event(&event, &mut state.translation_state);
                        for ag_ui_event in ag_ui_events {
                            let event_type = ag_ui_event.event_type().as_str();
                            let json = serde_json::to_string(&ag_ui_event)
                                .unwrap_or_else(|_| "{}".to_string());
                            sse_events.push(Ok(SseEvent::default()
                                .event(event_type)
                                .data(json)
                                .id(event.id.to_string())));
                        }
                    }

                    // Reset backoff on new events
                    state.last_id = new_last_id;
                    state.backoff_ms = state.config.min_backoff_ms;
                    Some((stream::iter(sse_events), state))
                }
                Ok(_) => {
                    // No new events, wait with exponential backoff
                    tokio::time::sleep(Duration::from_millis(state.backoff_ms)).await;
                    state.backoff_ms = state.config.next_backoff(state.backoff_ms);
                    Some((stream::iter(vec![]), state))
                }
                Err(e) => {
                    tracing::error!("Failed to fetch events for AG-UI: {}", e);
                    None
                }
            }
        }
    })
    .flatten();

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use everruns_core::events::*;
    use everruns_core::typed_id::{EventId, MessageId, TurnId};

    fn make_test_event(data: EventData) -> Event {
        Event {
            id: EventId::new(),
            event_type: data.event_type().to_string(),
            ts: Utc::now(),
            session_id: "session_00000000000000000000000000000001".parse().unwrap(),
            context: EventContext::default(),
            data,
            metadata: None,
            tags: None,
            sequence: None,
        }
    }

    #[test]
    fn test_translate_turn_started_to_run_started() {
        let mut state = TranslationState::default();
        let turn_id: TurnId = "turn_00000000000000000000000000000001".parse().unwrap();
        let message_id: MessageId = "message_00000000000000000000000000000001".parse().unwrap();

        let event = make_test_event(EventData::TurnStarted(TurnStartedData {
            turn_id,
            input_message_id: message_id,
            input_content: None,
        }));

        let ag_ui_events = translate_event(&event, &mut state);
        assert_eq!(ag_ui_events.len(), 1);

        match &ag_ui_events[0] {
            AgUiEvent::RunStarted(e) => {
                assert_eq!(e.event_type, AgUiEventType::RunStarted);
                assert!(e.run_id.contains("turn_"));
            }
            _ => panic!("Expected RunStarted event"),
        }
    }

    #[test]
    fn test_translate_turn_completed_to_run_finished() {
        let mut state = TranslationState::default();
        let turn_id: TurnId = "turn_00000000000000000000000000000001".parse().unwrap();

        let event = make_test_event(EventData::TurnCompleted(TurnCompletedData {
            turn_id,
            iterations: 1,
            duration_ms: Some(100),
            usage: None,
            input_content: None,
        }));

        let ag_ui_events = translate_event(&event, &mut state);
        assert_eq!(ag_ui_events.len(), 1);

        match &ag_ui_events[0] {
            AgUiEvent::RunFinished(e) => {
                assert_eq!(e.event_type, AgUiEventType::RunFinished);
            }
            _ => panic!("Expected RunFinished event"),
        }
    }

    #[test]
    fn test_translate_turn_failed_to_run_error() {
        let mut state = TranslationState::default();
        let turn_id: TurnId = "turn_00000000000000000000000000000001".parse().unwrap();

        let event = make_test_event(EventData::TurnFailed(TurnFailedData {
            turn_id,
            error: "Something went wrong".to_string(),
            error_code: Some("llm_error".to_string()),
        }));

        let ag_ui_events = translate_event(&event, &mut state);
        assert_eq!(ag_ui_events.len(), 1);

        match &ag_ui_events[0] {
            AgUiEvent::RunError(e) => {
                assert_eq!(e.event_type, AgUiEventType::RunError);
                assert_eq!(e.message, "Something went wrong");
                assert_eq!(e.code, Some("llm_error".to_string()));
            }
            _ => panic!("Expected RunError event"),
        }
    }

    #[test]
    fn test_translate_output_message_delta_to_text_message_content() {
        let mut state = TranslationState {
            current_message_id: Some("msg_test".to_string()),
            ..Default::default()
        };
        let turn_id: TurnId = "turn_00000000000000000000000000000001".parse().unwrap();

        let event = make_test_event(EventData::OutputMessageDelta(OutputMessageDeltaData {
            turn_id,
            delta: "Hello ".to_string(),
            accumulated: "Hello ".to_string(),
        }));

        let ag_ui_events = translate_event(&event, &mut state);
        assert_eq!(ag_ui_events.len(), 1);

        match &ag_ui_events[0] {
            AgUiEvent::TextMessageContent(e) => {
                assert_eq!(e.event_type, AgUiEventType::TextMessageContent);
                assert_eq!(e.delta, "Hello ");
                assert_eq!(e.message_id, "msg_test");
            }
            _ => panic!("Expected TextMessageContent event"),
        }
    }

    #[test]
    fn test_translate_tool_started_emits_start_args_end() {
        let mut state = TranslationState::default();
        let tool_call = everruns_core::tool_types::ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "Tokyo"}),
        };

        let event = make_test_event(EventData::ToolStarted(ToolStartedData { tool_call }));

        let ag_ui_events = translate_event(&event, &mut state);
        assert_eq!(ag_ui_events.len(), 3);

        // Should emit TOOL_CALL_START, TOOL_CALL_ARGS, TOOL_CALL_END
        assert!(matches!(&ag_ui_events[0], AgUiEvent::ToolCallStart(_)));
        assert!(matches!(&ag_ui_events[1], AgUiEvent::ToolCallArgs(_)));
        assert!(matches!(&ag_ui_events[2], AgUiEvent::ToolCallEnd(_)));

        if let AgUiEvent::ToolCallArgs(e) = &ag_ui_events[1] {
            assert!(e.delta.contains("Tokyo"));
        }
    }

    #[test]
    fn test_translation_state_tracks_ids() {
        let mut state = TranslationState::default();
        let turn_id: TurnId = "turn_00000000000000000000000000000001".parse().unwrap();
        let message_id: MessageId = "message_00000000000000000000000000000001".parse().unwrap();

        // Start turn
        let event = make_test_event(EventData::TurnStarted(TurnStartedData {
            turn_id,
            input_message_id: message_id,
            input_content: None,
        }));
        translate_event(&event, &mut state);
        assert!(state.current_run_id.is_some());

        // Start output message
        let event = make_test_event(EventData::OutputMessageStarted(OutputMessageStartedData {
            turn_id,
            model: Some("gpt-4".to_string()),
        }));
        translate_event(&event, &mut state);
        assert!(state.current_message_id.is_some());

        // Complete turn - should clear state
        let event = make_test_event(EventData::TurnCompleted(TurnCompletedData {
            turn_id,
            iterations: 1,
            duration_ms: None,
            usage: None,
            input_content: None,
        }));
        translate_event(&event, &mut state);
        assert!(state.current_run_id.is_none());
        assert!(state.current_message_id.is_none());
    }

    #[test]
    fn test_event_serialization_screaming_snake_case() {
        let event = RunStartedEvent {
            event_type: AgUiEventType::RunStarted,
            thread_id: "thread_1".to_string(),
            run_id: "run_1".to_string(),
            timestamp: Some(1234567890.123),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"RUN_STARTED""#));
        assert!(json.contains(r#""threadId""#));
        assert!(json.contains(r#""runId""#));
    }

    #[test]
    fn test_internal_events_not_mapped() {
        let mut state = TranslationState::default();

        // reason.started should not produce AG-UI events
        let event = make_test_event(EventData::ReasonStarted(ReasonStartedData {
            agent_id: "agent_00000000000000000000000000000001".parse().unwrap(),
            metadata: None,
        }));
        let ag_ui_events = translate_event(&event, &mut state);
        assert!(ag_ui_events.is_empty());

        // session.activated should not produce AG-UI events
        let turn_id: TurnId = "turn_00000000000000000000000000000001".parse().unwrap();
        let message_id: MessageId = "message_00000000000000000000000000000001".parse().unwrap();
        let event = make_test_event(EventData::SessionActivated(SessionActivatedData {
            turn_id,
            input_message_id: message_id,
        }));
        let ag_ui_events = translate_event(&event, &mut state);
        assert!(ag_ui_events.is_empty());
    }
}

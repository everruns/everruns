// Message HTTP routes and API contracts
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)
//
// BREAKING CHANGE: Simplified message roles to just `user` and `agent`.
// - Tool results are conveyed via `tool.completed` events
// - System messages are internal and not exposed via API
//
// ContentPart and InputContentPart are defined in everruns-core.
// We re-export them here with ToSchema for OpenAPI documentation.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::messages::{
    CreateMessage, ExportSessionMessages, ListMessages, SessionExportFormat,
};
use crate::middleware::RequestId;
use crate::storage::StorageBackend;
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use everruns_provider::typed_id::{MessageId, SessionId, SessionParticipantId};

use super::common::{ApiResult, ErrorResponse, ListResponse, impl_auth_state};
use everruns_scale::RunController;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use utoipa::ToSchema;

use everruns_core::Caller;

use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;

// Re-export core types with ToSchema for OpenAPI
#[allow(unused_imports)]
pub use everruns_core::{
    ContentPart, ContentType, Controls, ImageContentPart, ImageFileContentPart, InputContentPart,
    ReasoningConfig, TextContentPart, ToolCallContentPart, ToolResultContentPart,
};

// ============================================
// Message API Contracts
// ============================================

/// Message role (API layer)
///
/// Simplified to only user and agent messages.
/// Tool results are conveyed via `tool.completed` events.
/// System messages are internal and not exposed via API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// User message (input from the user)
    User,
    /// Agent message (response from the AI agent)
    Agent,
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageRole::User => write!(f, "user"),
            MessageRole::Agent => write!(f, "agent"),
        }
    }
}

impl From<&str> for MessageRole {
    fn from(s: &str) -> Self {
        match s {
            // Map both "agent" and legacy "assistant" to Agent role
            "agent" | "assistant" => MessageRole::Agent,
            _ => MessageRole::User,
        }
    }
}

/// Message - primary conversation data (API response)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Message {
    /// Unique message ID (format: message_{32-hex})
    #[schema(value_type = String, example = "message_01933b5a00007000800000000000001")]
    pub id: MessageId,
    /// Session ID this message belongs to (format: session_{32-hex})
    #[schema(value_type = String, example = "session_01933b5a00007000800000000000001")]
    pub session_id: SessionId,
    pub sequence: i32,
    pub role: MessageRole,
    /// Array of content parts
    pub content: Vec<ContentPart>,
    /// Runtime controls (model, reasoning, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<Controls>,
    /// Message-level metadata (locale, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    /// External actor identity (for messages from external channels like Slack)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_actor: Option<everruns_core::ExternalActor>,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
}

/// Input message for creating a user message
///
/// Only user messages can be created via the API.
/// Agent messages are created internally by the workflow.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(example = json!({"role": "user", "content": [{"type": "text", "text": "Why is the build failing on main?"}]}))]
pub struct InputMessage {
    /// Message role (always "user" for API-created messages)
    #[serde(default = "default_user_role")]
    pub role: MessageRole,
    /// Array of content parts (text and image only)
    pub content: Vec<InputContentPart>,
}

fn default_user_role() -> MessageRole {
    MessageRole::User
}

/// Request to create a message
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateMessageRequest {
    /// The message to create. Example shape is defined on `InputMessage`.
    pub message: InputMessage,
    /// Optional active agent participant to address for this turn. When omitted,
    /// the session host remains the responder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(
        value_type = Option<String>,
        example = "part_01933b5a00007000800000000000001"
    )]
    pub addressed_participant_id: Option<SessionParticipantId>,
    /// Runtime controls (model, reasoning, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<Controls>,
    /// Request-level metadata. Arbitrary key/value pairs persisted with the message
    /// for downstream filtering and analytics. Not interpreted by the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = json!({"source": "slack", "thread_ts": "1715000000.123456"}))]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    /// Tags for the message. Free-form labels used for grouping and filtering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = json!(["bug-report", "from-slack"]))]
    pub tags: Option<Vec<String>>,
    /// External actor identity (for messages from external channels like Slack)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_actor: Option<everruns_core::ExternalActor>,
}

#[cfg(test)]
impl CreateMessageRequest {
    /// Create a user message with text (for tests)
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            message: InputMessage {
                role: MessageRole::User,
                content: vec![InputContentPart::text(text)],
            },
            addressed_participant_id: None,
            controls: None,
            metadata: None,
            tags: None,
            external_actor: None,
        }
    }
}

// ============================================
// App State and Routes
// ============================================

/// App state for messages routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub auth: AuthState,
    /// Response-size cap for the ATIF session export, in bytes. Production
    /// always uses `crate::atif::ATIF_EXPORT_MAX_BYTES`; tests shrink it via
    /// `with_atif_export_max_bytes` to exercise the 413 path cheaply.
    pub atif_export_max_bytes: usize,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        runner: Arc<dyn RunController>,
        auth: AuthState,
        notifications_enabled: bool,
        event_delivery: crate::event_delivery::EventDelivery,
    ) -> Self {
        Self {
            db: db.clone(),
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(
                db,
                runner,
                notifications_enabled,
                event_delivery,
            )),
            auth,
            atif_export_max_bytes: crate::atif::ATIF_EXPORT_MAX_BYTES,
        }
    }

    /// Test-only override of the ATIF export size cap (kept small so 413
    /// coverage does not need to allocate a 50 MiB document).
    pub fn with_atif_export_max_bytes(mut self, max_bytes: usize) -> Self {
        self.atif_export_max_bytes = max_bytes;
        self
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_session_service(self.session_service.clone())
        .with_message_service(self.message_service.clone())
    }
}

impl_auth_state!(AppState);

/// Create message routes (nested under sessions)
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/sessions/{session_id}/messages",
            post(create_message).get(list_messages),
        )
        .route(
            "/v1/sessions/{session_id}/export",
            get(export_session_jsonl),
        )
        .with_state(state)
}

// ============================================
// HTTP Handlers
// ============================================

/// POST /v1/sessions/{session_id}/messages - Create message (user message triggers workflow)
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/messages",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)")
    ),
    request_body = CreateMessageRequest,
    responses(
        (status = 201, description = "Message created successfully", body = Message),
        (status = 400, description = "Invalid ID format"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "messages"
)]
pub async fn create_message(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    req_id: Option<Extension<RequestId>>,
    Json(req): Json<CreateMessageRequest>,
) -> Result<(StatusCode, Json<Message>), (StatusCode, Json<ErrorResponse>)> {
    let request_id = req_id.map(|Extension(r)| r.0);
    let message = CreateMessage {
        session_id,
        message: req.message,
        addressed_participant_id: req.addressed_participant_id,
        controls: req.controls,
        metadata: req.metadata,
        tags: req.tags,
        external_actor: req.external_actor,
        request_id,
    }
    .run(&state.ctx(&org))
    .await?;

    Ok((StatusCode::CREATED, Json(message)))
}

/// GET /v1/sessions/{session_id}/messages - List messages (PRIMARY data)
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/messages",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)")
    ),
    responses(
        (status = 200, description = "List of messages", body = ListResponse<Message>),
        (status = 400, description = "Invalid ID format"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "messages"
)]
pub async fn list_messages(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<ListResponse<Message>> {
    let messages = ListMessages {
        session_id,
        limit: None,
    }
    .run(&state.ctx(&org))
    .await?;

    Ok(Json(ListResponse::new(messages)))
}

/// Query parameters for session export.
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
pub struct ExportSessionQuery {
    /// Output format: `jsonl` (default) or `atif`.
    #[serde(default)]
    pub format: SessionExportFormat,
    /// ATIF only. When `true`, return the session as a chain of byte-bounded
    /// segments (each a standalone ATIF-v1.7 document under the size cap)
    /// instead of one document. A segment with more steps remaining carries a
    /// root `continued_trajectory_ref` URL embedding the next `cursor`; the
    /// final/only segment omits it. Ignored for `jsonl`.
    #[serde(default)]
    pub segmented: bool,
    /// ATIF segmented export only: opaque continuation cursor from the previous
    /// segment's `continued_trajectory_ref`. Omit for the first segment. A
    /// malformed or foreign cursor is rejected with 400.
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Export session messages as a JSONL file (default) or as an ATIF trajectory
///
/// Default (`format=jsonl`): all materialized messages (user, agent) as
/// newline-delimited JSON, one complete JSON object per line; delta events are
/// excluded. `format=atif` returns a single ATIF-v1.7 trajectory JSON document
/// folded from the session's event log (see `knowledge/evaluation/atif-adoption.md`); image
/// content parts are exported as ATIF multimodal ContentParts. When an image
/// cannot be materialized (an inline image with neither a URL nor bytes) it is
/// flattened to an `"[image]"` marker and the response carries an
/// `X-Atif-Images-Omitted` header with that count (usually 0 and absent).
/// Documents over the 50 MiB `ATIF_EXPORT_MAX_BYTES` cap are rejected with 413.
/// The response includes `Content-Disposition: attachment` for browser
/// download.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/export",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)"),
        ("format" = Option<String>, Query, description = "Output format: jsonl (default) or atif"),
        ("segmented" = Option<bool>, Query, description = "ATIF only: return byte-bounded segments linked by continued_trajectory_ref instead of one document"),
        ("cursor" = Option<String>, Query, description = "ATIF segmented export: opaque continuation cursor from the previous segment")
    ),
    responses(
        (status = 200, description = "JSONL file with one message per line, or one ATIF trajectory JSON document (images export as multimodal ContentParts; X-Atif-Images-Omitted header only when an image could not be materialized). With segmented=true, one ATIF segment linked forward by continued_trajectory_ref.", content_type = "application/x-ndjson"),
        (status = 400, description = "Invalid ID format, or malformed/foreign segmented-export cursor"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Session not found"),
        (status = 413, description = "ATIF document exceeds the 50 MiB export cap; retry with segmented=true for a recoverable chunked export", body = ErrorResponse),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn export_session_jsonl(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<ExportSessionQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorResponse>)> {
    // Opt-in segmented ATIF export: a recoverable, byte-bounded chain of
    // standalone ATIF documents for sessions that would 413 as one document.
    // Each segment is bounded to `atif_export_max_bytes` by the builder, so this
    // path never 413s (a single giant step is returned alone; see atif.rs).
    if query.format == SessionExportFormat::Atif && query.segmented {
        return segmented_atif_response(&state, &org, session_id, query.cursor).await;
    }

    let export = ExportSessionMessages {
        session_id: session_id.clone(),
        format: query.format,
    }
    .run(&state.ctx(&org))
    .await?;
    // THREAT[TM-DOS-026]: the ATIF export is a single synchronous JSON
    // document; cap the response body so a huge event log cannot produce an
    // unbounded response. See `crate::atif::ATIF_EXPORT_MAX_BYTES`.
    if query.format == SessionExportFormat::Atif && export.body.len() > state.atif_export_max_bytes
    {
        return Err(ErrorResponse::new(format!(
            "ATIF export for session {} is {} bytes, over the {}-byte limit; retry with &segmented=true to export it as a chain of bounded segments linked by continued_trajectory_ref",
            session_id,
            export.body.len(),
            state.atif_export_max_bytes,
        ))
        .with_code("atif_export_too_large")
        .into_response(StatusCode::PAYLOAD_TOO_LARGE));
    }
    let (content_type, filename) = match query.format {
        SessionExportFormat::Jsonl => ("application/x-ndjson", format!("{}.jsonl", session_id)),
        SessionExportFormat::Atif => ("application/json", format!("{}.atif.json", session_id)),
    };
    // Lossiness signal: set only when image parts were flattened to markers,
    // so clients can tell a lossy ATIF export from a complete one without
    // parsing the body.
    let mut lossiness_header = Vec::new();
    if export.atif_images_omitted > 0 {
        lossiness_header.push((
            axum::http::HeaderName::from_static("x-atif-images-omitted"),
            export.atif_images_omitted.to_string(),
        ));
    }
    Ok((
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, content_type.to_string()),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            ),
        ],
        axum::response::AppendHeaders(lossiness_header),
        export.body,
    )
        .into_response())
}

/// Serve one segment of a segmented ATIF export. The session is resolved
/// org-scoped from the path; the opaque `cursor` only selects a step offset
/// within that session (a malformed or foreign cursor → 400). Each segment is
/// bounded to the export size cap by the builder, so no 413 guard is needed.
async fn segmented_atif_response(
    state: &AppState,
    org: &ResolvedOrg,
    session_id: String,
    cursor: Option<String>,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorResponse>)> {
    let ctx = state.ctx(org);
    let link_base = format!("/v1/sessions/{session_id}/export");
    let segment = crate::domains::messages::export_session_segment(
        &ctx,
        &session_id,
        cursor.as_deref(),
        state.atif_export_max_bytes,
        &link_base,
    )
    .await?;

    // Per-segment out-of-band signals so a client can walk the chain and detect
    // lossiness without parsing each body: the images-omitted count for THIS
    // segment, whether more segments follow, and the next opaque cursor. The
    // authoritative continuation link is `continued_trajectory_ref` in the body.
    let mut extra_headers: Vec<(axum::http::HeaderName, String)> = vec![(
        axum::http::HeaderName::from_static("x-atif-segment-index"),
        segment.segment_index.to_string(),
    )];
    if segment.images_omitted > 0 {
        extra_headers.push((
            axum::http::HeaderName::from_static("x-atif-images-omitted"),
            segment.images_omitted.to_string(),
        ));
    }
    if let Some(next) = &segment.next_cursor {
        extra_headers.push((
            axum::http::HeaderName::from_static("x-atif-next-cursor"),
            next.clone(),
        ));
    }
    let filename = format!("{}.atif.seg{}.json", session_id, segment.segment_index);

    Ok((
        StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            ),
        ],
        axum::response::AppendHeaders(extra_headers),
        segment.body,
    )
        .into_response())
}

// ============================================
// Tests
// ============================================

#[cfg(test)]
mod tests {
    use super::*;

    // Trivial derive-only serde round-trips removed; covered by the derive + handler tests.

    #[test]
    fn test_create_message_request_user() {
        let req = CreateMessageRequest::user("Hello, how are you?");
        assert_eq!(req.message.content.len(), 1);
        assert_eq!(
            req.message.content[0].as_text(),
            Some("Hello, how are you?")
        );
    }

    #[test]
    fn test_message_role_display() {
        assert_eq!(MessageRole::User.to_string(), "user");
        assert_eq!(MessageRole::Agent.to_string(), "agent");
    }

    #[test]
    fn test_message_role_from_str() {
        assert_eq!(MessageRole::from("user"), MessageRole::User);
        assert_eq!(MessageRole::from("agent"), MessageRole::Agent);
        // Legacy "assistant" maps to Agent
        assert_eq!(MessageRole::from("assistant"), MessageRole::Agent);
        // Unknown roles default to User
        assert_eq!(MessageRole::from("unknown"), MessageRole::User);
    }
}

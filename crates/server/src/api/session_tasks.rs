// Session task routes.
//
// Exposes the session task registry — background work owned by a session
// (subagents, external agent runs, background tools). See
// specs/session-tasks.md.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::session_tasks::{
    CancelSessionTask, GetSessionTask, ListSessionTasks, PostSessionTaskMessage, SessionTaskDetail,
};
use crate::services::EventService;
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use everruns_core::session_task::{TaskMessage, TaskMessagePart};
use everruns_core::{Caller, SessionTask};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::{ApiResult, impl_auth_state};

/// App state for session task routes.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
    pub event_service: Arc<EventService>,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState, event_service: Arc<EventService>) -> Self {
        Self {
            db,
            auth,
            event_service,
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        let mut ctx = Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        );
        ctx.event_service = Some(self.event_service.clone());
        ctx
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/sessions/{session_id}/tasks", get(list_tasks))
        .route("/v1/sessions/{session_id}/tasks/{task_id}", get(get_task))
        .route(
            "/v1/sessions/{session_id}/tasks/{task_id}/messages",
            post(post_task_message),
        )
        .route(
            "/v1/sessions/{session_id}/tasks/{task_id}/cancel",
            post(cancel_task),
        )
        .with_state(state)
}

#[derive(Debug, Default, Deserialize)]
pub struct ListTasksQuery {
    pub state: Option<String>,
    pub kind: Option<String>,
}

/// List background tasks owned by a session.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/tasks",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("state" = Option<String>, Query, description = "Filter by state"),
        ("kind" = Option<String>, Query, description = "Filter by kind"),
    ),
    responses(
        (status = 200, description = "Session tasks", body = Vec<SessionTask>),
        (status = 404, description = "Session not found"),
    ),
    tag = "session-tasks"
)]
pub async fn list_tasks(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<ListTasksQuery>,
) -> ApiResult<Vec<SessionTask>> {
    Ok(Json(
        ListSessionTasks {
            session_id,
            state: query.state,
            kind: query.kind,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

#[derive(Debug, Default, Deserialize)]
pub struct GetTaskQuery {
    pub after_id: Option<String>,
    pub limit: Option<u32>,
}

/// Get one session task with its recent message thread.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/tasks/{task_id}",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("task_id" = String, Path, description = "Task ID"),
        ("after_id" = Option<String>, Query, description = "Return only messages after this message ID (exclusive cursor)"),
        ("limit" = Option<u32>, Query, description = "Max messages to return (default 50, max 500)"),
    ),
    responses(
        (status = 200, description = "Session task detail", body = SessionTaskDetail),
        (status = 404, description = "Session or task not found"),
    ),
    tag = "session-tasks"
)]
pub async fn get_task(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, task_id)): Path<(String, String)>,
    Query(query): Query<GetTaskQuery>,
) -> ApiResult<SessionTaskDetail> {
    Ok(Json(
        GetSessionTask {
            session_id,
            task_id,
            after_id: query.after_id,
            limit: query.limit,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

/// Request body for posting an inbound task message.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PostTaskMessageBody {
    /// Plain-text message (alternative to `content`).
    #[serde(default)]
    pub text: Option<String>,
    /// Structured message parts (alternative to `text`).
    #[serde(default)]
    pub content: Option<Vec<TaskMessagePart>>,
    /// Input request ID this message answers, when applicable.
    #[serde(default)]
    pub in_reply_to: Option<String>,
}

/// Send an inbound message to a session task.
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/tasks/{task_id}/messages",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("task_id" = String, Path, description = "Task ID"),
    ),
    request_body = PostTaskMessageBody,
    responses(
        (status = 200, description = "Recorded task message", body = TaskMessage),
        (status = 400, description = "Empty message"),
        (status = 404, description = "Session or task not found"),
    ),
    tag = "session-tasks"
)]
pub async fn post_task_message(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, task_id)): Path<(String, String)>,
    Json(body): Json<PostTaskMessageBody>,
) -> ApiResult<TaskMessage> {
    Ok(Json(
        PostSessionTaskMessage {
            session_id,
            task_id,
            text: body.text,
            content: body.content,
            in_reply_to: body.in_reply_to,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

/// Request cooperative cancellation of a session task.
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/tasks/{task_id}/cancel",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("task_id" = String, Path, description = "Task ID"),
    ),
    responses(
        (status = 200, description = "Task with cancel intent recorded", body = SessionTask),
        (status = 404, description = "Session or task not found"),
    ),
    tag = "session-tasks"
)]
pub async fn cancel_task(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, task_id)): Path<(String, String)>,
) -> ApiResult<SessionTask> {
    Ok(Json(
        CancelSessionTask {
            session_id,
            task_id,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

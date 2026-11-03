//! `GET /v1/sessions/{session_id}/run-requests`: what the other agents in a
//! run wait on the person for, read on the run's first session.
//!
//! Spec: `knowledge/runtime-resources/agents-using-agents.md`. The cards are
//! answered on the first session (`tool-approvals`, `question-answers`), which
//! routes each answer to the asking agent's session.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use everruns_contracts::typed_id::SessionId;
use everruns_core::Caller;

use super::common::{ApiOptionExt, ApiResult, ApiResultExt, ErrorResponse};
use super::tool_results::AppState;
use crate::domains::sessions::run_requests::{
    AnswerTargetError, RunRequestsResponse, answer_target, run_requests, waiting_agents,
};

pub fn routes() -> axum::Router<AppState> {
    axum::Router::new().route(
        "/v1/sessions/{session_id}/run-requests",
        get(get_run_requests),
    )
}

/// GET /v1/sessions/{session_id}/run-requests
///
/// Questions and approvals that the agents this session handed work to wait
/// on. Empty unless the session is a run's first session.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/run-requests",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 200, description = "Requests from the other agents in the run", body = RunRequestsResponse),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_run_requests(
    org: crate::auth::ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<RunRequestsResponse> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid session ID: {e}"))
            .into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let session = state
        .session_service
        .get(&caller, session_id.uuid(), None)
        .await
        .log_internal_error_json("get session")?
        .ok_or_not_found_json("Session")?;
    if !crate::domains::sessions::platform_chat_owner_matches_session(&state.db, &caller, &session)
        .await
        .log_internal_error_json("authorize session owner")?
    {
        // THREAT[TM-AGENT-017]: another user's Platform Chat stays invisible.
        return Err(ErrorResponse::not_found("Session"));
    }
    let waiting = waiting_agents(
        &state.db,
        org.org_id,
        session_id,
        session.parent_session_id.is_none(),
    )
    .await
    .log_internal_error_json("read run requests")?;
    Ok(Json(RunRequestsResponse {
        requests: run_requests(&waiting),
    }))
}

/// The session in `session`'s run that waits on `tool_call_ids`. Shared by
/// the `/v1` answer endpoints.
pub(crate) async fn route_answer(
    db: &crate::storage::StorageBackend,
    org_id: i64,
    session: &crate::domains::sessions::record::Session,
    tool_call_ids: &[&str],
) -> Result<SessionId, (StatusCode, Json<ErrorResponse>)> {
    answer_target(
        db,
        org_id,
        session.id,
        session.parent_session_id.is_none(),
        tool_call_ids,
    )
    .await
    .map_err(answer_target_error)
}

pub(crate) fn answer_target_error(error: AnswerTargetError) -> (StatusCode, Json<ErrorResponse>) {
    match error {
        AnswerTargetError::SpansAgents => ErrorResponse::new(
            "These requests come from different agents. Answer each agent's requests separately"
                .to_string(),
        )
        .into_response(StatusCode::BAD_REQUEST),
        AnswerTargetError::Internal(detail) => {
            tracing::error!(error = %detail, "Failed to route an answer in a run");
            ErrorResponse::new("Internal server error".to_string())
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

// Client-side tool results HTTP routes
//
// POST /v1/sessions/{session_id}/tool-results
// Accepts tool results from the client and resumes the agent workflow.
//
// Design: When an LLM requests client-side tools, the workflow pauses and
// session status becomes "waiting_for_tool_results". The client receives a
// tool.call_requested event via SSE and submits results here. This endpoint
// stores tool results as tool.completed events and resumes the workflow.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::sessions::SessionService;
pub use crate::domains::tool_results::types::{
    ClientToolResult, SubmitToolResultsRequest, SubmitToolResultsResponse,
};
use crate::services::EventService;
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::TurnBackend;

use super::common::{ApiOptionExt, ApiResult, ApiResultExt, ErrorResponse, impl_auth_state};
use crate::domains::tool_results::waiting_turn_resolution::execute_waiting_turn_resolution;
use crate::storage::ClaimWaitingTurnResult;
use everruns_core::Caller;
use std::sync::Arc;

/// App state for tool results routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub event_service: EventService,
    pub runner: Arc<dyn TurnBackend>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        runner: Arc<dyn TurnBackend>,
        auth: AuthState,
        event_delivery: crate::live_updates::event_delivery::EventDelivery,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            event_service: EventService::new(db.clone(), event_delivery),
            db,
            runner,
            auth,
        }
    }
}

impl_auth_state!(AppState);

/// Create tool results routes.
///
/// URL mode elicitation consent is merged in here: it resumes a paused turn the
/// same way a client tool result does, just with a decision instead of output.
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/sessions/{session_id}/tool-results",
            post(submit_tool_results),
        )
        .merge(super::mcp_url_consent::routes())
        .merge(super::question_answers::routes())
        .merge(super::tool_approvals::routes())
        .with_state(state)
}

/// POST /v1/sessions/{session_id}/tool-results - Submit client-side tool results
///
/// Accepts tool results executed by the client and resumes the agent workflow.
/// Session must be in `waiting_for_tool_results` status.
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/tool-results",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    request_body = SubmitToolResultsRequest,
    responses(
        (status = 200, description = "Tool results accepted and workflow resumed", body = SubmitToolResultsResponse),
        (status = 400, description = "Invalid session ID or request"),
        (status = 404, description = "Session not found"),
        (status = 409, description = "Session not in waiting_for_tool_results state"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn submit_tool_results(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<SubmitToolResultsRequest>,
) -> ApiResult<SubmitToolResultsResponse> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid session ID: {}", e))
            .into_response(StatusCode::BAD_REQUEST)
    })?;

    if req.tool_results.is_empty() {
        return Err(
            ErrorResponse::new("tool_results must not be empty".to_string())
                .into_response(StatusCode::BAD_REQUEST),
        );
    }

    // Get session and verify status
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
        // THREAT[TM-AGENT-017]: raw tool results are another user-driven way
        // to resume Platform Chat and must bind to its persisted owner too.
        return Err(ErrorResponse::not_found("Session"));
    }
    let accepted = req.tool_results.len();
    let result = state
        .db
        .claim_tool_result_command(
            org.org_id,
            session_id,
            req.tool_results
                .iter()
                .map(ClientToolResult::completed)
                .collect(),
        )
        .await;
    if let Err(error) = &result
        && let Some(error) = error.downcast_ref::<everruns_contracts::AgentLoopError>()
    {
        return Err(ErrorResponse::new(error.to_string()).into_response(StatusCode::BAD_REQUEST));
    }
    let claim = match result.log_internal_error_json("claim waiting turn")? {
        ClaimWaitingTurnResult::Claimed(claim) => claim,
        ClaimWaitingTurnResult::Conflict { current_status } => {
            return Err(ErrorResponse::new(format!(
                "Session is not waiting for tool results (current status: {current_status})"
            ))
            .into_response(StatusCode::CONFLICT));
        }
        ClaimWaitingTurnResult::SessionNotFound => {
            return Err(ErrorResponse::not_found("Session"));
        }
    };
    let accepted = claim.plan.response["accepted"]
        .as_u64()
        .unwrap_or(accepted as u64) as usize;
    let result = execute_waiting_turn_resolution(
        &state.db,
        &state.event_service,
        &state.runner,
        org.org_id,
        session_id,
        &claim,
    )
    .await
    .map(|_| SubmitToolResultsResponse {
        accepted,
        status: if claim
            .plan
            .response
            .get("resume")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
        {
            "waiting_for_tool_results"
        } else {
            "active"
        }
        .to_string(),
    });
    Ok(Json(
        result.log_internal_error_json("resolve client tool results")?,
    ))
}

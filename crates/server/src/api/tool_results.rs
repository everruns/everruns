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
use crate::services::EventService;
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use everruns_core::events::{EventContext, EventRequest, ToolCompletedData};
use everruns_core::message::ContentPart;
use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
use everruns_worker::AgentRunner;

use super::common::{ApiOptionExt, ApiResult, ApiResultExt, ErrorResponse, impl_auth_state};
use crate::services::waiting_turn_resolution::execute_waiting_turn_resolution;
use crate::storage::models::{ClaimWaitingTurnResult, WaitingTurnResolutionPlan};
use everruns_core::Caller;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

/// Request to submit client-side tool results
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SubmitToolResultsRequest {
    /// Tool results from the client
    #[schema(example = json!([{"tool_call_id": "toolu_01933b5a00007000800000000000001", "result": {"url": "https://example.com/orders/42"}}]))]
    pub tool_results: Vec<ClientToolResult>,
}

/// A single tool result from the client
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ClientToolResult {
    /// Tool call ID (correlates with the tool call from tool.call_requested event)
    #[schema(example = "toolu_01933b5a00007000800000000000001")]
    pub tool_call_id: String,
    /// Result value (any JSON — object, array, string, number, etc.). Null if the tool failed.
    /// Example: `{"url": "https://example.com/orders/42"}`.
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    /// Error message if the tool failed
    #[serde(default)]
    #[schema(example = "Refund failed: order is outside refund window")]
    pub error: Option<String>,
}

/// Response from submitting tool results
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SubmitToolResultsResponse {
    /// Number of tool results accepted
    pub accepted: usize,
    /// Session status after submission
    pub status: String,
}

/// App state for tool results routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub event_service: EventService,
    pub runner: Arc<dyn AgentRunner>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        runner: Arc<dyn AgentRunner>,
        auth: AuthState,
        event_delivery: crate::event_delivery::EventDelivery,
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
    state
        .session_service
        .get(&caller, session_id.uuid(), None)
        .await
        .log_internal_error_json("get session")?
        .ok_or_not_found_json("Session")?;
    let turn_id = TurnId::from_uuid(session_id.uuid());
    let event_message_id = MessageId::from_uuid(session_id.uuid());

    let accepted = req.tool_results.len();
    let events = req
        .tool_results
        .iter()
        .map(|client_result| {
            let tool_result = if let Some(ref error) = client_result.error {
                ToolCompletedData::failure(
                    client_result.tool_call_id.clone(),
                    String::new(),
                    "error".to_string(),
                    error.clone(),
                    None,
                )
            } else {
                let result_content = client_result
                    .result
                    .as_ref()
                    .map(|result| vec![ContentPart::tool_result_text(result)])
                    .unwrap_or_default();
                ToolCompletedData::success(
                    client_result.tool_call_id.clone(),
                    String::new(),
                    result_content,
                    None,
                )
            };
            EventRequest::new(
                session_id,
                EventContext::turn(turn_id, event_message_id),
                tool_result,
            )
        })
        .collect();
    let plan = WaitingTurnResolutionPlan {
        kind: "tool_results".to_string(),
        events,
        session_values: Vec::new(),
        response: serde_json::json!({ "accepted": accepted }),
    };
    let claim = match state
        .db
        .claim_waiting_turn(org.org_id, session_id, plan)
        .await
        .log_internal_error_json("claim waiting turn")?
    {
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
        status: "active".to_string(),
    });
    Ok(Json(
        result.log_internal_error_json("resolve client tool results")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submit_tool_results_request_with_error() {
        let json = r#"{
            "tool_results": [
                {
                    "tool_call_id": "call_def456",
                    "error": "Connection refused"
                }
            ]
        }"#;

        let req: SubmitToolResultsRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.tool_results.len(), 1);
        assert_eq!(req.tool_results[0].tool_call_id, "call_def456");
        assert!(req.tool_results[0].result.is_none());
        assert_eq!(
            req.tool_results[0].error.as_ref().unwrap(),
            "Connection refused"
        );
    }

    #[test]
    fn test_submit_tool_results_request_multiple() {
        let json = r#"{
            "tool_results": [
                {"tool_call_id": "call_1", "result": "ok"},
                {"tool_call_id": "call_2", "error": "failed"},
                {"tool_call_id": "call_3", "result": {"key": "value"}}
            ]
        }"#;

        let req: SubmitToolResultsRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.tool_results.len(), 3);
    }
}

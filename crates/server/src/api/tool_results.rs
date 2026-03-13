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
use crate::services::{EventService, SessionService};
use crate::storage::StorageBackend;
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use everruns_core::SessionStatus;
use everruns_core::events::{EventContext, EventRequest, ToolCompletedData};
use everruns_core::message::ContentPart;
use everruns_core::typed_id::{MessageId, SessionId, TurnId};
use everruns_worker::AgentRunner;

use super::common::{ApiOptionExt, ApiResultExt, ErrorResponse};
use everruns_core::Caller;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

/// Request to submit client-side tool results
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SubmitToolResultsRequest {
    /// Tool results from the client
    pub tool_results: Vec<ClientToolResult>,
}

/// A single tool result from the client
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ClientToolResult {
    /// Tool call ID (correlates with the tool call from tool.call_requested event)
    pub tool_call_id: String,
    /// Result value (JSON). Null if the tool failed.
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    /// Error message if the tool failed
    #[serde(default)]
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
    pub fn new(db: Arc<StorageBackend>, runner: Arc<dyn AgentRunner>, auth: AuthState) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            event_service: EventService::new(db.clone()),
            db,
            runner,
            auth,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Create tool results routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/sessions/{session_id}/tool-results",
            post(submit_tool_results),
        )
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
) -> Result<Json<SubmitToolResultsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid session ID: {}", e),
            }),
        )
    })?;

    if req.tool_results.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "tool_results must not be empty".to_string(),
            }),
        ));
    }

    // Get session and verify status
    let caller = Caller::from(&org);
    let session = state
        .session_service
        .get(&caller, session_id.uuid(), None)
        .await
        .log_internal_error_json("get session")?
        .ok_or_not_found_json("Session")?;

    if session.status != SessionStatus::WaitingForToolResults {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: format!(
                    "Session is not waiting for tool results (current status: {})",
                    session.status
                ),
            }),
        ));
    }

    // Use session_id as turn_id (matches how DurableRunner uses workflow_id = session_id)
    let turn_id = TurnId::from_uuid(session_id.uuid());
    let input_message_id = MessageId::new();

    let accepted = req.tool_results.len();

    // Emit tool.completed events for each result
    for client_result in &req.tool_results {
        let tool_result = if let Some(ref error) = client_result.error {
            ToolCompletedData::failure(
                client_result.tool_call_id.clone(),
                String::new(), // tool name not available from client
                "error".to_string(),
                error.clone(),
                None,
            )
        } else {
            let result_content = client_result
                .result
                .as_ref()
                .map(|r| vec![ContentPart::text(r.to_string())])
                .unwrap_or_default();
            ToolCompletedData::success(
                client_result.tool_call_id.clone(),
                String::new(), // tool name not available from client
                result_content,
                None,
            )
        };

        let event = EventRequest::new(
            session_id,
            EventContext::turn(turn_id, input_message_id),
            tool_result,
        );

        if let Err(e) = state.event_service.emit(event).await {
            tracing::warn!(
                session_id = %session_id,
                tool_call_id = %client_result.tool_call_id,
                error = %e,
                "Failed to emit tool.completed event for client tool result"
            );
        }
    }

    // Set session status back to active
    if let Err(e) = state
        .session_service
        .update_status(&caller, session_id.uuid(), "active".to_string())
        .await
    {
        tracing::warn!(error = %e, "Failed to set session status to active");
    }

    // Resume the workflow by starting another run
    let runner = state.runner.clone();
    let harness_id = session.harness_id;
    let agent_id = session.agent_id;
    let org_id = org.org_id;
    tokio::spawn(async move {
        if let Err(e) = runner
            .start_run(org_id, session_id, harness_id, agent_id, input_message_id)
            .await
        {
            tracing::error!(
                session_id = %session_id,
                error = %e,
                "Failed to resume workflow after tool results"
            );
        } else {
            tracing::info!(
                session_id = %session_id,
                "Workflow resumed after client tool results"
            );
        }
    });

    Ok(Json(SubmitToolResultsResponse {
        accepted,
        status: "active".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_submit_tool_results_request_deserialization() {
        let json = r#"{
            "tool_results": [
                {
                    "tool_call_id": "call_abc123",
                    "result": {"status": "deployed", "url": "https://staging.app"}
                }
            ]
        }"#;

        let req: SubmitToolResultsRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.tool_results.len(), 1);
        assert_eq!(req.tool_results[0].tool_call_id, "call_abc123");
        assert!(req.tool_results[0].result.is_some());
        assert!(req.tool_results[0].error.is_none());
    }

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

    #[test]
    fn test_submit_tool_results_response_serialization() {
        let resp = SubmitToolResultsResponse {
            accepted: 2,
            status: "active".to_string(),
        };

        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["accepted"], 2);
        assert_eq!(json["status"], "active");
    }
}

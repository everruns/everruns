use super::AppState;
use crate::api::common::{ApiResult, ErrorResponse};
use crate::auth::ResolvedOrg;
use crate::domains::agents::types::{AgentPreviewResponse, PreviewAgentRequest};
use crate::domains::common::Command;
use axum::{Json, extract::State};

/// POST /v1/agents/preview - Preview the final agent shape with capabilities applied
///
/// Returns the merged system prompt and all tools that would be available to the agent.
/// This is useful for previewing what the agent will look like before saving.
#[utoipa::path(
    post,
    path = "/v1/agents/preview",
    request_body = PreviewAgentRequest,
    responses(
        (status = 200, description = "Agent preview generated", body = AgentPreviewResponse),
        (status = 403, description = "Harness view permission required", body = ErrorResponse),
        (status = 404, description = "Harness not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn preview_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<PreviewAgentRequest>,
) -> ApiResult<AgentPreviewResponse> {
    let result = crate::domains::agents::PreviewAgent {
        harness_id: req.harness_id,
        initial_files: req.initial_files,
        system_prompt: Some(req.system_prompt),
        capabilities: req.capabilities,
        tools: req.tools,
        mcp_servers: req.mcp_servers,
    }
    .run(&state.ctx(&org))
    .await?;

    Ok(Json(AgentPreviewResponse {
        features: result.features,
        initial_files: result.initial_files,
        system_prompt: result.system_prompt,
        tools: result.tools,
        findings: result.findings,
    }))
}

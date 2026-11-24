//! HTTP routes for an agent's effective MCP attachments. The logic lives in
//! the commands (`domains::agents::mcp_attachments`), which the CLI, MCP and
//! Platform Chat share.
use crate::auth::ResolvedOrg;
use crate::domains::agents::mcp_attachments::{
    AgentMcpAttachment, ListAgentMcpAttachments, RevokeAgentMcpConnection,
};
use crate::domains::common::Command;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};

use super::agents::AppState;
use super::common::{ApiResult, ErrorResponse};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/platform/agents/{agent_id}/mcp-attachments",
            get(list_agent_mcp_attachments),
        )
        .route(
            "/v1/platform/agents/{agent_id}/mcp-attachments/{name}/connection",
            axum::routing::delete(revoke_agent_mcp_connection),
        )
}

#[utoipa::path(
    get,
    path = "/v1/platform/agents/{agent_id}/mcp-attachments",
    description = "Lists the effective MCP attachments after capability, harness, and agent layers are merged. Connection state, the agent's sign-in state, and permitted actions are resolved for the current caller.",
    params(("agent_id" = String, Path, description = "Agent ID (prefixed) or name")),
    responses(
        (status = 200, description = "Effective MCP attachments for the agent", body = Vec<AgentMcpAttachment>),
        (status = 404, description = "Agent or harness not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "agents"
)]
pub async fn list_agent_mcp_attachments(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id_or_name): Path<String>,
) -> ApiResult<Vec<AgentMcpAttachment>> {
    Ok(Json(
        ListAgentMcpAttachments {
            id: agent_id_or_name,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

#[utoipa::path(
    delete,
    path = "/v1/platform/agents/{agent_id}/mcp-attachments/{name}/connection",
    description = "Revokes the current caller's user connection or the virtual user's shared service connection for an effective MCP attachment. The attachment configuration remains unchanged.",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed) or name"),
        ("name" = String, Path, description = "Effective MCP attachment name"),
    ),
    responses(
        (status = 204, description = "MCP connection revoked or already absent"),
        (status = 400, description = "Attachment does not use a connection", body = ErrorResponse),
        (status = 403, description = "Permission denied", body = ErrorResponse),
        (status = 404, description = "Agent, attachment, or preset not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "agents"
)]
pub async fn revoke_agent_mcp_connection(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id_or_name, name)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    RevokeAgentMcpConnection {
        id: agent_id_or_name,
        name,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// Agent-script HTTP routes.
//
// Sub-resource of an agent: `/v1/agents/{agent_id}/scripts`. Policy enforcement
// happens inside the domain commands (AGENT_VIEW / AGENT_MANAGE). Scripts need
// only storage, so the router reuses the agent-trigger state for its context.

use super::agent_triggers::AppState;
use crate::auth::ResolvedOrg;
use crate::domains::agent_scripts::types::{CreateAgentScriptRequest, UpdateAgentScriptRequest};
use crate::domains::agent_scripts::{
    CreateAgentScript, DeleteAgentScript, GetAgentScript, ListAgentScripts, UpdateAgentScriptCmd,
};
use crate::domains::common::Command;
use crate::records::AgentScript;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use serde::Deserialize;
use utoipa::IntoParams;

use super::common::{ApiResult, ErrorResponse};

/// Create agent-script routes.
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/agents/{agent_id}/scripts",
            get(list_agent_scripts).post(create_agent_script),
        )
        .route(
            "/v1/agents/{agent_id}/scripts/{script_id}",
            get(get_agent_script)
                .patch(update_agent_script)
                .delete(delete_agent_script),
        )
        .with_state(state)
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListAgentScriptsQuery {
    /// Include archived scripts (default false).
    #[serde(default)]
    pub include_archived: bool,
}

#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/scripts",
    description = "List an agent's saved scripts with their bodies. Set include_archived=true to also return archived scripts.",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed)"),
        ListAgentScriptsQuery
    ),
    responses(
        (status = 200, description = "Agent scripts", body = Vec<AgentScript>),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agent-scripts"
)]
pub async fn list_agent_scripts(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(query): Query<ListAgentScriptsQuery>,
) -> ApiResult<Vec<AgentScript>> {
    let scripts = ListAgentScripts {
        agent_id,
        include_archived: query.include_archived,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(scripts))
}

#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/scripts",
    params(("agent_id" = String, Path, description = "Agent ID (prefixed)")),
    request_body = CreateAgentScriptRequest,
    responses(
        (status = 201, description = "Script created", body = AgentScript),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Agent not found", body = ErrorResponse),
        (status = 409, description = "Name already in use", body = ErrorResponse)
    ),
    tag = "agent-scripts"
)]
pub async fn create_agent_script(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<CreateAgentScriptRequest>,
) -> Result<(StatusCode, Json<AgentScript>), (StatusCode, Json<ErrorResponse>)> {
    let script = CreateAgentScript { agent_id, req }
        .run(&state.ctx(&org))
        .await?;
    Ok((StatusCode::CREATED, Json(script)))
}

#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/scripts/{script_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed)"),
        ("script_id" = String, Path, description = "Script ID (prefixed)")
    ),
    responses(
        (status = 200, description = "Script", body = AgentScript),
        (status = 404, description = "Script not found", body = ErrorResponse)
    ),
    tag = "agent-scripts"
)]
pub async fn get_agent_script(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, script_id)): Path<(String, String)>,
) -> ApiResult<AgentScript> {
    let script = GetAgentScript {
        agent_id,
        script_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(script))
}

#[utoipa::path(
    patch,
    path = "/v1/agents/{agent_id}/scripts/{script_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed)"),
        ("script_id" = String, Path, description = "Script ID (prefixed)")
    ),
    request_body = UpdateAgentScriptRequest,
    responses(
        (status = 200, description = "Script updated", body = AgentScript),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Script not found", body = ErrorResponse)
    ),
    tag = "agent-scripts"
)]
pub async fn update_agent_script(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, script_id)): Path<(String, String)>,
    Json(req): Json<UpdateAgentScriptRequest>,
) -> ApiResult<AgentScript> {
    let script = UpdateAgentScriptCmd {
        agent_id,
        script_id,
        req,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(script))
}

#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}/scripts/{script_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed)"),
        ("script_id" = String, Path, description = "Script ID (prefixed)")
    ),
    responses(
        (status = 204, description = "Script archived"),
        (status = 404, description = "Script not found", body = ErrorResponse)
    ),
    tag = "agent-scripts"
)]
pub async fn delete_agent_script(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, script_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    DeleteAgentScript {
        agent_id,
        script_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

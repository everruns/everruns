// Session resource routes.
//
// Exposes the session resource registry — a unified view of all resources
// active in a session (sandboxes, subagents, browser sessions, etc.).

use crate::api::state::ApiState;
use crate::auth::ResolvedOrg;
use crate::domains::common::Command;
use crate::domains::mcp_servers::session_servers::{
    ChatMcpServer, ListChatMcpServers, RemoveChatMcpServer,
};
use crate::domains::session_resources::ListSessionResources;
use crate::kernel_imports::{SessionResourceEntry, contracts::typed_id::SessionId};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get},
};

use super::common::{ApiResult, ErrorResponse};

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/v1/sessions/{session_id}/resources", get(list_resources))
        .route(
            "/v1/sessions/{session_id}/mcp-servers",
            get(list_chat_mcp_servers),
        )
        .route(
            "/v1/sessions/{session_id}/mcp-servers/{name}",
            delete(remove_chat_mcp_server),
        )
        .with_state(state)
}

/// List all resources registered in the session resource registry.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/resources",
    responses(
        (status = 200, description = "Session resources", body = Vec<SessionResourceEntry>),
        (status = 404, description = "Session not found"),
    ),
    tag = "session-resources"
)]
pub async fn list_resources(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(session_id): Path<SessionId>,
) -> ApiResult<Vec<SessionResourceEntry>> {
    Ok(Json(
        ListSessionResources {
            session_id: session_id.to_string(),
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

/// List the MCP servers added to this chat only.
///
/// Servers a person added with "this chat only" join every later turn of this
/// session and no other. Sign-in state is the viewer's own.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/mcp-servers",
    params(("session_id" = String, Path, description = "Session ID")),
    responses(
        (status = 200, description = "Chat-only MCP servers", body = Vec<ChatMcpServer>),
        (status = 404, description = "Session not found", body = ErrorResponse),
    ),
    tag = "session-resources"
)]
pub async fn list_chat_mcp_servers(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> ApiResult<Vec<ChatMcpServer>> {
    Ok(Json(
        ListChatMcpServers { session_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}

/// Remove an MCP server added to this chat only. Its tools leave from the next turn.
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}/mcp-servers/{name}",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("name" = String, Path, description = "Server name")
    ),
    responses(
        (status = 204, description = "Server removed"),
        (status = 404, description = "Session or chat-only server not found", body = ErrorResponse),
    ),
    tag = "session-resources"
)]
pub async fn remove_chat_mcp_server(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path((session_id, name)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let removed = RemoveChatMcpServer { session_id, name }
        .run(&state.ctx(&org))
        .await?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::not_found("MCP server"))
    }
}

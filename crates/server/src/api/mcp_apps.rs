// MCP Apps host API: the console loads a view an MCP server ships and relays
// the calls the view makes. Console-internal like `/commands`, so the routes
// are not part of the public OpenAPI document.
//
// Rules live in `domains::session_commands::mcp_apps`; see
// knowledge/integrations/mcp-apps-host.md.

use super::commands::AppState;
use super::common::{ApiPolicyResultExt, ApiResult};
use crate::auth::ResolvedOrg;
use crate::domains::session_commands::mcp_apps::{
    McpAppResourceReadRequest, McpAppToolCallRequest, McpAppToolCallResponse, McpAppView,
};
use crate::domains::sessions::{SESSION_MANAGE, SESSION_VIEW};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use everruns_contracts::typed_id::SessionId;
use everruns_core::Caller;
use serde::Deserialize;
use serde_json::Value;

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/sessions/{session_id}/mcp-apps/view", get(get_view))
        .route(
            "/v1/sessions/{session_id}/mcp-apps/tools-call",
            post(call_tool),
        )
        .route(
            "/v1/sessions/{session_id}/mcp-apps/resources-read",
            post(read_resource),
        )
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct ViewQuery {
    server: String,
    uri: String,
}

/// GET /v1/sessions/{session_id}/mcp-apps/view?server=&uri=
async fn get_view(
    State(state): State<AppState>,
    org: ResolvedOrg,
    Path(session_id): Path<SessionId>,
    Query(query): Query<ViewQuery>,
) -> ApiResult<McpAppView> {
    let caller = Caller::from(&org);
    SESSION_VIEW
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(anyhow::Error::from)
        .map_policy_or_internal("authorize MCP app view")?;
    let view = state
        .command_service
        .read_mcp_app_view(&caller, session_id, &query.server, &query.uri)
        .await
        .map_policy_or_internal("read MCP app view")?;
    Ok(Json(view))
}

/// POST /v1/sessions/{session_id}/mcp-apps/tools-call
async fn call_tool(
    State(state): State<AppState>,
    org: ResolvedOrg,
    Path(session_id): Path<SessionId>,
    Json(request): Json<McpAppToolCallRequest>,
) -> ApiResult<McpAppToolCallResponse> {
    let caller = Caller::from(&org);
    SESSION_MANAGE
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(anyhow::Error::from)
        .map_policy_or_internal("authorize MCP app tool call")?;
    let response = state
        .command_service
        .call_mcp_app_tool(&caller, session_id, request)
        .await
        .map_policy_or_internal("call MCP app tool")?;
    Ok(Json(response))
}

/// POST /v1/sessions/{session_id}/mcp-apps/resources-read
async fn read_resource(
    State(state): State<AppState>,
    org: ResolvedOrg,
    Path(session_id): Path<SessionId>,
    Json(request): Json<McpAppResourceReadRequest>,
) -> ApiResult<Value> {
    let caller = Caller::from(&org);
    SESSION_VIEW
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(anyhow::Error::from)
        .map_policy_or_internal("authorize MCP app resource read")?;
    let result = state
        .command_service
        .read_mcp_app_resource(&caller, session_id, request)
        .await
        .map_policy_or_internal("read MCP app resource")?;
    Ok(Json(result))
}

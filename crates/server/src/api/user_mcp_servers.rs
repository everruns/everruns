// User MCP servers: self-service routes for the servers a person adds for
// themselves. Spec: knowledge/integrations/user-mcp-servers.md.
//
// Same authority as the other `/v1/virtual-users/{id}/...` profile routes:
// `me` (or your own id) for yourself, or virtual-user management authority
// for an account you manage. Organization MCP management grants nothing here.

use crate::api::state::ApiState;
use crate::api::virtual_users::authorized_profile;
use crate::auth::runtime::RuntimeAccount;
use crate::domains::mcp_servers::user_servers::{
    AddUserMcpServerRequest, UpdateUserMcpServerRequest, UserMcpServer, UserMcpServerError,
    UserMcpServers,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use everruns_contracts::typed_id::McpServerId;

use super::common::ErrorResponse;

type ApiError = (StatusCode, Json<ErrorResponse>);

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route(
            "/v1/virtual-users/{identity_id}/mcp-servers",
            get(list_user_mcp_servers).post(add_user_mcp_server),
        )
        .route(
            "/v1/virtual-users/{identity_id}/mcp-servers/{server_id}",
            get(get_user_mcp_server)
                .patch(update_user_mcp_server)
                .delete(remove_user_mcp_server),
        )
}

/// List response for user MCP servers.
#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct UserMcpServersResponse {
    pub data: Vec<UserMcpServer>,
}

fn error(e: UserMcpServerError) -> ApiError {
    let status = match &e {
        UserMcpServerError::Invalid(_) => StatusCode::BAD_REQUEST,
        UserMcpServerError::NotFound => StatusCode::NOT_FOUND,
        UserMcpServerError::Conflict(_) => StatusCode::CONFLICT,
        UserMcpServerError::Internal(err) => {
            tracing::error!(error = %err, "user MCP server request failed");
            return ErrorResponse::new("User MCP servers unavailable")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    ErrorResponse::new(e.to_string()).into_response(status)
}

fn parse_server_id(raw: &str) -> Result<uuid::Uuid, ApiError> {
    raw.parse::<McpServerId>().map(|id| id.uuid()).map_err(|_| {
        ErrorResponse::new("MCP server not found").into_response(StatusCode::NOT_FOUND)
    })
}

async fn servers<'a>(
    state: &'a ApiState,
    org: &RuntimeAccount,
    raw: &str,
) -> Result<UserMcpServers<'a>, ApiError> {
    let owner = authorized_profile(state, org, raw).await?;
    Ok(UserMcpServers {
        db: &state.db,
        encryption: state.encryption.as_deref(),
        org_id: org.org_id,
        owner: owner.uuid(),
    })
}

#[utoipa::path(summary = "List the person's own MCP servers.", get, path = "/v1/virtual-users/{identity_id}/mcp-servers", params(("identity_id" = String, Path, description = "`me` or a virtual user id")), responses((status = 200, description = "Success", body = UserMcpServersResponse), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied")), tag = "virtual-users")]
pub async fn list_user_mcp_servers(
    org: RuntimeAccount,
    State(state): State<ApiState>,
    Path(raw): Path<String>,
) -> Result<Json<UserMcpServersResponse>, ApiError> {
    let data = servers(&state, &org, &raw)
        .await?
        .list()
        .await
        .map_err(error)?;
    Ok(Json(UserMcpServersResponse { data }))
}

#[utoipa::path(summary = "Add an MCP server for the person, from the catalog or by URL.", post, path = "/v1/virtual-users/{identity_id}/mcp-servers", params(("identity_id" = String, Path, description = "`me` or a virtual user id")), request_body = AddUserMcpServerRequest, responses((status = 201, description = "Created", body = UserMcpServer), (status = 400, description = "Invalid request"), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied"), (status = 409, description = "Name already used")), tag = "virtual-users")]
pub async fn add_user_mcp_server(
    org: RuntimeAccount,
    State(state): State<ApiState>,
    Path(raw): Path<String>,
    Json(req): Json<AddUserMcpServerRequest>,
) -> Result<(StatusCode, Json<UserMcpServer>), ApiError> {
    let server = servers(&state, &org, &raw)
        .await?
        .add(req)
        .await
        .map_err(error)?;
    Ok((StatusCode::CREATED, Json(server)))
}

#[utoipa::path(summary = "Read one of the person's MCP servers.", get, path = "/v1/virtual-users/{identity_id}/mcp-servers/{server_id}", params(("identity_id" = String, Path, description = "`me` or a virtual user id"), ("server_id" = String, Path)), responses((status = 200, description = "Success", body = UserMcpServer), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied"), (status = 404, description = "Not found")), tag = "virtual-users")]
pub async fn get_user_mcp_server(
    org: RuntimeAccount,
    State(state): State<ApiState>,
    Path((raw, server_id)): Path<(String, String)>,
) -> Result<Json<UserMcpServer>, ApiError> {
    let id = parse_server_id(&server_id)?;
    let server = servers(&state, &org, &raw)
        .await?
        .get(id)
        .await
        .map_err(error)?;
    Ok(Json(server))
}

#[utoipa::path(summary = "Rename, enable, disable or replace the API key of one of the person's MCP servers.", patch, path = "/v1/virtual-users/{identity_id}/mcp-servers/{server_id}", params(("identity_id" = String, Path, description = "`me` or a virtual user id"), ("server_id" = String, Path)), request_body = UpdateUserMcpServerRequest, responses((status = 200, description = "Success", body = UserMcpServer), (status = 400, description = "Invalid request"), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied"), (status = 404, description = "Not found"), (status = 409, description = "Name already used")), tag = "virtual-users")]
pub async fn update_user_mcp_server(
    org: RuntimeAccount,
    State(state): State<ApiState>,
    Path((raw, server_id)): Path<(String, String)>,
    Json(req): Json<UpdateUserMcpServerRequest>,
) -> Result<Json<UserMcpServer>, ApiError> {
    let id = parse_server_id(&server_id)?;
    let server = servers(&state, &org, &raw)
        .await?
        .update(id, req)
        .await
        .map_err(error)?;
    Ok(Json(server))
}

#[utoipa::path(summary = "Remove one of the person's MCP servers and its sign-in.", delete, path = "/v1/virtual-users/{identity_id}/mcp-servers/{server_id}", params(("identity_id" = String, Path, description = "`me` or a virtual user id"), ("server_id" = String, Path)), responses((status = 204, description = "Removed"), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied"), (status = 404, description = "Not found")), tag = "virtual-users")]
pub async fn remove_user_mcp_server(
    org: RuntimeAccount,
    State(state): State<ApiState>,
    Path((raw, server_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let id = parse_server_id(&server_id)?;
    servers(&state, &org, &raw)
        .await?
        .remove(id)
        .await
        .map_err(error)?;
    Ok(StatusCode::NO_CONTENT)
}

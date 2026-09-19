use super::AppState;
use crate::auth::ResolvedOrg;
use crate::auth::middleware::AuthUser;
use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct UserMcpConnectionResponse {
    pub provider: String,
    pub server_id: String,
    pub server_name: String,
    pub server_url: String,
    pub server_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<String>,
    pub connected_at: DateTime<Utc>,
}

#[utoipa::path(
    get,
    path = "/v1/user/mcp-connections",
    responses(
        (status = 200, description = "Current user's MCP OAuth connections in the selected organization", body = Vec<UserMcpConnectionResponse>),
        (status = 500, description = "Internal server error"),
    ),
    tag = "users"
)]
pub async fn list_mcp_connections(
    State(state): State<AppState>,
    org: ResolvedOrg,
    auth: AuthUser,
) -> Result<Json<Vec<UserMcpConnectionResponse>>, StatusCode> {
    let rows = state
        .db
        .list_user_mcp_connections(org.org_id, auth.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "failed to list user MCP connections");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(
        rows.into_iter()
            .map(|row| UserMcpConnectionResponse {
                provider: row.provider,
                server_id: row.server_id.to_string(),
                server_name: row.server_name,
                server_url: row.server_url,
                server_status: row.server_status,
                provider_username: row.provider_username,
                scopes: row.scopes,
                connected_at: row.connected_at,
            })
            .collect(),
    ))
}

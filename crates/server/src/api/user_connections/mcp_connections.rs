use super::super::common::ErrorResponse;
use super::super::pagination::bounded_page_limit;
use super::AppState;
use crate::auth::ResolvedOrg;
use crate::auth::middleware::AuthUser;
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 100;

#[derive(Debug, Default, Deserialize, IntoParams)]
pub struct ListUserMcpConnectionsQuery {
    /// Continue after this opaque connection cursor.
    pub cursor: Option<String>,
    /// Page size (default: 50, max: 100).
    pub limit: Option<u32>,
}

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
#[derive(Debug, Serialize, ToSchema)]
pub struct UserMcpConnectionsResponse {
    pub data: Vec<UserMcpConnectionResponse>,
    pub next_cursor: Option<String>,
}

#[utoipa::path(
    get,
    path = "/v1/user/mcp-connections",
    params(ListUserMcpConnectionsQuery),
    responses(
        (status = 200, description = "Cursor-paginated current user's MCP OAuth connections in the selected organization", body = UserMcpConnectionsResponse),
        (status = 400, description = "Invalid cursor or limit", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    ),
    tag = "users"
)]
pub async fn list_mcp_connections(
    State(state): State<AppState>,
    org: ResolvedOrg,
    auth: AuthUser,
    Query(query): Query<ListUserMcpConnectionsQuery>,
) -> Result<Json<UserMcpConnectionsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let limit = bounded_page_limit(query.limit, DEFAULT_LIMIT, MAX_LIMIT)
        .map_err(|message| ErrorResponse::new(message).into_response(StatusCode::BAD_REQUEST))?;
    let cursor = query
        .cursor
        .map(|cursor| {
            Uuid::parse_str(&cursor).map_err(|_| {
                ErrorResponse::new("Invalid connection cursor")
                    .into_response(StatusCode::BAD_REQUEST)
            })
        })
        .transpose()?;
    let mut rows = state
        .db
        .list_user_mcp_connections_page(org.org_id, auth.id, cursor, i64::from(limit) + 1)
        .await
        .map_err(|error| {
            tracing::error!(%error, "failed to list user MCP connections");
            ErrorResponse::internal_error()
        })?;
    let has_more = rows.len() > limit as usize;
    if has_more {
        rows.truncate(limit as usize);
    }
    let next_cursor = has_more
        .then(|| rows.last().map(|row| row.connection_id.to_string()))
        .flatten();
    Ok(Json(UserMcpConnectionsResponse {
        data: rows
            .into_iter()
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
        next_cursor,
    }))
}

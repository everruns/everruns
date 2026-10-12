// Connected AI clients API: list and revoke the external MCP clients the
// signed-in person approved on `/oauth/authorize`.
//
// Spec: knowledge/integrations/mcp-connected-clients.md (phase 1). User-scoped
// like `/v1/user/preferences`: a grant spans all of the person's organizations.
// MCP access tokens are rejected on `/api/*`, so a connected client cannot list
// or revoke grants itself.

use crate::api::state::ApiState;
use crate::auth::audit;
use crate::auth::middleware::AuthUser;
use crate::domains::connected_clients::{self, ConnectedClientsResponse};
use axum::{
    Json, Router,
    extract::{ConnectInfo, Extension, Path, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get},
};
use std::net::SocketAddr;
use uuid::Uuid;

use super::common::ErrorResponse;

type ApiError = (StatusCode, Json<ErrorResponse>);

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/v1/user/connected-clients", get(list_connected_clients))
        .route(
            "/v1/user/connected-clients/{grant_id}",
            delete(revoke_connected_client),
        )
        .with_state(state)
}

fn internal(context: &str, error: anyhow::Error) -> ApiError {
    tracing::error!(%error, "{context}");
    ErrorResponse::new("Connected AI clients unavailable")
        .into_response(StatusCode::INTERNAL_SERVER_ERROR)
}

#[utoipa::path(summary = "List the AI clients you connected to Everruns MCP.", description = "External MCP clients (Claude, ChatGPT, Cursor, ...) you approved to act as you on `/mcp`, most recently used first.", get, path = "/v1/user/connected-clients", responses((status = 200, description = "Success", body = ConnectedClientsResponse), (status = 401, description = "Authentication required")), tag = "users")]
pub async fn list_connected_clients(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<ConnectedClientsResponse>, ApiError> {
    let data = connected_clients::list(&state.db, user.id)
        .await
        .map_err(|e| internal("Failed to list connected AI clients", e))?;
    Ok(Json(ConnectedClientsResponse { data }))
}

#[utoipa::path(summary = "Disconnect an AI client.", description = "Revokes the grant: the client's refresh tokens are deleted and `/mcp` rejects its access tokens within about 30 seconds. Reconnecting needs a new approval.", delete, path = "/v1/user/connected-clients/{grant_id}", params(("grant_id" = Uuid, Path, description = "Connected client (grant) id")), responses((status = 204, description = "Revoked"), (status = 401, description = "Authentication required"), (status = 404, description = "Not found")), tag = "users")]
pub async fn revoke_connected_client(
    State(state): State<ApiState>,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    user: AuthUser,
    Path(grant_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let client_id = connected_clients::revoke(&state.db, user.id, grant_id)
        .await
        .map_err(|e| internal("Failed to revoke connected AI client", e))?
        .ok_or_else(|| {
            ErrorResponse::new("Connected client not found").into_response(StatusCode::NOT_FOUND)
        })?;

    state.auth.backend.on_mcp_grant_revoked(grant_id).await;
    audit::emit(
        state.db.clone(),
        user.organizations
            .first()
            .map(|o| o.org_id)
            .unwrap_or(everruns_core::DEFAULT_ORG_ID),
        Some(user.id),
        "auth.mcp_oauth.revoke",
        audit::client_ip_from_connect_info(connect_info, &headers),
        serde_json::json!({"client_id": client_id, "grant_id": grant_id.to_string()}),
    );
    Ok(StatusCode::NO_CONTENT)
}

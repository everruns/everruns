// Management routes for agent keys, the credentials of an agent's `api` channel.
// The commands own the rules; see `domains/agent_channels/commands/keys.rs`.

use crate::auth::ResolvedOrg;
use crate::domains::agent_channels::record::api::{
    AgentKey, AgentKeyPermission, AgentKeyWithSecret,
};
use crate::domains::agent_channels::{
    CreateAgentKey, ListAgentKeys, RevokeAgentKey, RotateAgentKey,
};
use crate::domains::common::Command;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use utoipa::ToSchema;

use super::common::{ApiResult, ErrorResponse};

pub type AppState = super::agent_triggers::AppState;

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/agents/{agent_id}/channels/{channel_id}/keys",
            get(list_agent_keys).post(create_agent_key),
        )
        .route(
            "/v1/agents/{agent_id}/channels/{channel_id}/keys/{key_id}/rotate",
            post(rotate_agent_key),
        )
        .route(
            "/v1/agents/{agent_id}/channels/{channel_id}/keys/{key_id}/revoke",
            post(revoke_agent_key),
        )
        .with_state(state)
}

/// Request to create an agent key.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAgentKeyRequest {
    /// Display name, e.g. the application that holds the key.
    #[schema(example = "Support backend")]
    pub name: String,
    /// When the key stops working. Omit for no expiry.
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// What the key may do. Default `["sessions"]`; add `end_user` to let the
    /// key act for the application's users with the `End-User` header.
    #[serde(default)]
    pub permissions: Option<Vec<AgentKeyPermission>>,
}

/// Request to rotate an agent key.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct RotateAgentKeyRequest {
    /// Hours the replaced secret keeps working (0 to 168, default 24).
    #[serde(default)]
    pub overlap_hours: Option<u32>,
}

#[utoipa::path(
    description = "List the agent keys of an agent's API channel. Secrets are never returned.",
    get,
    path = "/v1/agents/{agent_id}/channels/{channel_id}/keys",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("channel_id" = String, Path, description = "API channel ID")
    ),
    responses(
        (status = 200, description = "Agent keys, newest first", body = Vec<AgentKey>),
        (status = 404, description = "Agent or API channel not found", body = ErrorResponse)
    ),
    tag = "agent-channels"
)]
pub async fn list_agent_keys(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, channel_id)): Path<(String, String)>,
) -> ApiResult<Vec<AgentKey>> {
    let keys = ListAgentKeys {
        agent_id,
        channel_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(keys))
}

#[utoipa::path(
    description = "Create an agent key for an agent's API channel. The secret is in this response only.",
    post,
    path = "/v1/agents/{agent_id}/channels/{channel_id}/keys",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("channel_id" = String, Path, description = "API channel ID")
    ),
    request_body = CreateAgentKeyRequest,
    responses(
        (status = 201, description = "Key created", body = AgentKeyWithSecret),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Agent or API channel not found", body = ErrorResponse)
    ),
    tag = "agent-channels"
)]
pub async fn create_agent_key(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, channel_id)): Path<(String, String)>,
    Json(req): Json<CreateAgentKeyRequest>,
) -> Result<(StatusCode, Json<AgentKeyWithSecret>), (StatusCode, Json<ErrorResponse>)> {
    let key = CreateAgentKey {
        agent_id,
        channel_id,
        name: req.name,
        expires_at: req.expires_at,
        permissions: req.permissions,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok((StatusCode::CREATED, Json(key)))
}

#[utoipa::path(
    description = "Replace an agent key's secret, keeping its id. The new secret is in this response only; the old one keeps working for the overlap.",
    post,
    path = "/v1/agents/{agent_id}/channels/{channel_id}/keys/{key_id}/rotate",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("channel_id" = String, Path, description = "API channel ID"),
        ("key_id" = String, Path, description = "Agent key ID")
    ),
    request_body = RotateAgentKeyRequest,
    responses(
        (status = 200, description = "Key rotated", body = AgentKeyWithSecret),
        (status = 400, description = "Invalid overlap", body = ErrorResponse),
        (status = 404, description = "Key not found or revoked", body = ErrorResponse)
    ),
    tag = "agent-channels"
)]
pub async fn rotate_agent_key(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, channel_id, key_id)): Path<(String, String, String)>,
    req: Option<Json<RotateAgentKeyRequest>>,
) -> ApiResult<AgentKeyWithSecret> {
    let req = req.map(|Json(req)| req).unwrap_or_default();
    let key = RotateAgentKey {
        agent_id,
        channel_id,
        key_id,
        overlap_hours: req.overlap_hours,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(key))
}

#[utoipa::path(
    description = "Revoke an agent key. It never works again, including a secret still in its rotation overlap.",
    post,
    path = "/v1/agents/{agent_id}/channels/{channel_id}/keys/{key_id}/revoke",
    params(
        ("agent_id" = String, Path, description = "Agent ID or name"),
        ("channel_id" = String, Path, description = "API channel ID"),
        ("key_id" = String, Path, description = "Agent key ID")
    ),
    responses(
        (status = 200, description = "Key revoked", body = AgentKey),
        (status = 404, description = "Key not found", body = ErrorResponse)
    ),
    tag = "agent-channels"
)]
pub async fn revoke_agent_key(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, channel_id, key_id)): Path<(String, String, String)>,
) -> ApiResult<AgentKey> {
    let key = RevokeAgentKey {
        agent_id,
        channel_id,
        key_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(key))
}

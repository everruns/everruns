// Session Storage HTTP routes
// Routes for listing session key-value storage and secrets
// Secrets are returned without their values for security

use crate::api::state::ApiState;
use crate::auth::ResolvedOrg;
use crate::domains::common::Command;
pub use crate::domains::session_storage::types::{
    BatchSetSecretsRequest, BatchSetSecretsResponse, KeyValueInfo, SecretInfo,
};
pub use crate::domains::session_storage::types::{AgentSecretInfo, SecretAuth, SecretBinding};
use crate::domains::session_storage::{
    BatchSetSessionSecrets, DeleteAgentSecret, DeleteSessionSecret, ListAgentSecrets,
    ListSessionSecrets, ListSessionStorage, SetAgentSecret,
};
use crate::kernel_imports::contracts::typed_id::SessionId;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};

use super::common::{ApiResult, ErrorResponse, ListResponse};

/// Create session storage routes (nested under sessions)
pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/v1/sessions/{session_id}/storage/keys", get(list_keys))
        .route(
            "/v1/sessions/{session_id}/storage/secrets",
            get(list_secrets).put(batch_set_secrets),
        )
        .route(
            "/v1/sessions/{session_id}/storage/secrets/{name}",
            axum::routing::delete(delete_secret),
        )
        .route("/v1/agents/{agent_id}/secrets", get(list_agent_secrets))
        .route(
            "/v1/agents/{agent_id}/secrets/{name}",
            axum::routing::put(set_agent_secret).delete(delete_agent_secret),
        )
        .with_state(state)
}

#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}/storage/secrets/{name}",
    description = "Delete a user-managed encrypted secret from one session.",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("name" = String, Path, description = "Secret name")
    ),
    responses(
        (status = 204, description = "Session secret deleted"),
        (status = 404, description = "Session secret not found", body = ErrorResponse)
    ),
    tag = "session-storage"
)]
pub async fn delete_secret(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path((session_id, name)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let deleted = DeleteSessionSecret { session_id, name }
        .run(&state.ctx(&org))
        .await?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::not_found("Secret"))
    }
}

/// GET /v1/sessions/{session_id}/storage/keys - List all key-value pairs
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/storage/keys",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "List of key-value pairs", body = ListResponse<KeyValueInfo>),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "session-storage"
)]
pub async fn list_keys(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> Result<Json<ListResponse<KeyValueInfo>>, StatusCode> {
    let session_id: SessionId = session_id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    let items = ListSessionStorage {
        session_id: session_id.to_string(),
    }
    .run(&state.ctx(&org))
    .await
    .map_err(|e| e.status())?;
    Ok(Json(ListResponse::new(items)))
}

/// GET /v1/sessions/{session_id}/storage/secrets - List all secrets (names only)
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/storage/secrets",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "List of secrets (names only, no values)", body = ListResponse<SecretInfo>),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "session-storage"
)]
pub async fn list_secrets(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
) -> Result<Json<ListResponse<SecretInfo>>, StatusCode> {
    let session_id: SessionId = session_id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    let items = ListSessionSecrets {
        session_id: session_id.to_string(),
    }
    .run(&state.ctx(&org))
    .await
    .map_err(|e| e.status())?;
    Ok(Json(ListResponse::new(items)))
}

/// PUT /v1/sessions/{session_id}/storage/secrets - Batch set secrets
///
/// Encrypts and stores multiple secrets in a single request.
/// Existing secrets with the same name are overwritten.
#[utoipa::path(
    put,
    path = "/v1/sessions/{session_id}/storage/secrets",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = BatchSetSecretsRequest,
    responses(
        (status = 200, description = "Secrets stored", body = BatchSetSecretsResponse),
        (status = 400, description = "Bad request (encryption not configured or invalid input)"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "session-storage"
)]
pub async fn batch_set_secrets(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
    Json(body): Json<BatchSetSecretsRequest>,
) -> ApiResult<BatchSetSecretsResponse> {
    let session_id: SessionId = session_id.parse().map_err(|_| {
        ErrorResponse::new("Invalid session ID").into_response(StatusCode::BAD_REQUEST)
    })?;
    Ok(Json(
        BatchSetSessionSecrets {
            session_id: session_id.to_string(),
            secrets: body.secrets,
            bindings: body.bindings,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

/// Request to store an agent's bound secret.
#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct SetAgentSecretRequest {
    /// Write-only value; encrypted and never returned.
    pub value: String,
    /// Where the value may be sent, and how.
    pub binding: SecretBinding,
}

/// GET /v1/agents/{agent_id}/secrets - List an agent's bound secrets
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/secrets",
    description = "List an agent's bound secrets: names, bound URLs and auth forms, never values.",
    params(("agent_id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Bound secrets (no values)", body = ListResponse<AgentSecretInfo>),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn list_agent_secrets(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(agent_id): Path<String>,
) -> ApiResult<ListResponse<AgentSecretInfo>> {
    let items = ListAgentSecrets { agent_id }.run(&state.ctx(&org)).await?;
    Ok(Json(ListResponse::new(items)))
}

/// PUT /v1/agents/{agent_id}/secrets/{name} - Store an agent's bound secret
#[utoipa::path(
    put,
    path = "/v1/agents/{agent_id}/secrets/{name}",
    description = "Encrypt and store a bound secret for every session of an agent. The shell sees $NAME as a placeholder; the value goes only on requests to the bound URL.",
    params(
        ("agent_id" = String, Path, description = "Agent ID"),
        ("name" = String, Path, description = "Env var name, e.g. JEV_API_KEY")
    ),
    request_body = SetAgentSecretRequest,
    responses(
        (status = 200, description = "Bound secret stored", body = AgentSecretInfo),
        (status = 400, description = "Invalid name, binding or value", body = ErrorResponse),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn set_agent_secret(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path((agent_id, name)): Path<(String, String)>,
    Json(body): Json<SetAgentSecretRequest>,
) -> ApiResult<AgentSecretInfo> {
    Ok(Json(
        SetAgentSecret {
            agent_id,
            name,
            value: body.value,
            binding: body.binding,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

/// DELETE /v1/agents/{agent_id}/secrets/{name} - Delete an agent's bound secret
#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}/secrets/{name}",
    params(
        ("agent_id" = String, Path, description = "Agent ID"),
        ("name" = String, Path, description = "Secret name")
    ),
    responses(
        (status = 204, description = "Bound secret deleted"),
        (status = 404, description = "Agent or secret not found", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn delete_agent_secret(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path((agent_id, name)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    DeleteAgentSecret { agent_id, name }
        .run(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// Trivial derive-only serde round-trips removed; covered by the derive + handler tests.

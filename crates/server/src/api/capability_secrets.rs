// Capability Secrets API endpoints
// Routes: /v1/agents/{agent_id}/capabilities/{capability_id}/secrets/...
//
// These endpoints manage encrypted secrets for capability configurations,
// such as API keys for web search providers.
//
// Secrets are write-only - values cannot be read back, only set/checked/deleted.

use crate::auth::{AuthState, ResolvedOrg};
use crate::storage::{EncryptionService, StorageBackend};
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, put},
};
use everruns_core::typed_id::AgentId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::ErrorResponse;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            encryption,
            auth,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

// ============================================================================
// Request/Response Types
// ============================================================================

/// Request to set a capability secret
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SetCapabilitySecretRequest {
    /// The secret value (will be encrypted before storage)
    pub value: String,
}

/// Response from setting a capability secret
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SetCapabilitySecretResponse {
    /// Whether the operation was successful
    pub success: bool,
}

/// Response listing capability secret names
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ListCapabilitySecretsResponse {
    /// List of secret names (not values)
    pub secrets: Vec<String>,
}

/// Response checking if a secret exists
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SecretExistsResponse {
    /// Whether the secret exists
    pub exists: bool,
}

// ============================================================================
// Path Parameters
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CapabilitySecretPath {
    pub agent_id: String,
    pub capability_id: String,
    pub secret_name: String,
}

#[derive(Debug, Deserialize)]
pub struct CapabilityPath {
    pub agent_id: String,
    pub capability_id: String,
}

// ============================================================================
// Endpoint Handlers
// ============================================================================

/// Set a capability secret
///
/// Stores an encrypted secret for a specific capability on an agent.
/// The value is encrypted before storage and cannot be read back.
#[utoipa::path(
    put,
    path = "/v1/agents/{agent_id}/capabilities/{capability_id}/secrets/{secret_name}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed, e.g., agt_...)"),
        ("capability_id" = String, Path, description = "Capability ID (e.g., web_search)"),
        ("secret_name" = String, Path, description = "Secret name (e.g., api_key)")
    ),
    request_body = SetCapabilitySecretRequest,
    responses(
        (status = 200, description = "Secret set successfully", body = SetCapabilitySecretResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Agent not found"),
        (status = 500, description = "Internal error")
    ),
    tag = "capability-secrets"
)]
pub async fn set_secret(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(params): Path<CapabilitySecretPath>,
    Json(req): Json<SetCapabilitySecretRequest>,
) -> Result<Json<SetCapabilitySecretResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Parse agent ID
    let agent_id: AgentId = params.agent_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid agent ID: {}", e),
            }),
        )
    })?;

    // Check encryption is configured
    let encryption = state.encryption.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Encryption not configured. Cannot store secrets.".to_string(),
            }),
        )
    })?;

    // Verify agent exists and belongs to org
    let _agent = state
        .db
        .get_agent(org.org_id, agent_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Agent not found".to_string(),
                }),
            )
        })?;

    // Encrypt the value
    let encrypted = encryption.encrypt_string(&req.value).map_err(|e| {
        tracing::error!("Encryption failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal server error".to_string(),
            }),
        )
    })?;

    // Store the secret
    state
        .db
        .upsert_capability_secret(
            agent_id.uuid(),
            &params.capability_id,
            &params.secret_name,
            encrypted,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to store capability secret: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;

    Ok(Json(SetCapabilitySecretResponse { success: true }))
}

/// Delete a capability secret
#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}/capabilities/{capability_id}/secrets/{secret_name}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed, e.g., agt_...)"),
        ("capability_id" = String, Path, description = "Capability ID (e.g., web_search)"),
        ("secret_name" = String, Path, description = "Secret name (e.g., api_key)")
    ),
    responses(
        (status = 204, description = "Secret deleted"),
        (status = 400, description = "Invalid agent ID"),
        (status = 404, description = "Agent or secret not found")
    ),
    tag = "capability-secrets"
)]
pub async fn delete_secret(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(params): Path<CapabilitySecretPath>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    // Parse agent ID
    let agent_id: AgentId = params.agent_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid agent ID: {}", e),
            }),
        )
    })?;

    // Verify agent exists and belongs to org
    let _agent = state
        .db
        .get_agent(org.org_id, agent_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Agent not found".to_string(),
                }),
            )
        })?;

    // Delete the secret
    let deleted = state
        .db
        .delete_capability_secret(agent_id.uuid(), &params.capability_id, &params.secret_name)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete capability secret: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Secret not found".to_string(),
            }),
        ))
    }
}

/// List capability secret names
///
/// Returns the names of all secrets configured for a capability on an agent.
/// Does not return the secret values.
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/capabilities/{capability_id}/secrets",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed, e.g., agt_...)"),
        ("capability_id" = String, Path, description = "Capability ID (e.g., web_search)")
    ),
    responses(
        (status = 200, description = "List of secret names", body = ListCapabilitySecretsResponse),
        (status = 400, description = "Invalid agent ID"),
        (status = 404, description = "Agent not found")
    ),
    tag = "capability-secrets"
)]
pub async fn list_secrets(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(params): Path<CapabilityPath>,
) -> Result<Json<ListCapabilitySecretsResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Parse agent ID
    let agent_id: AgentId = params.agent_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid agent ID: {}", e),
            }),
        )
    })?;

    // Verify agent exists and belongs to org
    let _agent = state
        .db
        .get_agent(org.org_id, agent_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Agent not found".to_string(),
                }),
            )
        })?;

    // List the secrets
    let secrets = state
        .db
        .list_capability_secrets(agent_id.uuid(), &params.capability_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list capability secrets: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;

    Ok(Json(ListCapabilitySecretsResponse { secrets }))
}

/// Check if a capability secret exists
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/capabilities/{capability_id}/secrets/{secret_name}/exists",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed, e.g., agt_...)"),
        ("capability_id" = String, Path, description = "Capability ID (e.g., web_search)"),
        ("secret_name" = String, Path, description = "Secret name (e.g., api_key)")
    ),
    responses(
        (status = 200, description = "Secret existence check", body = SecretExistsResponse),
        (status = 400, description = "Invalid agent ID"),
        (status = 404, description = "Agent not found")
    ),
    tag = "capability-secrets"
)]
pub async fn secret_exists(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(params): Path<CapabilitySecretPath>,
) -> Result<Json<SecretExistsResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Parse agent ID
    let agent_id: AgentId = params.agent_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid agent ID: {}", e),
            }),
        )
    })?;

    // Verify agent exists and belongs to org
    let _agent = state
        .db
        .get_agent(org.org_id, agent_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Agent not found".to_string(),
                }),
            )
        })?;

    // Check if secret exists
    let exists = state
        .db
        .has_capability_secret(agent_id.uuid(), &params.capability_id, &params.secret_name)
        .await
        .map_err(|e| {
            tracing::error!("Failed to check capability secret: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;

    Ok(Json(SecretExistsResponse { exists }))
}

// ============================================================================
// Router
// ============================================================================

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/agents/:agent_id/capabilities/:capability_id/secrets",
            get(list_secrets),
        )
        .route(
            "/v1/agents/:agent_id/capabilities/:capability_id/secrets/:secret_name",
            put(set_secret).delete(delete_secret),
        )
        .route(
            "/v1/agents/:agent_id/capabilities/:capability_id/secrets/:secret_name/exists",
            get(secret_exists),
        )
        .with_state(state)
}

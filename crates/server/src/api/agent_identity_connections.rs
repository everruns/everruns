// Agent identity connection HTTP routes.
// Sub-resource of agent identities: /v1/agent-identities/{identity_id}/connections/...
// Mirrors user_connections but scoped to an agent identity instead of a user.

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::agent_identity::AGENT_IDENTITY_MANAGE;
use crate::storage::models::CreateAgentIdentityConnectionRow;
use crate::storage::{EncryptionService, StorageBackend};
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use everruns_core::connection_provider::{ConnectionProviderRegistry, ConnectionType};
use everruns_core::{AgentIdentityId, Caller, evaluate_policies_with};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::common::ErrorResponse;

/// App state for identity connection routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub auth: AuthState,
    pub connection_providers: ConnectionProviderRegistry,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        auth: AuthState,
        connection_providers: ConnectionProviderRegistry,
    ) -> Self {
        Self {
            db,
            encryption,
            auth,
            connection_providers,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

// ============================================================================
// Response / Request Types
// ============================================================================

#[derive(Debug, Serialize)]
pub struct ConnectionResponse {
    pub provider: String,
    pub connection_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<String>,
    pub connected_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyConnectionRequest {
    pub api_key: String,
}

// ============================================================================
// Routes
// ============================================================================

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/agent-identities/{identity_id}/connections",
            get(list_connections),
        )
        .route(
            "/v1/agent-identities/{identity_id}/connections/{provider}",
            post(create_api_key_connection).delete(delete_connection),
        )
        .route(
            "/v1/agent-identities/{identity_id}/connections/{provider}/verify",
            post(verify_connection),
        )
        .with_state(state)
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /v1/agent-identities/:identity_id/connections
pub async fn list_connections(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(identity_id): Path<String>,
) -> Result<Json<Vec<ConnectionResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let identity_id: AgentIdentityId = identity_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid identity ID: {}", e))
            .into_response(StatusCode::BAD_REQUEST)
    })?;

    let caller = Caller::from(&org);
    evaluate_policies_with(
        state.auth.permission_resolver.as_ref(),
        &caller,
        &[&AGENT_IDENTITY_MANAGE],
    );

    // Verify identity exists in this org
    state
        .db
        .get_agent_identity(caller.org_id, identity_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent identity: {}", e);
            ErrorResponse::new("Internal error".to_string())
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .ok_or_else(|| {
            ErrorResponse::new("Agent identity not found".to_string())
                .into_response(StatusCode::NOT_FOUND)
        })?;

    let rows = state
        .db
        .list_agent_identity_connections(identity_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list identity connections: {}", e);
            ErrorResponse::new("Internal error".to_string())
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;

    let connections = rows
        .into_iter()
        .map(|r| ConnectionResponse {
            provider: r.provider,
            connection_type: r.connection_type,
            provider_username: r.provider_username,
            scopes: r.scopes,
            connected_at: r.created_at,
        })
        .collect();

    Ok(Json(connections))
}

/// POST /v1/agent-identities/:identity_id/connections/:provider
pub async fn create_api_key_connection(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((identity_id, provider_id)): Path<(String, String)>,
    Json(body): Json<CreateApiKeyConnectionRequest>,
) -> Result<(StatusCode, Json<ConnectionResponse>), (StatusCode, Json<ErrorResponse>)> {
    let identity_id: AgentIdentityId = identity_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid identity ID: {}", e))
            .into_response(StatusCode::BAD_REQUEST)
    })?;

    let caller = Caller::from(&org);
    evaluate_policies_with(
        state.auth.permission_resolver.as_ref(),
        &caller,
        &[&AGENT_IDENTITY_MANAGE],
    );

    // Verify identity exists
    state
        .db
        .get_agent_identity(caller.org_id, identity_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent identity: {}", e);
            ErrorResponse::new("Internal error".to_string())
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .ok_or_else(|| {
            ErrorResponse::new("Agent identity not found".to_string())
                .into_response(StatusCode::NOT_FOUND)
        })?;

    let provider = state
        .connection_providers
        .get(&provider_id)
        .ok_or_else(|| {
            ErrorResponse::new(format!("Unknown connection provider: {provider_id}"))
                .into_response(StatusCode::NOT_FOUND)
        })?;

    if provider.connection_type() != ConnectionType::ApiKey {
        return Err(ErrorResponse::new(format!(
            "Provider '{provider_id}' uses OAuth, not API key"
        ))
        .into_response(StatusCode::BAD_REQUEST));
    }

    let encryption = state.encryption.as_ref().ok_or_else(|| {
        ErrorResponse::new("Encryption not configured".to_string())
            .into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;

    // Validate the API key
    let validation = provider.validate(&body.api_key).await.map_err(|e| {
        ErrorResponse::new(format!("API key validation failed: {e}"))
            .into_response(StatusCode::BAD_REQUEST)
    })?;

    // Encrypt and store
    let access_token_encrypted = encryption.encrypt_string(&body.api_key).map_err(|e| {
        tracing::error!("Failed to encrypt API key: {}", e);
        ErrorResponse::new("Failed to store connection".to_string())
            .into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;

    let row = state
        .db
        .upsert_agent_identity_connection(CreateAgentIdentityConnectionRow {
            agent_identity_id: identity_id,
            provider: provider_id,
            connection_type: "api_key".to_string(),
            provider_user_id: None,
            provider_username: validation.provider_username.clone(),
            access_token_encrypted: Some(access_token_encrypted),
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            installation_id: None,
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to store identity connection: {}", e);
            ErrorResponse::new("Failed to store connection".to_string())
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;

    Ok((
        StatusCode::CREATED,
        Json(ConnectionResponse {
            provider: row.provider,
            connection_type: row.connection_type,
            provider_username: row.provider_username,
            scopes: row.scopes,
            connected_at: row.created_at,
        }),
    ))
}

/// DELETE /v1/agent-identities/:identity_id/connections/:provider
pub async fn delete_connection(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((identity_id, provider)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let identity_id: AgentIdentityId = identity_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid identity ID: {}", e))
            .into_response(StatusCode::BAD_REQUEST)
    })?;

    let caller = Caller::from(&org);
    evaluate_policies_with(
        state.auth.permission_resolver.as_ref(),
        &caller,
        &[&AGENT_IDENTITY_MANAGE],
    );

    // Verify identity exists
    state
        .db
        .get_agent_identity(caller.org_id, identity_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent identity: {}", e);
            ErrorResponse::new("Internal error".to_string())
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .ok_or_else(|| {
            ErrorResponse::new("Agent identity not found".to_string())
                .into_response(StatusCode::NOT_FOUND)
        })?;

    let deleted = state
        .db
        .delete_agent_identity_connection(identity_id, &provider)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete identity connection: {}", e);
            ErrorResponse::new("Internal error".to_string())
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::new("Connection not found".to_string())
            .into_response(StatusCode::NOT_FOUND))
    }
}

/// POST /v1/agent-identities/:identity_id/connections/:provider/verify
pub async fn verify_connection(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((identity_id, provider_id)): Path<(String, String)>,
) -> Result<Json<VerifyConnectionResponse>, (StatusCode, Json<ErrorResponse>)> {
    let identity_id: AgentIdentityId = identity_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid identity ID: {}", e))
            .into_response(StatusCode::BAD_REQUEST)
    })?;

    let caller = Caller::from(&org);
    evaluate_policies_with(
        state.auth.permission_resolver.as_ref(),
        &caller,
        &[&AGENT_IDENTITY_MANAGE],
    );

    // Verify identity exists
    state
        .db
        .get_agent_identity(caller.org_id, identity_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent identity: {}", e);
            ErrorResponse::new("Internal error".to_string())
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .ok_or_else(|| {
            ErrorResponse::new("Agent identity not found".to_string())
                .into_response(StatusCode::NOT_FOUND)
        })?;

    let row = state
        .db
        .get_agent_identity_connection(identity_id, &provider_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to look up connection for verify: {}", e);
            ErrorResponse::new("Internal error".to_string())
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?
        .ok_or_else(|| {
            ErrorResponse::new(format!("No connection found for provider: {provider_id}"))
                .into_response(StatusCode::NOT_FOUND)
        })?;

    if row.connection_type != "api_key" {
        return Err(ErrorResponse::new(format!(
            "Provider '{provider_id}' does not support API key verification"
        ))
        .into_response(StatusCode::BAD_REQUEST));
    }

    let encrypted = row.access_token_encrypted.ok_or_else(|| {
        ErrorResponse::new("No stored credential found".to_string())
            .into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;

    let encryption = state.encryption.as_ref().ok_or_else(|| {
        ErrorResponse::new("Encryption not configured".to_string())
            .into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;

    let api_key = encryption.decrypt_to_string(&encrypted).map_err(|e| {
        tracing::error!("Failed to decrypt stored credential: {}", e);
        ErrorResponse::new("Failed to verify connection".to_string())
            .into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;

    let provider = state
        .connection_providers
        .get(&provider_id)
        .ok_or_else(|| {
            ErrorResponse::new(format!("Unknown connection provider: {provider_id}"))
                .into_response(StatusCode::NOT_FOUND)
        })?;

    match provider.validate(&api_key).await {
        Ok(_) => Ok(Json(VerifyConnectionResponse {
            valid: true,
            error: None,
        })),
        Err(e) => Ok(Json(VerifyConnectionResponse {
            valid: false,
            error: Some(e.to_string()),
        })),
    }
}

#[derive(Debug, Serialize)]
pub struct VerifyConnectionResponse {
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

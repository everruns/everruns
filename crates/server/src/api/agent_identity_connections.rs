// Agent identity connection HTTP routes.
// Sub-resource of agent identities: /v1/agent-identities/{identity_id}/connections/...
// Mirrors user_connections but scoped to an agent identity instead of a user.
//
// THREAT[TM-AUTHZ-020]: All handlers enforce AGENT_IDENTITY_MANAGE policy before
// any data access. Identity org-scoping prevents cross-tenant access.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::agent_identities::AGENT_IDENTITY_MANAGE;
use crate::storage::models::CreateAgentIdentityConnectionRow;
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use everruns_core::connection_provider::{ConnectionProviderRegistry, ConnectionType};
use everruns_core::{AgentIdentityId, Caller};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::common::{ErrorResponse, impl_auth_state};

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

impl_auth_state!(AppState);

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
    #[serde(flatten)]
    pub extra_fields: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct VerifyConnectionResponse {
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
// Helpers
// ============================================================================

/// Enforce AGENT_IDENTITY_MANAGE policy. Returns 403 on failure.
fn enforce_manage_policy(
    state: &AppState,
    caller: &Caller,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    AGENT_IDENTITY_MANAGE
        .evaluate_with(state.auth.permission_resolver.as_ref(), caller)
        .map_err(|_| {
            ErrorResponse::new("Permission denied".to_string()).into_response(StatusCode::FORBIDDEN)
        })
}

/// Parse identity ID, enforce policy, and verify the identity exists in the caller's org.
async fn resolve_identity(
    state: &AppState,
    org: &ResolvedOrg,
    raw_id: &str,
) -> Result<(Caller, AgentIdentityId), (StatusCode, Json<ErrorResponse>)> {
    let identity_id: AgentIdentityId = raw_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid identity ID: {}", e))
            .into_response(StatusCode::BAD_REQUEST)
    })?;

    let caller = Caller::from(org);
    enforce_manage_policy(state, &caller)?;

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

    Ok((caller, identity_id))
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
    let (_, identity_id) = resolve_identity(&state, &org, &identity_id).await?;

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
    let (_, identity_id) = resolve_identity(&state, &org, &identity_id).await?;

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

    // Build form fields map for validation
    let mut fields = std::collections::HashMap::new();
    fields.insert("api_key".to_string(), body.api_key.clone());
    for (key, value) in &body.extra_fields {
        if let Some(s) = value.as_str() {
            fields.insert(key.clone(), s.to_string());
        }
    }

    // Validate all fields via the provider
    let validation = provider.validate_fields(&fields).await.map_err(|e| {
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
            provider_metadata: validation.provider_metadata.clone(),
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
    let (_, identity_id) = resolve_identity(&state, &org, &identity_id).await?;

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
    let (_, identity_id) = resolve_identity(&state, &org, &identity_id).await?;

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

    // Build fields map from stored credential + metadata for full validation
    let mut fields = std::collections::HashMap::new();
    fields.insert("api_key".to_string(), api_key);
    if let Some(meta) = row.provider_metadata.as_ref()
        && let Some(obj) = meta.as_object()
    {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                fields.insert(k.clone(), s.to_string());
            }
        }
    }

    match provider.validate_fields(&fields).await {
        Ok(_) => Ok(Json(VerifyConnectionResponse {
            valid: true,
            error: None,
        })),
        Err(e) => {
            tracing::error!("Connection verification failed: {e}");
            Ok(Json(VerifyConnectionResponse {
                valid: false,
                error: Some("Connection verification failed".to_string()),
            }))
        }
    }
}

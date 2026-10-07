// Virtual user connection HTTP routes.
// Sub-resource of virtual users: /v1/virtual-users/{identity_id}/connections/...
// Mirrors user_connections but scoped to a virtual user instead of a user.
//
// THREAT[TM-TOOL-041]: Private end-user grants require self authority; service
// grants require management policy. Every target is scoped to the authenticated org.

use crate::auth::AuthState;
use crate::auth::runtime::RuntimeAccount;
use crate::kernel_imports::contracts::typed_id::VirtualUserId;
use crate::storage::models::CreateVirtualUserConnectionRow;
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_contracts::connector::{ConnectorRegistry, ConnectorType};
use std::sync::Arc;

use super::common::{ErrorResponse, impl_auth_state};

/// App state for identity connection routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub auth: AuthState,
    pub connectors: ConnectorRegistry,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        auth: AuthState,
        connectors: ConnectorRegistry,
    ) -> Self {
        Self {
            db,
            encryption,
            auth,
            connectors,
        }
    }
}

impl_auth_state!(AppState);

// ============================================================================
// Response / Request Types
// ============================================================================

pub use super::user_connections::{
    ConnectionResponse, CreateApiKeyConnectionRequest, VerifyConnectionResponse,
};

// ============================================================================
// Routes
// ============================================================================

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/virtual-users/{identity_id}/connections",
            get(list_connections),
        )
        .route(
            "/v1/virtual-users/{identity_id}/connections/{provider}",
            post(create_api_key_connection).delete(delete_connection),
        )
        .route(
            "/v1/virtual-users/{identity_id}/connections/{provider}/verify",
            post(verify_connection),
        )
        .with_state(state.clone())
        .merge(super::github_apps::routes(
            super::github_apps::AppState::from_connections(&state),
        ))
}

// ============================================================================
// Helpers
// ============================================================================

/// Parse identity ID, enforce policy, and verify the identity exists in the caller's org.
async fn resolve_identity(
    state: &AppState,
    org: &RuntimeAccount,
    raw_id: &str,
) -> Result<VirtualUserId, (StatusCode, Json<ErrorResponse>)> {
    org.connection_target(&state.db, state.auth.permission_resolver.as_ref(), raw_id)
        .await
        .map_err(Into::into)
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /v1/virtual-users/:identity_id/connections
#[utoipa::path(summary = "List connections.", get, path = "/v1/virtual-users/{identity_id}/connections", params(("identity_id" = String, Path)),  responses((status = 200, description = "Success", body = Vec<ConnectionResponse>), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied"), (status = 404, description = "Virtual user not found")), tag = "virtual-users")]
pub async fn list_connections(
    org: RuntimeAccount,
    State(state): State<AppState>,
    Path(identity_id): Path<String>,
) -> Result<Json<Vec<ConnectionResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let identity_id = resolve_identity(&state, &org, &identity_id).await?;

    let rows = state
        .db
        .list_virtual_user_connections(identity_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to list identity connections");
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

/// POST /v1/virtual-users/:identity_id/connections/:provider
#[utoipa::path(summary = "Create api key connection.", post, path = "/v1/virtual-users/{identity_id}/connections/{provider}", params(("identity_id" = String, Path),("provider" = String, Path)), request_body = CreateApiKeyConnectionRequest, responses((status = 201, description = "Success", body = ConnectionResponse), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied"), (status = 404, description = "Virtual user not found")), tag = "virtual-users")]
pub async fn create_api_key_connection(
    org: RuntimeAccount,
    State(state): State<AppState>,
    Path((identity_id, provider_id)): Path<(String, String)>,
    Json(body): Json<CreateApiKeyConnectionRequest>,
) -> Result<(StatusCode, Json<ConnectionResponse>), (StatusCode, Json<ErrorResponse>)> {
    let identity_id = resolve_identity(&state, &org, &identity_id).await?;

    let provider = state.connectors.get(&provider_id).ok_or_else(|| {
        ErrorResponse::new(format!("Unknown connector: {provider_id}"))
            .into_response(StatusCode::NOT_FOUND)
    })?;
    // Flag-gated connectors are offered only to organisations where the flag
    // is effective; a storage failure fails closed.
    let connector_enabled = crate::services::org_feature_flags::resolve_org_feature_flags(
        &state.db,
        org.org_id,
        &state.auth.feature_flag_policy,
    )
    .await
    .is_ok_and(|flags| flags.is_connector_enabled(&provider_id));
    if !connector_enabled {
        return Err(
            ErrorResponse::new(format!("Unknown connector: {provider_id}"))
                .into_response(StatusCode::NOT_FOUND),
        );
    }

    if provider.connection_type() != ConnectorType::ApiKey {
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
        tracing::error!(error = %e, "Failed to encrypt API key");
        ErrorResponse::new("Failed to store connection".to_string())
            .into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;

    let row = state
        .db
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: identity_id,
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
            tracing::error!(error = %e, "Failed to store identity connection");
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

/// DELETE /v1/virtual-users/:identity_id/connections/:provider
#[utoipa::path(summary = "Delete connection.", delete, path = "/v1/virtual-users/{identity_id}/connections/{provider}", params(("identity_id" = String, Path),("provider" = String, Path)),  responses((status = 204, description = "Success"), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied"), (status = 404, description = "Virtual user not found")), tag = "virtual-users")]
pub async fn delete_connection(
    org: RuntimeAccount,
    State(state): State<AppState>,
    Path((identity_id, provider)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let identity_id = resolve_identity(&state, &org, &identity_id).await?;

    let deleted = state
        .db
        .delete_virtual_user_connection(identity_id, &provider)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to delete identity connection");
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

/// POST /v1/virtual-users/:identity_id/connections/:provider/verify
#[utoipa::path(summary = "Verify connection.", post, path = "/v1/virtual-users/{identity_id}/connections/{provider}/verify", params(("identity_id" = String, Path),("provider" = String, Path)),  responses((status = 200, description = "Success", body = VerifyConnectionResponse), (status = 401, description = "Authentication required"), (status = 403, description = "Permission denied"), (status = 404, description = "Virtual user not found")), tag = "virtual-users")]
pub async fn verify_connection(
    org: RuntimeAccount,
    State(state): State<AppState>,
    Path((identity_id, provider_id)): Path<(String, String)>,
) -> Result<Json<VerifyConnectionResponse>, (StatusCode, Json<ErrorResponse>)> {
    let identity_id = resolve_identity(&state, &org, &identity_id).await?;

    let row = state
        .db
        .get_virtual_user_connection(identity_id, &provider_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to look up connection for verify");
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
        tracing::error!(error = %e, "Failed to decrypt stored credential");
        ErrorResponse::new("Failed to verify connection".to_string())
            .into_response(StatusCode::INTERNAL_SERVER_ERROR)
    })?;

    let provider = state.connectors.get(&provider_id).ok_or_else(|| {
        ErrorResponse::new(format!("Unknown connector: {provider_id}"))
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
            tracing::error!(error = %e, "Connection verification failed");
            Ok(Json(VerifyConnectionResponse {
                valid: false,
                error: Some("Connection verification failed".to_string()),
            }))
        }
    }
}

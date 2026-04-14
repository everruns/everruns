// API key CRUD routes — auth-provider-agnostic.
// Decision: Extracted from auth_routes() so all AuthBackend implementations
// (builtin, PropelAuth, etc.) get API key management without reimplementing it.
// Follows the same pattern as cli_auth_routes().

use axum::{
    Json, Router,
    extract::{FromRef, Path, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get},
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::api_key::generate_api_key;
use super::audit;
use super::middleware::{AuthError, AuthMethod, AuthState, AuthUser};
use crate::api::common::ListResponse;
use crate::server::ResourceLimitsConfig;
use crate::storage::StorageBackend;
use crate::storage::models::CreateApiKeyRow;

/// State for API key CRUD routes — decoupled from any specific AuthBackend.
///
/// Embedders construct this with their own DB/auth and mount via `api_key_routes()`.
#[derive(Clone)]
pub struct ApiKeyState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
    pub resource_limits: ResourceLimitsConfig,
}

/// Enable AuthUser extractor when ApiKeyState is the route state.
impl FromRef<ApiKeyState> for AuthState {
    fn from_ref(state: &ApiKeyState) -> Self {
        state.auth.clone()
    }
}

/// API key response (shown only once at creation)
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyResponse {
    pub id: String,
    pub name: String,
    pub key: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<String>,
    pub created_at: String,
}

/// API key list item (without full key)
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiKeyListItem {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub created_at: String,
}

/// Create API key request
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateApiKeyRequest {
    pub name: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Expiration in days (optional)
    pub expires_in_days: Option<i64>,
}

/// Create API key CRUD routes
pub fn api_key_routes(state: ApiKeyState) -> Router {
    Router::new()
        .route("/v1/auth/api-keys", get(list_api_keys).post(create_api_key))
        .route("/v1/auth/api-keys/{key_id}", delete(delete_api_key))
        .with_state(state)
}

/// GET /v1/auth/api-keys - List API keys for current user
async fn list_api_keys(
    State(state): State<ApiKeyState>,
    user: AuthUser,
) -> Result<Json<ListResponse<ApiKeyListItem>>, AuthError> {
    let keys = state
        .db
        .list_api_keys_for_user(user.id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list API keys: {}", e);
            AuthError::unauthorized("Failed to list API keys")
        })?;

    let items: Vec<ApiKeyListItem> = keys
        .into_iter()
        .map(|k| {
            let scopes: Vec<String> = serde_json::from_value(k.scopes).unwrap_or_default();
            ApiKeyListItem {
                id: k.id.to_string(),
                name: k.name,
                key_prefix: k.key_prefix,
                scopes,
                expires_at: k.expires_at.map(|t| t.to_rfc3339()),
                last_used_at: k.last_used_at.map(|t| t.to_rfc3339()),
                created_at: k.created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(ListResponse::new(items)))
}

/// POST /v1/auth/api-keys - Create a new API key
///
/// API keys are user-scoped (not org-scoped). The key inherits access to all
/// organizations the user belongs to. Org context is resolved per-request via
/// `X-Org-Id` header or `everruns_org` cookie.
/// Cannot be called with API key authentication (must use session auth).
async fn create_api_key(
    State(state): State<ApiKeyState>,
    headers: HeaderMap,
    user: AuthUser,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<ApiKeyResponse>), AuthError> {
    // Cannot create API key using API key auth
    if user.auth_method == AuthMethod::ApiKey {
        return Err(AuthError::forbidden(
            "Cannot create API key using API key authentication",
        ));
    }

    // Enforce API key limit per user
    let key_count = state
        .db
        .count_api_keys_for_user(user.id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to count API keys: {e}");
            AuthError {
                error: "Internal server error".to_string(),
                status: StatusCode::INTERNAL_SERVER_ERROR,
            }
        })?;
    if key_count >= state.resource_limits.max_api_keys_per_user {
        return Err(AuthError {
            error: format!(
                "API key limit reached (max {})",
                state.resource_limits.max_api_keys_per_user
            ),
            status: StatusCode::CONFLICT,
        });
    }

    let generated = generate_api_key();

    let scopes = if req.scopes.is_empty() {
        vec!["*".to_string()]
    } else {
        req.scopes
    };

    let expires_at = req
        .expires_in_days
        .map(|days| Utc::now() + Duration::days(days));

    let key_row = state
        .db
        .create_api_key(CreateApiKeyRow {
            user_id: user.id,
            name: req.name.clone(),
            key_hash: generated.key_hash.clone(),
            key_prefix: generated.key_prefix.clone(),
            scopes: scopes.clone(),
            expires_at,
            metadata: serde_json::json!({"source": "web_ui"}),
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to create API key: {}", e);
            AuthError::unauthorized("Failed to create API key")
        })?;

    // Use the user's first org for audit context (API key CRUD is user-level,
    // not org-scoped, but audit logs are org-partitioned).
    let audit_org_id = user
        .organizations
        .first()
        .map(|o| o.org_id)
        .unwrap_or(everruns_core::DEFAULT_ORG_ID);
    audit::emit(
        state.db.clone(),
        audit_org_id,
        Some(user.id),
        "auth.api_key.created",
        audit::client_ip(&headers),
        serde_json::json!({"key_id": key_row.id.to_string(), "name": req.name}),
    );

    Ok((
        StatusCode::CREATED,
        Json(ApiKeyResponse {
            id: key_row.id.to_string(),
            name: key_row.name,
            key: generated.key, // Full key shown only once!
            key_prefix: key_row.key_prefix,
            scopes,
            expires_at: key_row.expires_at.map(|t| t.to_rfc3339()),
            created_at: key_row.created_at.to_rfc3339(),
        }),
    ))
}

/// DELETE /v1/auth/api-keys/:key_id - Delete an API key
async fn delete_api_key(
    State(state): State<ApiKeyState>,
    headers: HeaderMap,
    user: AuthUser,
    Path(key_id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    let deleted = state
        .db
        .delete_api_key(key_id, user.id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete API key: {}", e);
            AuthError::unauthorized("Failed to delete API key")
        })?;

    if deleted {
        // Notify the auth backend so it can invalidate caches.
        state.auth.backend.on_api_key_deleted();

        let audit_org_id = user
            .organizations
            .first()
            .map(|o| o.org_id)
            .unwrap_or(everruns_core::DEFAULT_ORG_ID);
        audit::emit(
            state.db.clone(),
            audit_org_id,
            Some(user.id),
            "auth.api_key.deleted",
            audit::client_ip(&headers),
            serde_json::json!({"key_id": key_id.to_string()}),
        );
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AuthError::unauthorized("API key not found"))
    }
}

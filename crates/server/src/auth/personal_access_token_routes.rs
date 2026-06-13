// Personal access token CRUD routes — auth-provider-agnostic.
// Decision: Extracted from auth_routes() so all AuthBackend implementations
// (builtin, PropelAuth, etc.) get token management without reimplementing it.
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

use super::audit;
use super::middleware::{AuthError, AuthMethod, AuthState, AuthUser};
use super::personal_access_token::generate_personal_access_token;
use crate::api::common::ListResponse;
use crate::server::ResourceLimitsConfig;
use crate::storage::StorageBackend;
use crate::storage::models::CreatePersonalAccessTokenRow;

/// State for personal access token CRUD routes — decoupled from any specific AuthBackend.
///
/// Embedders construct this with their own DB/auth and mount via `personal_access_token_routes()`.
#[derive(Clone)]
pub struct PersonalAccessTokenState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
    pub resource_limits: ResourceLimitsConfig,
}

/// Enable AuthUser extractor when PersonalAccessTokenState is the route state.
impl FromRef<PersonalAccessTokenState> for AuthState {
    fn from_ref(state: &PersonalAccessTokenState) -> Self {
        state.auth.clone()
    }
}

/// Personal access token response (full token shown only once at creation)
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonalAccessTokenResponse {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    pub token: String,
    pub token_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<String>,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: String,
}

/// Personal access token list item (without full token)
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonalAccessTokenListItem {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    pub token_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: String,
}

/// Create personal access token request
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePersonalAccessTokenRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Expiration in days (optional)
    pub expires_in_days: Option<i64>,
}

/// Create personal access token CRUD routes
pub fn personal_access_token_routes(state: PersonalAccessTokenState) -> Router {
    Router::new()
        .route(
            "/v1/auth/personal-access-tokens",
            get(list_personal_access_tokens).post(create_personal_access_token),
        )
        .route(
            "/v1/auth/personal-access-tokens/{token_id}",
            delete(delete_personal_access_token),
        )
        .with_state(state)
}

/// GET /v1/auth/personal-access-tokens - List personal access tokens for current user
async fn list_personal_access_tokens(
    State(state): State<PersonalAccessTokenState>,
    user: AuthUser,
) -> Result<Json<ListResponse<PersonalAccessTokenListItem>>, AuthError> {
    let tokens = state
        .db
        .list_personal_access_tokens_for_user(user.id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list personal access tokens: {}", e);
            AuthError::internal("Failed to list personal access tokens")
        })?;

    let items: Vec<PersonalAccessTokenListItem> = tokens
        .into_iter()
        .map(|t| {
            let scopes: Vec<String> = serde_json::from_value(t.scopes).unwrap_or_default();
            PersonalAccessTokenListItem {
                id: t.id.to_string(),
                name: t.name,
                token_prefix: t.token_prefix,
                scopes,
                expires_at: t.expires_at.map(|t| t.to_rfc3339()),
                last_used_at: t.last_used_at.map(|t| t.to_rfc3339()),
                created_at: t.created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(ListResponse::new(items)))
}

/// POST /v1/auth/personal-access-tokens - Create a new personal access token
///
/// Personal access tokens are user-scoped (not org-scoped). The token inherits
/// access to all organizations the user belongs to. Org context is resolved
/// per-request via `X-Org-Id` header or `everruns_org` cookie.
/// Cannot be called with personal access token authentication (must use session auth).
async fn create_personal_access_token(
    State(state): State<PersonalAccessTokenState>,
    headers: HeaderMap,
    user: AuthUser,
    Json(req): Json<CreatePersonalAccessTokenRequest>,
) -> Result<(StatusCode, Json<PersonalAccessTokenResponse>), AuthError> {
    // Cannot create a personal access token using personal access token auth
    if user.auth_method == AuthMethod::PersonalAccessToken {
        return Err(AuthError::forbidden(
            "Cannot create a personal access token using personal access token authentication",
        ));
    }

    let token_count = state
        .db
        .list_personal_access_tokens_for_user(user.id)
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to count personal access tokens for user {}: {}",
                user.id,
                e
            );
            AuthError::internal("Failed to create personal access token")
        })?;
    if token_count.len() as i64 >= state.resource_limits.max_personal_access_tokens_per_user {
        return Err(AuthError {
            error: format!(
                "Personal access token limit reached: maximum {} tokens per user",
                state.resource_limits.max_personal_access_tokens_per_user
            ),
            status: StatusCode::CONFLICT,
            code: Some("personal_access_token_limit_reached"),
        });
    }

    let generated = generate_personal_access_token();

    let scopes = if req.scopes.is_empty() {
        vec!["*".to_string()]
    } else {
        req.scopes
    };

    let expires_at = req
        .expires_in_days
        .map(|days| -> Result<_, AuthError> {
            if days <= 0 || days > 3650 {
                return Err(AuthError {
                    error: "expires_in_days must be between 1 and 3650".to_string(),
                    status: StatusCode::BAD_REQUEST,
                    code: Some("invalid_expires_in_days"),
                });
            }
            Duration::try_days(days)
                .and_then(|d| Utc::now().checked_add_signed(d))
                .ok_or_else(|| AuthError {
                    error: "expires_in_days out of range".to_string(),
                    status: StatusCode::BAD_REQUEST,
                    code: Some("invalid_expires_in_days"),
                })
        })
        .transpose()?;

    let token_row = state
        .db
        .create_personal_access_token(CreatePersonalAccessTokenRow {
            user_id: user.id,
            name: req.name.clone(),
            token_hash: generated.token_hash.clone(),
            token_prefix: generated.token_prefix.clone(),
            scopes: scopes.clone(),
            expires_at,
            metadata: serde_json::json!({"source": "web_ui"}),
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to create personal access token: {}", e);
            AuthError::internal("Failed to create personal access token")
        })?;

    // Use the user's first org for audit context (token CRUD is user-level,
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
        "auth.personal_access_token.created",
        audit::client_ip(&headers),
        serde_json::json!({"token_id": token_row.id.to_string(), "name": req.name}),
    );

    Ok((
        StatusCode::CREATED,
        Json(PersonalAccessTokenResponse {
            id: token_row.id.to_string(),
            name: token_row.name,
            token: generated.token, // Full token shown only once!
            token_prefix: token_row.token_prefix,
            scopes,
            expires_at: token_row.expires_at.map(|t| t.to_rfc3339()),
            created_at: token_row.created_at.to_rfc3339(),
        }),
    ))
}

/// DELETE /v1/auth/personal-access-tokens/:token_id - Delete a personal access token
async fn delete_personal_access_token(
    State(state): State<PersonalAccessTokenState>,
    headers: HeaderMap,
    user: AuthUser,
    Path(token_id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    let deleted = state
        .db
        .delete_personal_access_token(token_id, user.id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete personal access token: {}", e);
            AuthError::internal("Failed to delete personal access token")
        })?;

    if deleted {
        // Notify the auth backend so it can invalidate caches.
        state.auth.backend.on_personal_access_token_deleted();

        let audit_org_id = user
            .organizations
            .first()
            .map(|o| o.org_id)
            .unwrap_or(everruns_core::DEFAULT_ORG_ID);
        audit::emit(
            state.db.clone(),
            audit_org_id,
            Some(user.id),
            "auth.personal_access_token.deleted",
            audit::client_ip(&headers),
            serde_json::json!({"token_id": token_id.to_string()}),
        );
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AuthError::not_found("Personal access token not found"))
    }
}

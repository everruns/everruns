// User Connections API routes
// Decision: User-scoped (not org-scoped) — token represents user's GitHub identity
// Decision: GitHub App installation flow replaces OAuth App for repo access

use crate::auth::config::AuthConfig;
use crate::auth::middleware::{AuthState, AuthUser};
use crate::auth::oauth::GitHubAppService;
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Redirect,
    routing::{delete, get},
};
use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use axum::extract::FromRef;

use crate::storage::models::CreateUserConnectionRow;

/// App state for user connections routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub auth: AuthState,
    pub auth_config: AuthConfig,
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Connection info returned in API responses (never includes token)
#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectionResponse {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<String>,
    pub connected_at: DateTime<Utc>,
}

/// GitHub App installation callback query params
#[derive(Debug, Deserialize)]
pub struct GitHubInstallationCallbackQuery {
    pub installation_id: i64,
    #[allow(dead_code)]
    pub setup_action: Option<String>,
    #[allow(dead_code)]
    pub state: Option<String>,
}

/// Create user connections routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/user/connections", get(list_connections))
        .route("/v1/user/connections/{provider}", delete(delete_connection))
        .route(
            "/v1/user/connections/github/authorize",
            get(github_authorize),
        )
        .route("/v1/user/connections/github/callback", get(github_callback))
        .with_state(state)
}

/// GET /v1/user/connections — List user's connected accounts
pub async fn list_connections(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<ConnectionResponse>>, StatusCode> {
    let rows = state.db.list_user_connections(auth.id).await.map_err(|e| {
        tracing::error!("Failed to list user connections: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let connections = rows
        .into_iter()
        .map(|r| ConnectionResponse {
            provider: r.provider,
            provider_username: r.provider_username,
            scopes: r.scopes,
            connected_at: r.created_at,
        })
        .collect();

    Ok(Json(connections))
}

/// DELETE /v1/user/connections/:provider — Disconnect
pub async fn delete_connection(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(provider): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let deleted = state
        .db
        .delete_user_connection(auth.id, &provider)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete connection: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// GET /v1/user/connections/github/authorize — Redirect to GitHub App installation
pub async fn github_authorize(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(_params): Query<std::collections::HashMap<String, String>>,
) -> Result<Redirect, (StatusCode, String)> {
    let config = state
        .auth_config
        .github_connection
        .as_ref()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "GitHub App not configured".to_string(),
            )
        })?;

    let service = GitHubAppService::new(config);

    // Generate state for CSRF protection
    let mut rng = rand::rng();
    let bytes: [u8; 16] = rng.random();
    let install_state = hex::encode(bytes);

    // TODO: Store state in session/cookie for validation in callback
    let auth_url = service.installation_url(&install_state);
    Ok(Redirect::to(&auth_url.url))
}

/// GET /v1/user/connections/github/callback — GitHub App installation callback
///
/// After user installs the GitHub App on their repos, GitHub redirects here
/// with the installation_id. We verify the installation and store the ID.
pub async fn github_callback(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<GitHubInstallationCallbackQuery>,
) -> Result<Redirect, (StatusCode, String)> {
    let config = state
        .auth_config
        .github_connection
        .as_ref()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "GitHub App not configured".to_string(),
            )
        })?;

    let service = GitHubAppService::new(config);

    // Verify the installation exists and get account details
    let result = service
        .verify_installation(query.installation_id)
        .await
        .map_err(|e| {
            tracing::error!("GitHub App installation verification failed: {}", e);
            (
                StatusCode::BAD_REQUEST,
                "GitHub App installation verification failed".to_string(),
            )
        })?;

    // Store installation_id (no OAuth token needed — tokens minted on demand)
    state
        .db
        .upsert_user_connection(CreateUserConnectionRow {
            user_id: auth.id,
            provider: "github".to_string(),
            provider_user_id: Some(result.account_id),
            provider_username: Some(result.account_login),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: Some(result.permissions),
            expires_at: None,
            installation_id: Some(result.installation_id),
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to store GitHub App installation: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to store connection".to_string(),
            )
        })?;

    // Redirect back to settings page
    let frontend_url = state.auth_config.frontend_url.trim_end_matches('/');
    Ok(Redirect::to(&format!(
        "{}/settings/connections?connected=github",
        frontend_url
    )))
}

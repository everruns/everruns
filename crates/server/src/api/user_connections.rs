// User Connections API routes
// Decision: User-scoped (not org-scoped) — token represents user's GitHub identity
// Decision: Separate GitHub OAuth App from login (different scopes: repo vs profile)

use crate::auth::config::AuthConfig;
use crate::auth::middleware::{AuthState, AuthUser};
use crate::auth::oauth::GitHubConnectionService;
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Redirect,
    routing::{delete, get, post},
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

/// Request to add a connection via manual token
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateConnectionRequest {
    pub provider: String,
    pub access_token: String,
}

/// OAuth callback query params
#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: String,
    #[allow(dead_code)]
    pub state: String,
}

/// Create user connections routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/user/connections", get(list_connections))
        .route("/v1/user/connections", post(create_connection))
        .route(
            "/v1/user/connections/{provider}",
            delete(delete_connection),
        )
        .route(
            "/v1/user/connections/github/authorize",
            get(github_authorize),
        )
        .route(
            "/v1/user/connections/github/callback",
            get(github_callback),
        )
        .with_state(state)
}

/// GET /v1/user/connections — List user's connected accounts
pub async fn list_connections(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<ConnectionResponse>>, StatusCode> {
    let rows = state
        .db
        .list_user_connections(auth.user_id)
        .await
        .map_err(|e| {
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

/// POST /v1/user/connections — Add connection via manual PAT
pub async fn create_connection(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateConnectionRequest>,
) -> Result<(StatusCode, Json<ConnectionResponse>), (StatusCode, String)> {
    if req.provider != "github" {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Unsupported provider: {}", req.provider),
        ));
    }

    let encryption = state.encryption.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured".to_string(),
        )
    })?;

    // Validate token against GitHub API
    let client = reqwest::Client::new();
    let user_info: serde_json::Value = client
        .get("https://api.github.com/user")
        .header("User-Agent", "Everruns")
        .bearer_auth(&req.access_token)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Failed to validate GitHub token: {}", e);
            (
                StatusCode::BAD_REQUEST,
                "Invalid token: could not authenticate with github".to_string(),
            )
        })?
        .json()
        .await
        .map_err(|e| {
            tracing::error!("Failed to parse GitHub response: {}", e);
            (
                StatusCode::BAD_REQUEST,
                "Invalid token: could not authenticate with github".to_string(),
            )
        })?;

    let provider_user_id = user_info["id"].as_i64().map(|id| id.to_string());
    let provider_username = user_info["login"].as_str().map(|s| s.to_string());

    if provider_user_id.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid token: could not authenticate with github".to_string(),
        ));
    }

    // Encrypt token
    let access_token_encrypted = encryption.encrypt_string(&req.access_token).map_err(|e| {
        tracing::error!("Failed to encrypt token: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to store connection".to_string(),
        )
    })?;

    let row = state
        .db
        .upsert_user_connection(CreateUserConnectionRow {
            user_id: auth.user_id,
            provider: req.provider.clone(),
            provider_user_id,
            provider_username: provider_username.clone(),
            access_token_encrypted,
            refresh_token_encrypted: None,
            scopes: None, // Manual PAT — scopes unknown
            expires_at: None,
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to create connection: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to store connection".to_string(),
            )
        })?;

    Ok((
        StatusCode::CREATED,
        Json(ConnectionResponse {
            provider: row.provider,
            provider_username: row.provider_username,
            scopes: row.scopes,
            connected_at: row.created_at,
        }),
    ))
}

/// DELETE /v1/user/connections/:provider — Disconnect
pub async fn delete_connection(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(provider): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let deleted = state
        .db
        .delete_user_connection(auth.user_id, &provider)
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

/// GET /v1/user/connections/github/authorize — Start GitHub OAuth flow
pub async fn github_authorize(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Redirect, (StatusCode, String)> {
    let config = state.auth_config.github_connection.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "GitHub connection not configured".to_string(),
        )
    })?;

    let service = GitHubConnectionService::new(config);

    // Generate state for CSRF protection
    let mut rng = rand::rng();
    let bytes: [u8; 16] = rng.random();
    let oauth_state = hex::encode(bytes);

    // TODO: Store state in session/cookie for validation in callback
    // For now, include redirect_uri in state param
    let _redirect_after = params.get("redirect_uri").cloned();

    let auth_url = service.authorization_url(&oauth_state);
    Ok(Redirect::to(&auth_url.url))
}

/// GET /v1/user/connections/github/callback — GitHub OAuth callback
pub async fn github_callback(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<OAuthCallbackQuery>,
) -> Result<Redirect, (StatusCode, String)> {
    let config = state.auth_config.github_connection.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "GitHub connection not configured".to_string(),
        )
    })?;

    let encryption = state.encryption.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured".to_string(),
        )
    })?;

    let service = GitHubConnectionService::new(config);

    // Exchange code for token + user info
    let result = service.exchange_code(&query.code).await.map_err(|e| {
        tracing::error!("GitHub connection OAuth exchange failed: {}", e);
        (
            StatusCode::BAD_REQUEST,
            "GitHub authentication failed".to_string(),
        )
    })?;

    // Encrypt token
    let access_token_encrypted =
        encryption
            .encrypt_string(&result.access_token)
            .map_err(|e| {
                tracing::error!("Failed to encrypt token: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to store connection".to_string(),
                )
            })?;

    // Upsert connection
    state
        .db
        .upsert_user_connection(CreateUserConnectionRow {
            user_id: auth.user_id,
            provider: "github".to_string(),
            provider_user_id: Some(result.user_id),
            provider_username: Some(result.username),
            access_token_encrypted,
            refresh_token_encrypted: None,
            scopes: Some(result.scopes),
            expires_at: None,
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to store GitHub connection: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to store connection".to_string(),
            )
        })?;

    // Redirect back to settings page
    Ok(Redirect::to("/settings/connections?connected=github"))
}

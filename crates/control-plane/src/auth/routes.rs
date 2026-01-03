// Authentication HTTP routes
// Decision: Use /v1/auth/* prefix for all auth endpoints (consistent with other API routes)
// Decision: Support both JSON and cookie-based sessions

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Redirect,
    routing::{delete, get, post},
    Json, Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{Duration, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    api_key::generate_api_key,
    config::AuthMode,
    jwt::hash_token,
    middleware::{AuthError, AuthState, AuthUser},
    oauth::{GitHubOAuthService, GoogleOAuthService, OAuthProvider},
};
use crate::storage::{
    models::{CreateApiKeyRow, CreateRefreshTokenRow, CreateUserRow},
    password::{hash_password, verify_password},
};

/// Generate a random state string for OAuth (32 hex characters)
fn generate_oauth_state() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.gen();
    hex::encode(bytes)
}

/// Login request
#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

/// Register request
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub name: String,
}

/// Token response
#[derive(Debug, Serialize, ToSchema)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

/// User info response
#[derive(Debug, Serialize, ToSchema)]
pub struct UserInfoResponse {
    pub id: String,
    pub email: String,
    pub name: String,
    pub roles: Vec<String>,
    pub avatar_url: Option<String>,
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

/// Refresh token request
#[derive(Debug, Deserialize, ToSchema)]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

/// OAuth callback query parameters
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OAuthCallbackQuery {
    pub code: String,
    pub state: String, // TODO: Use for CSRF validation
}

/// Auth configuration response
#[derive(Debug, Serialize, ToSchema)]
pub struct AuthConfigResponse {
    pub mode: String,
    pub password_auth_enabled: bool,
    pub oauth_providers: Vec<String>,
    pub signup_enabled: bool,
}

/// Create auth routes
pub fn routes(state: AuthState) -> Router {
    Router::new()
        // Public routes
        .route("/v1/auth/config", get(get_auth_config))
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/register", post(register))
        .route("/v1/auth/refresh", post(refresh_token))
        .route("/v1/auth/logout", post(logout))
        // OAuth routes
        .route("/v1/auth/oauth/:provider", get(oauth_redirect))
        .route("/v1/auth/callback/:provider", get(oauth_callback))
        // Protected routes
        .route("/v1/auth/me", get(get_current_user))
        .route(
            "/v1/auth/api-keys",
            get(list_api_keys).post(create_api_key_route),
        )
        .route("/v1/auth/api-keys/:key_id", delete(delete_api_key_route))
        .with_state(state)
}

/// GET /v1/auth/config - Get authentication configuration
pub async fn get_auth_config(State(state): State<AuthState>) -> Json<AuthConfigResponse> {
    let mut oauth_providers = Vec::new();

    if state.config.google.is_some() {
        oauth_providers.push("google".to_string());
    }
    if state.config.github.is_some() {
        oauth_providers.push("github".to_string());
    }

    Json(AuthConfigResponse {
        mode: match state.config.mode {
            AuthMode::None => "none".to_string(),
            AuthMode::Admin => "admin".to_string(),
            AuthMode::Full => "full".to_string(),
        },
        password_auth_enabled: state.config.password_auth_enabled(),
        oauth_providers,
        // Admin mode has a single predefined user, no signup allowed
        signup_enabled: state.config.mode != AuthMode::Admin && !state.config.disable_signup,
    })
}

/// POST /v1/auth/login - Login with email and password
pub async fn login(
    State(state): State<AuthState>,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> Result<(CookieJar, Json<TokenResponse>), AuthError> {
    // In admin mode, check admin credentials directly (no database lookup)
    if state.config.mode == AuthMode::Admin {
        if let Some(admin) = &state.config.admin {
            if req.email == admin.email && req.password == admin.password {
                // Create or get admin user
                let user = get_or_create_admin_user(&state, admin).await?;
                return generate_token_response(&state, jar, &user).await;
            }
        }
        // Admin mode only allows the configured admin credentials
        return Err(AuthError::unauthorized("Invalid email or password"));
    }

    // Check if password auth is enabled (for non-admin modes)
    if !state.config.password_auth_enabled() {
        return Err(AuthError::unauthorized(
            "Password authentication is disabled",
        ));
    }

    // Find user by email
    let user = state
        .db
        .get_user_by_email(&req.email)
        .await
        .map_err(|e| {
            tracing::error!("Database error during login: {}", e);
            AuthError::unauthorized("Login failed")
        })?
        .ok_or_else(|| AuthError::unauthorized("Invalid email or password"))?;

    // Verify password
    let password_hash = user
        .password_hash
        .as_ref()
        .ok_or_else(|| AuthError::unauthorized("Password login not available for this account"))?;

    let valid = verify_password(&req.password, password_hash).map_err(|e| {
        tracing::error!("Password verification error: {}", e);
        AuthError::unauthorized("Login failed")
    })?;

    if !valid {
        return Err(AuthError::unauthorized("Invalid email or password"));
    }

    let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();

    let auth_user = AuthUser {
        id: user.id,
        email: user.email,
        name: user.name,
        roles,
        auth_method: super::middleware::AuthMethod::Jwt,
    };

    generate_token_response(&state, jar, &auth_user).await
}

/// POST /v1/auth/register - Register a new user
pub async fn register(
    State(state): State<AuthState>,
    jar: CookieJar,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, CookieJar, Json<TokenResponse>), AuthError> {
    // Check if signup is enabled
    if state.config.disable_signup {
        return Err(AuthError::forbidden("Registration is disabled"));
    }

    // Check if password auth is enabled
    if !state.config.password_auth_enabled() {
        return Err(AuthError::forbidden("Password registration is disabled"));
    }

    // Check if user already exists
    let existing = state.db.get_user_by_email(&req.email).await.map_err(|e| {
        tracing::error!("Database error during registration: {}", e);
        AuthError::unauthorized("Registration failed")
    })?;

    if existing.is_some() {
        return Err(AuthError::unauthorized("Email already registered"));
    }

    // Hash password
    let password_hash = hash_password(&req.password).map_err(|e| {
        tracing::error!("Password hashing error: {}", e);
        AuthError::unauthorized("Registration failed")
    })?;

    // Create user
    let user = state
        .db
        .create_user(CreateUserRow {
            email: req.email.clone(),
            name: req.name.clone(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: Some(password_hash),
            email_verified: false,
            auth_provider: Some("local".to_string()),
            auth_provider_id: None,
        })
        .await
        .map_err(|e| {
            tracing::error!("User creation error: {}", e);
            AuthError::unauthorized("Registration failed")
        })?;

    let auth_user = AuthUser {
        id: user.id,
        email: user.email,
        name: user.name,
        roles: vec!["user".to_string()],
        auth_method: super::middleware::AuthMethod::Jwt,
    };

    let (jar, json) = generate_token_response(&state, jar, &auth_user).await?;
    Ok((StatusCode::CREATED, jar, json))
}

/// POST /v1/auth/refresh - Refresh access token
pub async fn refresh_token(
    State(state): State<AuthState>,
    jar: CookieJar,
    Json(req): Json<RefreshTokenRequest>,
) -> Result<(CookieJar, Json<TokenResponse>), AuthError> {
    // Validate refresh token
    let claims = state
        .jwt_service
        .validate_refresh_token(&req.refresh_token)
        .map_err(|_| AuthError::unauthorized("Invalid refresh token"))?;

    // Check if token is in database (not revoked)
    let token_hash = hash_token(&req.refresh_token);
    let token_row = state
        .db
        .get_refresh_token_by_hash(&token_hash)
        .await
        .map_err(|e| {
            tracing::error!("Database error during refresh: {}", e);
            AuthError::unauthorized("Refresh failed")
        })?
        .ok_or_else(|| AuthError::unauthorized("Invalid refresh token"))?;

    // Check expiration
    if token_row.expires_at < Utc::now() {
        return Err(AuthError::unauthorized("Refresh token expired"));
    }

    // Delete old refresh token
    let _ = state.db.delete_refresh_token(token_row.id).await;

    // Get user
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AuthError::unauthorized("Invalid user ID in token"))?;

    let user = state
        .db
        .get_user(user_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error during refresh: {}", e);
            AuthError::unauthorized("Refresh failed")
        })?
        .ok_or_else(|| AuthError::unauthorized("User not found"))?;

    let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();

    let auth_user = AuthUser {
        id: user.id,
        email: user.email,
        name: user.name,
        roles,
        auth_method: super::middleware::AuthMethod::Jwt,
    };

    generate_token_response(&state, jar, &auth_user).await
}

/// POST /v1/auth/logout - Logout (clear cookies)
pub async fn logout(jar: CookieJar) -> CookieJar {
    jar.remove(Cookie::build("access_token").path("/"))
        .remove(Cookie::build("refresh_token").path("/"))
}

/// GET /v1/auth/me - Get current user info
pub async fn get_current_user(user: AuthUser) -> Json<UserInfoResponse> {
    Json(UserInfoResponse {
        id: user.id.to_string(),
        email: user.email,
        name: user.name,
        roles: user.roles,
        avatar_url: None,
    })
}

/// GET /v1/auth/oauth/:provider - Redirect to OAuth provider
pub async fn oauth_redirect(
    State(state): State<AuthState>,
    Path(provider): Path<String>,
) -> Result<Redirect, AuthError> {
    let provider_enum = OAuthProvider::from_str(&provider)
        .ok_or_else(|| AuthError::unauthorized("Unknown OAuth provider"))?;

    // Generate a random state for CSRF protection
    let oauth_state = generate_oauth_state();

    let auth_url = match provider_enum {
        OAuthProvider::Google => {
            let config = state
                .config
                .google
                .as_ref()
                .ok_or_else(|| AuthError::unauthorized("Google OAuth not configured"))?;
            let service = GoogleOAuthService::new(config)
                .map_err(|_| AuthError::unauthorized("OAuth configuration error"))?;
            service.authorization_url(&oauth_state)
        }
        OAuthProvider::GitHub => {
            let config = state
                .config
                .github
                .as_ref()
                .ok_or_else(|| AuthError::unauthorized("GitHub OAuth not configured"))?;
            let service = GitHubOAuthService::new(config)
                .map_err(|_| AuthError::unauthorized("OAuth configuration error"))?;
            service.authorization_url(&oauth_state)
        }
    };

    // In a production system, we'd store the state in a session/cookie for verification
    // TODO: Implement proper state management for OAuth

    Ok(Redirect::to(&auth_url.url))
}

/// GET /v1/auth/callback/:provider - OAuth callback
pub async fn oauth_callback(
    State(state): State<AuthState>,
    Path(provider): Path<String>,
    Query(query): Query<OAuthCallbackQuery>,
    jar: CookieJar,
) -> Result<(CookieJar, Redirect), AuthError> {
    let provider_enum = OAuthProvider::from_str(&provider)
        .ok_or_else(|| AuthError::unauthorized("Unknown OAuth provider"))?;

    // TODO: Validate state from session for CSRF protection
    // For now, we'll skip state validation in development

    let user_info = match provider_enum {
        OAuthProvider::Google => {
            let config = state
                .config
                .google
                .as_ref()
                .ok_or_else(|| AuthError::unauthorized("Google OAuth not configured"))?;
            let service = GoogleOAuthService::new(config)
                .map_err(|_| AuthError::unauthorized("OAuth configuration error"))?;
            service.exchange_code(&query.code).await
        }
        OAuthProvider::GitHub => {
            let config = state
                .config
                .github
                .as_ref()
                .ok_or_else(|| AuthError::unauthorized("GitHub OAuth not configured"))?;
            let service = GitHubOAuthService::new(config)
                .map_err(|_| AuthError::unauthorized("OAuth configuration error"))?;
            service.exchange_code(&query.code).await
        }
    }
    .map_err(|e| {
        tracing::error!("OAuth exchange failed: {}", e);
        AuthError::unauthorized("OAuth authentication failed")
    })?;

    // Find or create user
    let provider_str = provider_enum.as_str();
    let user = state
        .db
        .get_user_by_oauth(provider_str, &user_info.provider_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error during OAuth: {}", e);
            AuthError::unauthorized("OAuth authentication failed")
        })?;

    let user = if let Some(user) = user {
        user
    } else {
        // Check if user exists by email (for account linking)
        let existing_user = state
            .db
            .get_user_by_email(&user_info.email)
            .await
            .map_err(|e| {
                tracing::error!("Database error during OAuth: {}", e);
                AuthError::unauthorized("OAuth authentication failed")
            })?;

        if let Some(_existing) = existing_user {
            // For now, don't auto-link accounts - require explicit action
            // TODO: Implement account linking flow
            return Err(AuthError::unauthorized(
                "An account with this email already exists. Please login with your existing credentials.",
            ));
        }

        // Create new user
        state
            .db
            .create_user(CreateUserRow {
                email: user_info.email.clone(),
                name: user_info.name.clone(),
                avatar_url: user_info.avatar_url.clone(),
                roles: vec!["user".to_string()],
                password_hash: None,
                email_verified: user_info.email_verified,
                auth_provider: Some(provider_str.to_string()),
                auth_provider_id: Some(user_info.provider_id.clone()),
            })
            .await
            .map_err(|e| {
                tracing::error!("User creation error during OAuth: {}", e);
                AuthError::unauthorized("OAuth authentication failed")
            })?
    };

    let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();

    let auth_user = AuthUser {
        id: user.id,
        email: user.email,
        name: user.name,
        roles,
        auth_method: super::middleware::AuthMethod::Jwt,
    };

    // Generate tokens and set cookies
    let (jar, _) = generate_token_response(&state, jar, &auth_user).await?;

    // Redirect to frontend
    Ok((jar, Redirect::to("/")))
}

/// GET /v1/auth/api-keys - List API keys for current user
pub async fn list_api_keys(
    State(state): State<AuthState>,
    user: AuthUser,
) -> Result<Json<Vec<ApiKeyListItem>>, AuthError> {
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

    Ok(Json(items))
}

/// POST /v1/auth/api-keys - Create a new API key
pub async fn create_api_key_route(
    State(state): State<AuthState>,
    user: AuthUser,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<ApiKeyResponse>), AuthError> {
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
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to create API key: {}", e);
            AuthError::unauthorized("Failed to create API key")
        })?;

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
pub async fn delete_api_key_route(
    State(state): State<AuthState>,
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
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AuthError::unauthorized("API key not found"))
    }
}

/// Helper: Generate token response with cookies
async fn generate_token_response(
    state: &AuthState,
    jar: CookieJar,
    user: &AuthUser,
) -> Result<(CookieJar, Json<TokenResponse>), AuthError> {
    let (token_pair, _refresh_jti) = state
        .jwt_service
        .generate_token_pair(user.id, &user.email, &user.name, &user.roles)
        .map_err(|e| {
            tracing::error!("Token generation error: {}", e);
            AuthError::unauthorized("Login failed")
        })?;

    // Store refresh token hash in database
    let refresh_token_hash = hash_token(&token_pair.refresh_token);
    let expires_at = Utc::now()
        + Duration::from_std(state.config.jwt.refresh_token_lifetime)
            .map_err(|_| AuthError::unauthorized("Login failed"))?;

    state
        .db
        .create_refresh_token(CreateRefreshTokenRow {
            user_id: user.id,
            token_hash: refresh_token_hash,
            expires_at,
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to store refresh token: {}", e);
            AuthError::unauthorized("Login failed")
        })?;

    // Set cookies
    let access_cookie = Cookie::build(("access_token", token_pair.access_token.clone()))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::seconds(token_pair.expires_in))
        .build();

    let refresh_cookie = Cookie::build(("refresh_token", token_pair.refresh_token.clone()))
        .path("/v1/auth")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Strict)
        .max_age(time::Duration::seconds(
            state.jwt_service.refresh_token_lifetime_secs(),
        ))
        .build();

    let jar = jar.add(access_cookie).add(refresh_cookie);

    Ok((
        jar,
        Json(TokenResponse {
            access_token: token_pair.access_token,
            token_type: token_pair.token_type,
            expires_in: token_pair.expires_in,
            refresh_token: Some(token_pair.refresh_token),
        }),
    ))
}

/// Helper: Get or create admin user
async fn get_or_create_admin_user(
    state: &AuthState,
    admin: &super::config::AdminConfig,
) -> Result<AuthUser, AuthError> {
    let user = state
        .db
        .get_user_by_email(&admin.email)
        .await
        .map_err(|e| {
            tracing::error!("Database error: {}", e);
            AuthError::unauthorized("Login failed")
        })?;

    let user = if let Some(user) = user {
        user
    } else {
        // Create admin user
        let password_hash = hash_password(&admin.password).map_err(|e| {
            tracing::error!("Password hashing error: {}", e);
            AuthError::unauthorized("Login failed")
        })?;

        state
            .db
            .create_user(CreateUserRow {
                email: admin.email.clone(),
                name: "Admin".to_string(),
                avatar_url: None,
                roles: vec!["admin".to_string()],
                password_hash: Some(password_hash),
                email_verified: true,
                auth_provider: Some("local".to_string()),
                auth_provider_id: None,
            })
            .await
            .map_err(|e| {
                tracing::error!("User creation error: {}", e);
                AuthError::unauthorized("Login failed")
            })?
    };

    let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();

    Ok(AuthUser {
        id: user.id,
        email: user.email,
        name: user.name,
        roles,
        auth_method: super::middleware::AuthMethod::Jwt,
    })
}

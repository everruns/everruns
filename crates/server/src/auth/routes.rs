// Authentication HTTP routes
// Decision: Use /v1/auth/* prefix for all auth endpoints (consistent with other API routes)
// Decision: Support both JSON and cookie-based sessions

use axum::{
    Json, Router,
    body::Body,
    extract::{FromRef, Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{Redirect, Response},
    routing::{delete, get, post},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{Duration, Utc};
use everruns_core::DEFAULT_ORG_ID;
use rand::Rng;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::audit;
use super::rate_limit::extract_client_ip;

use super::{
    api_key::generate_api_key,
    builtin::{self, BuiltinAuthBackend},
    config::AuthMode,
    jwt::hash_token,
    middleware::{AuthError, AuthMethod, AuthState, AuthUser, ORG_COOKIE_NAME, ResolvedOrg},
    oauth::{GitHubOAuthService, GoogleOAuthService, OAuthProvider},
};
/// Enable AuthUser extractor when BuiltinAuthBackend is the route state.
/// AuthUser needs AuthState via FromRef — this converts BuiltinAuthBackend to AuthState.
impl FromRef<BuiltinAuthBackend> for AuthState {
    fn from_ref(backend: &BuiltinAuthBackend) -> Self {
        AuthState::new(backend.config.clone(), std::sync::Arc::new(backend.clone()))
    }
}

use crate::api::common::ListResponse;
use crate::storage::{
    models::{CreateApiKeyRow, CreateRefreshTokenRow, CreateUserRow},
    password::{hash_password, verify_password},
};

/// Generate a random state string for OAuth (32 hex characters)
fn generate_oauth_state() -> String {
    let mut rng = rand::rng();
    let bytes: [u8; 16] = rng.random();
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

/// Organization membership in user response
#[derive(Debug, Serialize, ToSchema)]
pub struct OrgMembershipResponse {
    pub public_id: String,
    pub name: String,
    pub role: String,
}

/// User info response
#[derive(Debug, Serialize, ToSchema)]
pub struct UserInfoResponse {
    pub id: String,
    pub email: String,
    pub name: String,
    pub roles: Vec<String>,
    pub avatar_url: Option<String>,
    /// Organizations the user belongs to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organizations: Option<Vec<OrgMembershipResponse>>,
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

/// Cookie name for OAuth CSRF state (TM-AUTH-007)
const OAUTH_STATE_COOKIE: &str = "oauth_state";

/// OAuth callback query parameters
#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: String,
    pub state: String,
}

/// Auth configuration response
#[derive(Debug, Serialize, ToSchema)]
pub struct AuthConfigResponse {
    pub mode: String,
    pub password_auth_enabled: bool,
    pub oauth_providers: Vec<String>,
    pub signup_enabled: bool,
}

/// Rate limit middleware for login endpoint
async fn rate_limit_login(
    State(state): State<BuiltinAuthBackend>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let ip = extract_client_ip(&req);
    if let Err(e) = state.rate_limiter.check_login(ip) {
        return e.into();
    }
    next.run(req).await
}

/// Rate limit middleware for register endpoint
async fn rate_limit_register(
    State(state): State<BuiltinAuthBackend>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let ip = extract_client_ip(&req);
    if let Err(e) = state.rate_limiter.check_register(ip) {
        return e.into();
    }
    next.run(req).await
}

/// Rate limit middleware for refresh endpoint
async fn rate_limit_refresh(
    State(state): State<BuiltinAuthBackend>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let ip = extract_client_ip(&req);
    if let Err(e) = state.rate_limiter.check_refresh(ip) {
        return e.into();
    }
    next.run(req).await
}

/// Create auth routes
pub fn routes(state: BuiltinAuthBackend) -> Router {
    // Rate-limited routes (sensitive auth endpoints)
    let login_route = Router::new()
        .route("/v1/auth/login", post(login))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_login,
        ));
    let register_route = Router::new()
        .route("/v1/auth/register", post(register))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_register,
        ));
    let refresh_route = Router::new()
        .route("/v1/auth/refresh", post(refresh_token))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_refresh,
        ));

    Router::new()
        // Public routes (no rate limit needed)
        .route("/v1/auth/config", get(get_auth_config))
        .route("/v1/auth/logout", post(logout))
        // OAuth routes
        .route("/v1/auth/oauth/{provider}", get(oauth_redirect))
        .route("/v1/auth/callback/{provider}", get(oauth_callback))
        // Protected routes
        .route("/v1/auth/me", get(get_current_user))
        .route(
            "/v1/auth/api-keys",
            get(list_api_keys).post(create_api_key_route),
        )
        .route("/v1/auth/api-keys/{key_id}", delete(delete_api_key_route))
        // Merge rate-limited routes
        .merge(login_route)
        .merge(register_route)
        .merge(refresh_route)
        .with_state(state)
}

/// GET /v1/auth/config - Get authentication configuration
pub async fn get_auth_config(State(state): State<BuiltinAuthBackend>) -> Json<AuthConfigResponse> {
    let mut oauth_providers = Vec::new();

    if state.config.google.is_some() {
        oauth_providers.push("google".to_string());
    }
    if state.config.github.is_some() {
        oauth_providers.push("github".to_string());
    }

    Json(AuthConfigResponse {
        mode: state.config.mode.as_str().to_string(),
        password_auth_enabled: state.config.password_auth_enabled(),
        oauth_providers,
        signup_enabled: state.config.signup_enabled(),
    })
}

/// POST /v1/auth/login - Login with email and password
pub async fn login(
    State(state): State<BuiltinAuthBackend>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> Result<(CookieJar, Json<TokenResponse>), AuthError> {
    let ip = audit::client_ip(&headers);

    // In admin mode, check admin credentials directly (no database lookup)
    if state.config.mode == AuthMode::Admin {
        if let Some(admin) = &state.config.admin
            && req.email == admin.email
            && req.password == admin.password
        {
            // Create or get admin user
            let user = get_or_create_admin_user(&state, admin).await?;
            audit::emit(
                state.db.clone(),
                DEFAULT_ORG_ID,
                Some(user.id),
                "auth.login.success",
                ip,
                serde_json::json!({"method": "admin"}),
            );
            return generate_token_response(&state, jar, &user).await;
        }
        // Admin mode only allows the configured admin credentials
        audit::emit(
            state.db.clone(),
            DEFAULT_ORG_ID,
            None,
            "auth.login.failure",
            ip,
            serde_json::json!({"method": "admin", "reason": "invalid_credentials"}),
        );
        return Err(AuthError::unauthorized("Invalid email or password"));
    }

    // Check if password auth is enabled (for non-admin modes)
    if !state.config.password_auth_enabled() {
        return Err(AuthError::unauthorized(
            "Password authentication is disabled",
        ));
    }

    // Find user by email
    let user = state.db.get_user_by_email(&req.email).await.map_err(|e| {
        tracing::error!("Database error during login: {}", e);
        AuthError::unauthorized("Login failed")
    })?;

    let Some(user) = user else {
        audit::emit(
            state.db.clone(),
            DEFAULT_ORG_ID,
            None,
            "auth.login.failure",
            ip,
            serde_json::json!({"reason": "user_not_found"}),
        );
        return Err(AuthError::unauthorized("Invalid email or password"));
    };

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
        audit::emit(
            state.db.clone(),
            DEFAULT_ORG_ID,
            Some(user.id),
            "auth.login.failure",
            ip,
            serde_json::json!({"reason": "invalid_password"}),
        );
        return Err(AuthError::unauthorized("Invalid email or password"));
    }

    let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();

    let organizations = builtin::fetch_user_organizations(&state.db, user.id)
        .await
        .unwrap_or_default();

    let auth_user = AuthUser {
        id: user.id,
        email: user.email,
        name: user.name,
        roles,
        auth_method: AuthMethod::Jwt,
        organizations: builtin::organizations_or_default(organizations),
    };

    audit::emit(
        state.db.clone(),
        DEFAULT_ORG_ID,
        Some(auth_user.id),
        "auth.login.success",
        ip,
        serde_json::json!({"method": "password"}),
    );

    generate_token_response(&state, jar, &auth_user).await
}

/// POST /v1/auth/register - Register a new user
pub async fn register(
    State(state): State<BuiltinAuthBackend>,
    headers: HeaderMap,
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

    // Hash password first to make timing consistent whether or not the email exists.
    // This prevents account enumeration via response-time differences (TM-AUTH-014).
    let password_hash = hash_password(&req.password).map_err(|e| {
        tracing::error!("Password hashing error: {}", e);
        AuthError::unauthorized("Registration failed")
    })?;

    // Check if user already exists — generic error to prevent account enumeration
    let existing = state.db.get_user_by_email(&req.email).await.map_err(|e| {
        tracing::error!("Database error during registration: {}", e);
        AuthError::unauthorized("Registration failed")
    })?;

    if existing.is_some() {
        return Err(AuthError::unauthorized("Registration failed"));
    }

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
            external_id: None,
        })
        .await
        .map_err(|e| {
            tracing::error!("User creation error: {}", e);
            AuthError::unauthorized("Registration failed")
        })?;

    // Add user to default organization
    let _ = state
        .db
        .add_organization_member(DEFAULT_ORG_ID, user.id, "member")
        .await
        .map_err(|e| {
            tracing::error!("Failed to add user to default org: {}", e);
            // Continue anyway - user is created, they just might not have org membership
        });

    let organizations = builtin::fetch_user_organizations(&state.db, user.id)
        .await
        .unwrap_or_default();

    let auth_user = AuthUser {
        id: user.id,
        email: user.email,
        name: user.name,
        roles: vec!["user".to_string()],
        auth_method: AuthMethod::Jwt,
        organizations: builtin::organizations_or_default(organizations),
    };

    audit::emit(
        state.db.clone(),
        DEFAULT_ORG_ID,
        Some(auth_user.id),
        "auth.register.success",
        audit::client_ip(&headers),
        serde_json::json!({}),
    );

    let (jar, json) = generate_token_response(&state, jar, &auth_user).await?;
    Ok((StatusCode::CREATED, jar, json))
}

/// POST /v1/auth/refresh - Refresh access token
///
/// Accepts the refresh token from either the JSON body (`{ "refresh_token": "..." }`)
/// or the `refresh_token` HttpOnly cookie (set at login). Cookie-based is the
/// primary flow for browser clients since the cookie is HttpOnly.
pub async fn refresh_token(
    State(state): State<BuiltinAuthBackend>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Option<Json<RefreshTokenRequest>>,
) -> Result<(CookieJar, Json<TokenResponse>), AuthError> {
    // Prefer JSON body, fall back to cookie
    let refresh_token_value = if let Some(Json(req)) = body {
        req.refresh_token
    } else if let Some(cookie) = jar.get("refresh_token") {
        cookie.value().to_string()
    } else {
        return Err(AuthError::unauthorized("Missing refresh token"));
    };

    // Validate refresh token
    let claims = state
        .jwt_service
        .validate_refresh_token(&refresh_token_value)
        .map_err(|_| AuthError::unauthorized("Invalid refresh token"))?;

    // Check if token is in database (not revoked)
    let token_hash = hash_token(&refresh_token_value);
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

    let organizations = builtin::fetch_user_organizations(&state.db, user.id)
        .await
        .unwrap_or_default();

    let auth_user = AuthUser {
        id: user.id,
        email: user.email,
        name: user.name,
        roles,
        auth_method: AuthMethod::Jwt,
        organizations: builtin::organizations_or_default(organizations),
    };

    audit::emit(
        state.db.clone(),
        DEFAULT_ORG_ID,
        Some(auth_user.id),
        "auth.token_refresh.success",
        audit::client_ip(&headers),
        serde_json::json!({}),
    );

    generate_token_response(&state, jar, &auth_user).await
}

/// POST /v1/auth/logout - Logout (clear cookies)
pub async fn logout(jar: CookieJar) -> CookieJar {
    jar.remove(Cookie::build("access_token").path("/"))
        .remove(Cookie::build("refresh_token").path("/"))
}

/// GET /v1/auth/me - Get current user info
/// Also sets everruns_org cookie if missing (using user's first org)
pub async fn get_current_user(
    user: AuthUser,
    jar: CookieJar,
) -> (CookieJar, Json<UserInfoResponse>) {
    // Always return organizations array (middleware ensures at least default org)
    let organizations = Some(
        user.organizations
            .iter()
            .map(|o| OrgMembershipResponse {
                public_id: o.public_id.clone(),
                name: o.name.clone(),
                role: o.role.as_str().to_string(),
            })
            .collect(),
    );

    // Set org cookie if missing (ensures subsequent API calls have org context)
    let jar = if jar.get(ORG_COOKIE_NAME).is_none() {
        if let Some(org) = user.organizations.first() {
            let cookie = Cookie::build((ORG_COOKIE_NAME, org.public_id.clone()))
                .path("/")
                .http_only(false) // Allow JS to read for UI state
                .secure(true)
                .same_site(SameSite::Lax)
                .build();
            jar.add(cookie)
        } else {
            jar
        }
    } else {
        jar
    };

    (
        jar,
        Json(UserInfoResponse {
            id: user.id.to_string(),
            email: user.email,
            name: user.name,
            roles: user.roles,
            avatar_url: None,
            organizations,
        }),
    )
}

/// GET /v1/auth/oauth/:provider - Redirect to OAuth provider
/// TM-AUTH-007: State stored in HttpOnly cookie for CSRF validation in callback.
pub async fn oauth_redirect(
    State(state): State<BuiltinAuthBackend>,
    Path(provider): Path<String>,
    jar: CookieJar,
) -> Result<(CookieJar, Redirect), AuthError> {
    let provider_enum = OAuthProvider::parse(&provider)
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

    // TM-AUTH-007: Store state in HttpOnly cookie for CSRF validation
    let state_cookie = Cookie::build((OAUTH_STATE_COOKIE, oauth_state))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::minutes(10))
        .build();
    let jar = jar.add(state_cookie);

    Ok((jar, Redirect::to(&auth_url.url)))
}

/// GET /v1/auth/callback/:provider - OAuth callback
/// TM-AUTH-007: Validates CSRF state from cookie before proceeding.
pub async fn oauth_callback(
    State(state): State<BuiltinAuthBackend>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Query(query): Query<OAuthCallbackQuery>,
    jar: CookieJar,
) -> Result<(CookieJar, Redirect), AuthError> {
    let provider_enum = OAuthProvider::parse(&provider)
        .ok_or_else(|| AuthError::unauthorized("Unknown OAuth provider"))?;

    // TM-AUTH-007: Validate CSRF state parameter
    let stored_state = jar
        .get(OAUTH_STATE_COOKIE)
        .map(|c| c.value().to_string())
        .ok_or_else(|| {
            tracing::warn!("OAuth callback missing state cookie (possible CSRF attempt)");
            AuthError::unauthorized("Invalid OAuth state")
        })?;

    if stored_state != query.state {
        tracing::warn!("OAuth callback state mismatch (possible CSRF attempt)");
        return Err(AuthError::unauthorized("Invalid OAuth state"));
    }

    // Clear the state cookie (single-use)
    let jar = jar.remove(Cookie::build(OAUTH_STATE_COOKIE).path("/"));

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
        audit::emit(
            state.db.clone(),
            DEFAULT_ORG_ID,
            None,
            "auth.oauth.failure",
            audit::client_ip(&headers),
            serde_json::json!({"provider": provider, "reason": "exchange_failed"}),
        );
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
        let created_user = state
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
                external_id: None,
            })
            .await
            .map_err(|e| {
                tracing::error!("User creation error during OAuth: {}", e);
                AuthError::unauthorized("OAuth authentication failed")
            })?;

        // Add newly created user to default organization
        let _ = state
            .db
            .add_organization_member(DEFAULT_ORG_ID, created_user.id, "member")
            .await
            .map_err(|e| {
                tracing::error!("Failed to add OAuth user to default org: {}", e);
                // Continue anyway
            });

        created_user
    };

    let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();

    let organizations = builtin::fetch_user_organizations(&state.db, user.id)
        .await
        .unwrap_or_default();

    let auth_user = AuthUser {
        id: user.id,
        email: user.email,
        name: user.name,
        roles,
        auth_method: AuthMethod::Jwt,
        organizations: builtin::organizations_or_default(organizations),
    };

    audit::emit(
        state.db.clone(),
        DEFAULT_ORG_ID,
        Some(auth_user.id),
        "auth.oauth.success",
        audit::client_ip(&headers),
        serde_json::json!({"provider": provider}),
    );

    // Generate tokens and set cookies
    let (jar, _) = generate_token_response(&state, jar, &auth_user).await?;

    // Redirect to frontend (different origin in dev)
    let redirect_url = format!("{}/", state.config.frontend_url.trim_end_matches('/'));
    Ok((jar, Redirect::to(&redirect_url)))
}

/// GET /v1/auth/api-keys - List API keys for current user
pub async fn list_api_keys(
    State(state): State<BuiltinAuthBackend>,
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
/// Organization is derived from the everruns_org cookie (set via /v1/users/me/switch-org).
/// Cannot be called with API key authentication (must use session auth).
pub async fn create_api_key_route(
    State(state): State<BuiltinAuthBackend>,
    headers: HeaderMap,
    user: AuthUser,
    org: ResolvedOrg,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<ApiKeyResponse>), AuthError> {
    // Cannot create API key using API key auth
    if user.auth_method == AuthMethod::ApiKey {
        return Err(AuthError::forbidden(
            "Cannot create API key using API key authentication",
        ));
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
            org_id: org.org_id,
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

    audit::emit(
        state.db.clone(),
        org.org_id,
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
pub async fn delete_api_key_route(
    State(state): State<BuiltinAuthBackend>,
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
        audit::emit(
            state.db.clone(),
            DEFAULT_ORG_ID,
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

/// Helper: Generate token response with cookies
async fn generate_token_response(
    state: &BuiltinAuthBackend,
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

    // Path must be "/" so the cookie is sent through the /api proxy.
    // "/v1/auth" doesn't match the browser-side path "/api/v1/auth".
    let refresh_cookie = Cookie::build(("refresh_token", token_pair.refresh_token.clone()))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Strict)
        .max_age(time::Duration::seconds(
            state.jwt_service.refresh_token_lifetime_secs(),
        ))
        .build();

    let mut jar = jar.add(access_cookie).add(refresh_cookie);

    // Set org cookie to user's first org (ensures org context for subsequent API calls)
    if let Some(org) = user.organizations.first() {
        let org_cookie = Cookie::build((ORG_COOKIE_NAME, org.public_id.clone()))
            .path("/")
            .http_only(false) // Allow JS to read for UI state
            .secure(true)
            .same_site(SameSite::Lax)
            .build();
        jar = jar.add(org_cookie);
    }

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
    state: &BuiltinAuthBackend,
    admin: &super::config::AdminConfig,
) -> Result<AuthUser, AuthError> {
    let existing_user = state
        .db
        .get_user_by_email(&admin.email)
        .await
        .map_err(|e| {
            tracing::error!("Database error: {}", e);
            AuthError::unauthorized("Login failed")
        })?;

    let user = if let Some(user) = existing_user {
        user
    } else {
        // Create admin user
        let password_hash = hash_password(&admin.password).map_err(|e| {
            tracing::error!("Password hashing error: {}", e);
            AuthError::unauthorized("Login failed")
        })?;

        let created_user = state
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
                external_id: None,
            })
            .await
            .map_err(|e| {
                tracing::error!("User creation error: {}", e);
                AuthError::unauthorized("Login failed")
            })?;

        // Add admin user to default organization
        let _ = state
            .db
            .add_organization_member(DEFAULT_ORG_ID, created_user.id, "member")
            .await
            .map_err(|e| {
                tracing::error!("Failed to add admin user to default org: {}", e);
                // Continue anyway
            });

        created_user
    };

    let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();

    let organizations = builtin::fetch_user_organizations(&state.db, user.id)
        .await
        .unwrap_or_default();

    // For existing admin users not in any org, try to add them
    if organizations.is_empty() {
        let _ = state
            .db
            .add_organization_member(DEFAULT_ORG_ID, user.id, "member")
            .await;
    }

    Ok(AuthUser {
        id: user.id,
        email: user.email,
        name: user.name,
        roles,
        auth_method: AuthMethod::Jwt,
        organizations: builtin::organizations_or_default(organizations),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_oauth_state_is_unique() {
        let s1 = generate_oauth_state();
        let s2 = generate_oauth_state();
        assert_ne!(s1, s2);
        assert_eq!(s1.len(), 32); // 16 bytes hex-encoded
    }

    #[test]
    fn test_oauth_state_cookie_name() {
        // Verify the constant is set correctly for TM-AUTH-007
        assert_eq!(OAUTH_STATE_COOKIE, "oauth_state");
    }
}

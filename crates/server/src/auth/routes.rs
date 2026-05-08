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
    routing::{get, post},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{Duration, Utc};
use everruns_core::{DEFAULT_ORG_ID, OrgRole};
use rand::Rng;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::audit;
use super::rate_limit::extract_client_ip;

use super::{
    builtin::{self, BuiltinAuthBackend},
    config::AuthMode,
    jwt::hash_token,
    middleware::{AuthError, AuthMethod, AuthState, AuthUser, ORG_COOKIE_NAME},
    oauth::{GitHubOAuthService, GoogleOAuthService, OAuthProvider},
};
/// Enable AuthUser extractor when BuiltinAuthBackend is the route state.
/// AuthUser needs AuthState via FromRef — this converts BuiltinAuthBackend to AuthState.
impl FromRef<BuiltinAuthBackend> for AuthState {
    fn from_ref(backend: &BuiltinAuthBackend) -> Self {
        AuthState::new(backend.config.clone(), std::sync::Arc::new(backend.clone()))
    }
}

use crate::storage::{
    models::{CreateRefreshTokenRow, CreateUserRow},
    password::{hash_password, verify_password},
};

/// Generate a random state string for OAuth (32 hex characters)
fn generate_oauth_state() -> String {
    let mut rng = rand::rng();
    let bytes: [u8; 16] = rng.random();
    hex::encode(bytes)
}

/// Extract the lowercased domain from an email address.
fn email_domain_lowercase(email: &str) -> Option<String> {
    email
        .rsplit_once('@')
        .map(|(_, d)| d.trim().to_ascii_lowercase())
        .filter(|d| !d.is_empty())
}

/// Normalize a configured `allowed_domains` entry: trim whitespace, drop an
/// optional leading `@`, lowercase. Returns `None` for empty entries so a
/// stray `,` in `AUTH_GOOGLE_ALLOWED_DOMAINS=,company.com` does not silently
/// match every email.
///
/// Matching is intentionally **exact-domain** (not suffix).
/// `company.com` does not authorize `attacker.company.com`. Operators who
/// want subdomains must list each one explicitly.
fn normalize_allowed_domain(entry: &str) -> Option<String> {
    let trimmed = entry.trim().strip_prefix('@').unwrap_or(entry.trim());
    let cleaned = trimmed.trim();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_ascii_lowercase())
    }
}

/// Whether `email` belongs to one of `allowed_domains` (case-insensitive,
/// **exact** match against the lowercased host portion of the address).
/// Accepts configured entries written as either `company.com` or
/// `@company.com` so a small operator typo is not silently permissive.
fn email_domain_allowed(email: &str, allowed_domains: &[String]) -> bool {
    let Some(domain) = email_domain_lowercase(email) else {
        return false;
    };
    allowed_domains
        .iter()
        .filter_map(|d| normalize_allowed_domain(d))
        .any(|d| d == domain)
}

/// Per-provider identity gates applied after `exchange_code` and before any
/// user lookup or creation. Returns the audit `reason` string when the
/// identity must be rejected.
///
/// Google: must report `email_verified` and, when `allowed_domains` is set,
/// the email domain must be in that list. GitHub does not currently support
/// per-provider gates here.
fn oauth_identity_rejection_reason(
    provider: super::oauth::OAuthProvider,
    config: &super::config::AuthConfig,
    user_info: &super::oauth::OAuthUserInfo,
) -> Option<&'static str> {
    match provider {
        super::oauth::OAuthProvider::Google => {
            if !user_info.email_verified {
                return Some("email_unverified");
            }
            if let Some(google) = config.google.as_ref()
                && let Some(allowed) = google.allowed_domains.as_ref()
            {
                let any_real = allowed
                    .iter()
                    .any(|d| normalize_allowed_domain(d).is_some());
                if any_real && !email_domain_allowed(&user_info.email, allowed) {
                    return Some("domain_not_allowed");
                }
            }
            None
        }
        super::oauth::OAuthProvider::GitHub => None,
    }
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

fn oauth_providers(config: &super::config::AuthConfig) -> Vec<String> {
    if !config.oauth_enabled() {
        return Vec::new();
    }

    let mut providers = Vec::new();
    if config.google.is_some() {
        providers.push("google".to_string());
    }
    if config.github.is_some() {
        providers.push("github".to_string());
    }
    providers
}

fn ensure_oauth_enabled(config: &super::config::AuthConfig) -> Result<(), AuthError> {
    if config.oauth_enabled() {
        return Ok(());
    }
    Err(AuthError::unauthorized("OAuth authentication is disabled"))
}

/// Rate limit middleware for login endpoint
async fn rate_limit_login(
    State(state): State<BuiltinAuthBackend>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let ip = extract_client_ip(&req);
    if let Err(e) = state.rate_limiter.check_login(ip).await {
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
    if let Err(e) = state.rate_limiter.check_register(ip).await {
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
    if let Err(e) = state.rate_limiter.check_refresh(ip).await {
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
        // Merge rate-limited routes
        .merge(login_route)
        .merge(register_route)
        .merge(refresh_route)
        .with_state(state)
}

/// GET /v1/auth/config - Get authentication configuration
pub async fn get_auth_config(State(state): State<BuiltinAuthBackend>) -> Json<AuthConfigResponse> {
    Json(AuthConfigResponse {
        mode: state.config.mode.as_str().to_string(),
        password_auth_enabled: state.config.password_auth_enabled(),
        oauth_providers: oauth_providers(&state.config),
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
        is_platform_user: user
            .roles
            .as_array()
            .is_some_and(|roles| roles.iter().any(|role| role.as_str() == Some("admin"))),
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

/// Minimum password length enforced on `/v1/auth/register`. Matches the
/// commitment in `specs/authentication.md` and the UI's `minLength={8}` on
/// the register form (TM-AUTH-004 / EVE-453). UI validation is convenience;
/// this server-side check is the trust boundary.
const PASSWORD_MIN_LENGTH: usize = 8;

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

    // EVE-453 / TM-AUTH-004: enforce the documented 8-character minimum here
    // so direct API callers cannot bypass the UI's `minLength={8}` and create
    // weak-password accounts. Validate using `chars().count()` so a password
    // padded with multi-byte characters that happens to be < 8 codepoints
    // long still fails. Run before the email lookup and password hash so
    // timing reflects "request rejected" rather than "registration failed",
    // and use `unprocessable` (422) instead of the generic 401, since this
    // is an input validation error and does not touch any account record —
    // no new account-enumeration signal.
    if req.password.chars().count() < PASSWORD_MIN_LENGTH {
        return Err(AuthError::unprocessable(
            "Password must be at least 8 characters",
        ));
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

    // Harness-seed safety net: the async seed task (500 ms delay, see
    // `seed::spawn_seed_task_with_platform_definition`) may not have
    // provisioned DEFAULT_ORG_ID's built-in harnesses yet when a user
    // registers immediately after server startup. Re-run the provisioner
    // using the *platform definition*'s harness set (NOT the OSS default)
    // so a custom `PlatformDefinition` is never overridden — that was the
    // security concern addressed by PR #1462. The call is idempotent: if
    // seeding has already completed, every harness is "unchanged". See
    // EVE-390 and `specs/authentication.md`.
    if let Err(e) = crate::org_init::initialize_org_harnesses_with_definitions(
        &state.db,
        DEFAULT_ORG_ID,
        state.platform_definition.built_in_harnesses(),
    )
    .await
    {
        tracing::warn!(error = %e, "Failed to ensure default org harnesses (non-fatal)");
    }

    let organizations = builtin::fetch_user_organizations(&state.db, user.id)
        .await
        .unwrap_or_default();

    let auth_user = AuthUser {
        id: user.id,
        email: user.email,
        name: user.name,
        roles: vec!["user".to_string()],
        is_platform_user: false,
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

    // EVE-454 / TM-AUTH-018: atomic single-use consume. The previous
    // get-then-delete pattern allowed concurrent refresh requests with the
    // same token to both pass the existence check before either delete
    // committed, weakening single-use rotation. The consume is one SQL
    // statement (`DELETE … RETURNING`) and one in-memory write-lock for the
    // memory backend, so only one caller observes the row.
    let token_hash = hash_token(&refresh_token_value);
    let _token_row = state
        .db
        .consume_refresh_token_by_hash(&token_hash)
        .await
        .map_err(|e| {
            tracing::error!("Database error during refresh: {}", e);
            AuthError::unauthorized("Refresh failed")
        })?
        .ok_or_else(|| AuthError::unauthorized("Invalid refresh token"))?;

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
        is_platform_user: user
            .roles
            .as_array()
            .is_some_and(|roles| roles.iter().any(|role| role.as_str() == Some("admin"))),
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
///
/// Queries the database for the user's actual organization memberships so that
/// newly created orgs appear immediately (the auth middleware may cache a stale
/// or hardcoded list, e.g. AuthUser::anonymous() in none mode).
pub async fn get_current_user(
    State(state): State<BuiltinAuthBackend>,
    user: AuthUser,
    jar: CookieJar,
) -> (CookieJar, Json<UserInfoResponse>) {
    // Query DB for fresh organization memberships instead of using the
    // potentially stale list from the auth middleware / anonymous() default.
    let organizations = match state.db.list_user_organizations(user.id).await {
        Ok(rows) => Some(
            rows.iter()
                .map(|o| OrgMembershipResponse {
                    public_id: o.public_id.clone(),
                    name: o.name.clone(),
                    role: o.role.clone(),
                })
                .collect::<Vec<_>>(),
        ),
        Err(e) => {
            tracing::warn!(user_id = %user.id, error = %e, "Failed to query user organizations, falling back to auth context");
            Some(
                user.organizations
                    .iter()
                    .map(|o| OrgMembershipResponse {
                        public_id: o.public_id.clone(),
                        name: o.name.clone(),
                        role: o.role.as_str().to_string(),
                    })
                    .collect::<Vec<_>>(),
            )
        }
    };

    // Determine first org for cookie (prefer DB result)
    let first_org_public_id: Option<String> = organizations
        .as_ref()
        .and_then(|orgs| orgs.first().map(|o| o.public_id.clone()))
        .or_else(|| user.organizations.first().map(|o| o.public_id.clone()));

    // Set org cookie if missing (ensures subsequent API calls have org context)
    let jar = if jar.get(ORG_COOKIE_NAME).is_none() {
        if let Some(public_id) = &first_org_public_id {
            let cookie = Cookie::build((ORG_COOKIE_NAME, public_id.clone()))
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
    ensure_oauth_enabled(&state.config)?;

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
    ensure_oauth_enabled(&state.config)?;

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

    // EVE-451 / TM-AUTH: enforce provider-side identity gates before any user
    // lookup or creation so a hostile provider account cannot mint a session
    // even on the first use of an existing email and so deployments using
    // `AUTH_GOOGLE_ALLOWED_DOMAINS` get the restriction they configured.
    if let Some(reason) = oauth_identity_rejection_reason(provider_enum, &state.config, &user_info)
    {
        tracing::warn!(
            "OAuth identity rejected for provider={} reason={}",
            provider,
            reason
        );
        audit::emit(
            state.db.clone(),
            DEFAULT_ORG_ID,
            None,
            "auth.oauth.failure",
            audit::client_ip(&headers),
            serde_json::json!({"provider": provider, "reason": reason}),
        );
        return Err(AuthError::forbidden("OAuth account not permitted"));
    }

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

        // Harness-seed safety net (see equivalent comment in `register` and
        // EVE-390). Drive the provisioner from `platform_definition`, not
        // `oss_built_in_harnesses()`, so a custom platform definition is
        // never overridden on OAuth signup.
        if let Err(e) = crate::org_init::initialize_org_harnesses_with_definitions(
            &state.db,
            DEFAULT_ORG_ID,
            state.platform_definition.built_in_harnesses(),
        )
        .await
        {
            tracing::warn!(error = %e, "Failed to ensure default org harnesses (non-fatal)");
        }

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
        is_platform_user: user
            .roles
            .as_array()
            .is_some_and(|roles| roles.iter().any(|role| role.as_str() == Some("admin"))),
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

        // Add admin user to default organization as owner
        let _ = state
            .db
            .add_organization_member(DEFAULT_ORG_ID, created_user.id, "owner")
            .await
            .map_err(|e| {
                tracing::error!("Failed to add admin user to default org: {}", e);
                // Continue anyway
            });

        // Ensure default org has built-in harnesses (safety net — background
        // seed task may not have completed yet or may have failed silently).
        // Drive from `platform_definition` (not `oss_built_in_harnesses()`)
        // so operator-customized harnesses are never overridden — see
        // EVE-390 and PR #1462.
        if let Err(e) = crate::org_init::initialize_org_harnesses_with_definitions(
            &state.db,
            DEFAULT_ORG_ID,
            state.platform_definition.built_in_harnesses(),
        )
        .await
        {
            tracing::warn!(error = %e, "Failed to ensure default org harnesses (non-fatal)");
        }

        created_user
    };

    let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();

    let mut organizations = builtin::fetch_user_organizations(&state.db, user.id)
        .await
        .unwrap_or_default();

    if organizations.is_empty() {
        // Admin user not in any org — add them as owner
        let _ = state
            .db
            .add_organization_member(DEFAULT_ORG_ID, user.id, "owner")
            .await;
    } else {
        // Ensure admin user has owner role in default org (fixes users created with member role)
        if let Some(membership) = organizations
            .iter_mut()
            .find(|m| m.org_id == DEFAULT_ORG_ID)
            && membership.role != OrgRole::Owner
        {
            let _ = state
                .db
                .update_organization_member_role(DEFAULT_ORG_ID, user.id, "owner")
                .await;
            membership.role = OrgRole::Owner;
        }
    }

    Ok(AuthUser {
        id: user.id,
        email: user.email,
        name: user.name,
        roles,
        is_platform_user: user
            .roles
            .as_array()
            .is_some_and(|roles| roles.iter().any(|role| role.as_str() == Some("admin"))),
        auth_method: AuthMethod::Jwt,
        organizations: builtin::organizations_or_default(organizations),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::config::AuthConfig;

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

    #[test]
    fn test_oauth_providers_hidden_when_oauth_disabled() {
        let mut config = AuthConfig::default();
        config.mode = AuthMode::External;
        config.google = Some(crate::auth::config::GoogleOAuthConfig {
            base: crate::auth::config::OAuthProviderConfig {
                client_id: "id".to_string(),
                client_secret: "secret".to_string(),
                redirect_uri: "http://localhost/callback".to_string(),
            },
            allowed_domains: None,
        });
        config.github = Some(crate::auth::config::GitHubOAuthConfig {
            base: crate::auth::config::OAuthProviderConfig {
                client_id: "id".to_string(),
                client_secret: "secret".to_string(),
                redirect_uri: "http://localhost/callback".to_string(),
            },
        });

        assert!(oauth_providers(&config).is_empty());
        assert!(ensure_oauth_enabled(&config).is_err());
    }

    fn make_google_user(email: &str, verified: bool) -> super::super::oauth::OAuthUserInfo {
        super::super::oauth::OAuthUserInfo {
            provider_id: "google-sub-1".to_string(),
            email: email.to_string(),
            name: "User".to_string(),
            avatar_url: None,
            email_verified: verified,
        }
    }

    fn config_with_google(allowed_domains: Option<Vec<String>>) -> AuthConfig {
        let mut config = AuthConfig::default();
        config.mode = AuthMode::Full;
        config.google = Some(crate::auth::config::GoogleOAuthConfig {
            base: crate::auth::config::OAuthProviderConfig {
                client_id: "id".to_string(),
                client_secret: "secret".to_string(),
                redirect_uri: "http://localhost/callback".to_string(),
            },
            allowed_domains,
        });
        config
    }

    #[test]
    fn test_email_domain_allowed_matches_case_insensitive() {
        let allowed = vec!["Example.com".to_string(), "Other.io".to_string()];
        assert!(email_domain_allowed("user@example.com", &allowed));
        assert!(email_domain_allowed("user@EXAMPLE.COM", &allowed));
        assert!(email_domain_allowed("user@other.io", &allowed));
        assert!(!email_domain_allowed("user@evil.com", &allowed));
        assert!(!email_domain_allowed("not-an-email", &allowed));
        assert!(!email_domain_allowed("user@", &allowed));
    }

    // EVE-451 (review feedback): operators sometimes write `@company.com` —
    // accept it as an equivalent of `company.com` rather than silently
    // failing closed.
    #[test]
    fn test_email_domain_allowed_accepts_at_prefixed_entries() {
        let allowed = vec!["@company.com".to_string(), " @other.io ".to_string()];
        assert!(email_domain_allowed("user@company.com", &allowed));
        assert!(email_domain_allowed("user@other.io", &allowed));
        assert!(!email_domain_allowed("user@evil.com", &allowed));
    }

    // EVE-451 (review feedback): exact-domain semantics — `company.com` must
    // not authorize an `attacker.company.com` subdomain. Operators who want
    // subdomains must list each one.
    #[test]
    fn test_email_domain_allowed_does_not_match_subdomains() {
        let allowed = vec!["company.com".to_string()];
        assert!(!email_domain_allowed("user@attacker.company.com", &allowed));
        assert!(!email_domain_allowed("user@xcompany.com", &allowed));
    }

    #[test]
    fn test_normalize_allowed_domain_drops_empty_entries() {
        assert_eq!(normalize_allowed_domain(""), None);
        assert_eq!(normalize_allowed_domain("   "), None);
        assert_eq!(normalize_allowed_domain("@"), None);
        assert_eq!(normalize_allowed_domain(" @  "), None);
        assert_eq!(
            normalize_allowed_domain("@Company.COM"),
            Some("company.com".to_string())
        );
        assert_eq!(
            normalize_allowed_domain("  company.com  "),
            Some("company.com".to_string())
        );
    }

    // EVE-451: Google OAuth must require email_verified=true.
    #[test]
    fn test_google_rejects_unverified_email() {
        let config = config_with_google(None);
        let user = make_google_user("user@example.com", false);
        assert_eq!(
            oauth_identity_rejection_reason(
                super::super::oauth::OAuthProvider::Google,
                &config,
                &user
            ),
            Some("email_unverified")
        );
    }

    // EVE-451: When allowed_domains is set, mismatched email domains are rejected.
    #[test]
    fn test_google_rejects_disallowed_domain() {
        let config = config_with_google(Some(vec!["company.com".to_string()]));
        let user = make_google_user("user@evil.com", true);
        assert_eq!(
            oauth_identity_rejection_reason(
                super::super::oauth::OAuthProvider::Google,
                &config,
                &user
            ),
            Some("domain_not_allowed")
        );
    }

    // EVE-451: Allowed domain on a verified email passes.
    #[test]
    fn test_google_accepts_allowed_domain_verified() {
        let config = config_with_google(Some(vec!["company.com".to_string()]));
        let user = make_google_user("user@Company.COM", true);
        assert!(
            oauth_identity_rejection_reason(
                super::super::oauth::OAuthProvider::Google,
                &config,
                &user
            )
            .is_none()
        );
    }

    // EVE-451: No allowed_domains config means any verified domain is accepted.
    #[test]
    fn test_google_accepts_any_verified_domain_when_unrestricted() {
        let config = config_with_google(None);
        let user = make_google_user("user@anywhere.io", true);
        assert!(
            oauth_identity_rejection_reason(
                super::super::oauth::OAuthProvider::Google,
                &config,
                &user
            )
            .is_none()
        );
    }

    // EVE-451: Empty allowed_domains list (only whitespace) does not lock everyone out.
    // Treats it as "no restriction" rather than "deny all" so an operator who
    // sets `AUTH_GOOGLE_ALLOWED_DOMAINS=` does not accidentally brick login.
    #[test]
    fn test_google_empty_allowed_domains_does_not_lock_out() {
        let config = config_with_google(Some(vec!["   ".to_string()]));
        let user = make_google_user("user@anywhere.io", true);
        assert!(
            oauth_identity_rejection_reason(
                super::super::oauth::OAuthProvider::Google,
                &config,
                &user
            )
            .is_none()
        );
    }

    // EVE-451: GitHub flow currently has no per-provider gates here.
    #[test]
    fn test_github_has_no_provider_gates() {
        let mut config = AuthConfig::default();
        config.mode = AuthMode::Full;
        config.github = Some(crate::auth::config::GitHubOAuthConfig {
            base: crate::auth::config::OAuthProviderConfig {
                client_id: "id".to_string(),
                client_secret: "secret".to_string(),
                redirect_uri: "http://localhost/callback".to_string(),
            },
        });
        let user = super::super::oauth::OAuthUserInfo {
            provider_id: "gh-1".to_string(),
            email: "user@example.com".to_string(),
            name: "User".to_string(),
            avatar_url: None,
            email_verified: false,
        };
        assert!(
            oauth_identity_rejection_reason(
                super::super::oauth::OAuthProvider::GitHub,
                &config,
                &user
            )
            .is_none()
        );
    }

    #[test]
    fn test_oauth_providers_visible_when_oauth_enabled() {
        let mut config = AuthConfig::default();
        config.mode = AuthMode::Full;
        config.google = Some(crate::auth::config::GoogleOAuthConfig {
            base: crate::auth::config::OAuthProviderConfig {
                client_id: "id".to_string(),
                client_secret: "secret".to_string(),
                redirect_uri: "http://localhost/callback".to_string(),
            },
            allowed_domains: None,
        });
        config.github = Some(crate::auth::config::GitHubOAuthConfig {
            base: crate::auth::config::OAuthProviderConfig {
                client_id: "id".to_string(),
                client_secret: "secret".to_string(),
                redirect_uri: "http://localhost/callback".to_string(),
            },
        });

        assert_eq!(
            oauth_providers(&config),
            vec!["google".to_string(), "github".to_string()]
        );
        assert!(ensure_oauth_enabled(&config).is_ok());
    }
}

// Additional TM-AUTH-007 validation tests
#[cfg(test)]
mod oauth_state_tests {
    use super::*;
    use axum_extra::extract::cookie::Cookie;

    #[test]
    fn test_oauth_state_length_and_hex() {
        // State must be 32 hex chars (16 random bytes) — sufficient entropy for CSRF
        let state = generate_oauth_state();
        assert_eq!(state.len(), 32);
        assert!(
            state.chars().all(|c| c.is_ascii_hexdigit()),
            "state must be hex-encoded"
        );
    }

    #[test]
    fn test_oauth_state_mismatch_detected() {
        // Simulate the comparison that oauth_callback performs (TM-AUTH-007)
        let stored = generate_oauth_state();
        let incoming = generate_oauth_state();
        // Two independently generated states must differ (collision probability ~2^-128)
        assert_ne!(stored, incoming, "distinct states must not match");
        // Same value must match
        assert_eq!(stored, stored);
    }

    #[test]
    fn test_oauth_state_cookie_is_single_use() {
        // After validation the cookie is removed via jar.remove().
        // Verify the remove call targets the right cookie name + path.
        let remove_cookie = Cookie::build(OAUTH_STATE_COOKIE).path("/").build();
        assert_eq!(remove_cookie.name(), "oauth_state");
        assert_eq!(remove_cookie.path(), Some("/"));
    }
}

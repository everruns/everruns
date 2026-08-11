// Authentication HTTP routes
// Decision: Use /v1/auth/* prefix for all auth endpoints (consistent with other API routes)
// Decision: Support both JSON and cookie-based sessions

use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, Extension, FromRef, Path, Query, State},
    http::{HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{Redirect, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{Duration, Utc};
use everruns_core::{DEFAULT_ORG_ID, OrgRole};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
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
use crate::security::constant_time_eq;
/// Enable AuthUser extractor when BuiltinAuthBackend is the route state.
/// AuthUser needs AuthState via FromRef — this converts BuiltinAuthBackend to AuthState.
impl FromRef<BuiltinAuthBackend> for AuthState {
    fn from_ref(backend: &BuiltinAuthBackend) -> Self {
        AuthState::new(backend.config.clone(), std::sync::Arc::new(backend.clone()))
    }
}

use crate::storage::{
    models::{CreateRefreshTokenRow, CreateUserRow, UserRow},
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
/// the email domain must be in that list. GitHub: must report `email_verified`
/// (derived from the real provider flag, not hardcoded — see EVE-702) so an
/// attacker cannot pre-empt an account with an unverified GitHub address.
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
        super::oauth::OAuthProvider::GitHub => {
            if !user_info.email_verified {
                return Some("email_unverified");
            }
            None
        }
    }
}

/// Returns the audit reason when an existing same-email account must not be
/// auto-linked to an OAuth identity. Verified OAuth proves the callback caller
/// owns the mailbox now, but an unverified password account may have been
/// pre-created by an attacker who never controlled that mailbox. Likewise,
/// provider verification alone is insufficient to link two OAuth identities:
/// stale or reassigned email claims could otherwise grant cross-provider access.
fn existing_oauth_link_rejection_reason(
    existing: &UserRow,
    provider: &str,
) -> Option<&'static str> {
    if matches!(
        existing.auth_provider.as_deref(),
        Some(existing_provider) if existing_provider != "local" && existing_provider != provider
    ) {
        return Some("email_bound_to_different_provider");
    }

    if !existing.email_verified {
        return Some("existing_email_unverified");
    }

    None
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
    /// Optional human-readable name. Minimal signup only asks for email +
    /// password; when omitted (or blank) the display name is derived from the
    /// email local-part. Safe to render in user-facing messages.
    #[serde(default)]
    pub name: Option<String>,
    /// Captcha token, required when `/v1/auth/config` advertises a captcha.
    #[serde(default)]
    pub captcha_token: Option<String>,
}

/// Derive a friendly display name from an email address when the client does
/// not supply one (minimal signup flow). Uses the local-part (before `@`),
/// splits on `.`/`_`/`-`, capitalizes each word, and joins with spaces:
/// `eli@acme.com` → "Eli", `eli.wong@x.com` → "Eli Wong". Falls back to the
/// raw email if the local-part yields nothing usable.
fn display_name_from_email(email: &str) -> String {
    let local = email.split('@').next().unwrap_or(email);
    let derived = local
        .split(['.', '_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if derived.trim().is_empty() {
        email.to_string()
    } else {
        derived
    }
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
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub public_id: String,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    pub role: String,
}

/// User info response
#[derive(Debug, Serialize, ToSchema)]
pub struct UserInfoResponse {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
    pub email: String,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    pub roles: Vec<String>,
    pub avatar_url: Option<String>,
    /// Whether the account's email address has been verified. Drives the
    /// in-app "verify your email" nudge so a signed-in but unverified user has
    /// a surfaced path to verification (auth-flow dead-end audit).
    pub email_verified: bool,
    /// Organizations the user belongs to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organizations: Option<Vec<OrgMembershipResponse>>,
}

/// Refresh token request
#[derive(Debug, Deserialize, ToSchema)]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

/// Request to start a password reset (or resend a verification email).
/// Only the email is supplied; the response is intentionally identical
/// regardless of whether the account exists (account-enumeration safety).
#[derive(Debug, Deserialize, ToSchema)]
pub struct EmailOnlyRequest {
    pub email: String,
    /// Captcha token, required when `/v1/auth/config` advertises a captcha.
    #[serde(default)]
    pub captcha_token: Option<String>,
}

/// Request to complete a password reset with the emailed token.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub password: String,
}

/// Request to verify an email address with the emailed token.
#[derive(Debug, Deserialize, ToSchema)]
pub struct VerifyEmailRequest {
    pub token: String,
}

/// Generic success body. Account-recovery endpoints return this with no
/// account-specific detail so callers cannot probe which emails are registered.
#[derive(Debug, Serialize, ToSchema)]
pub struct OkResponse {
    pub ok: bool,
}

impl OkResponse {
    fn ok() -> Json<Self> {
        Json(Self { ok: true })
    }
}

/// Cookie name for OAuth CSRF state (TM-AUTH-007)
const OAUTH_STATE_COOKIE: &str = "oauth_state";

/// OAuth callback query parameters. `code`/`state` are optional so a
/// provider error callback (e.g. `?error=access_denied` when the user
/// cancels the consent screen) is handled by the branded redirect flow
/// instead of failing axum's extractor with a bare 400.
#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    /// Provider-reported error code, if the provider redirected with one.
    pub error: Option<String>,
}

/// Auth configuration response
#[derive(Debug, Serialize, ToSchema)]
pub struct AuthConfigResponse {
    pub mode: String,
    /// Trusted configured origin hosting the login page. Absent means the
    /// frontend's same-origin `/login` route.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_origin: Option<String>,
    pub password_auth_enabled: bool,
    pub oauth_providers: Vec<String>,
    pub signup_enabled: bool,
    /// True when email/password signup ends at a "check your email"
    /// confirmation instead of an instant session (AUTH_SIGNUP_EMAIL_CONFIRM).
    pub signup_email_confirm: bool,
    /// Bot-mitigation challenge the UI must solve on abuse-prone auth forms
    /// (register / forgot-password / resend-verification). Absent when no
    /// captcha is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captcha: Option<CaptchaConfigResponse>,
}

/// Public captcha configuration (site key only — never the secret).
#[derive(Debug, Serialize, ToSchema)]
pub struct CaptchaConfigResponse {
    /// Challenge provider; currently always `turnstile`.
    pub provider: String,
    pub site_key: String,
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

    // Account-recovery routes. These are reused enumeration-safe email and
    // token flows. They send email (forgot-password / resend-verification) or
    // mutate credentials (reset-password / verify-email), so they share the
    // register rate limiter to throttle abuse per client IP.
    let recovery_routes = Router::new()
        .route("/v1/auth/forgot-password", post(forgot_password))
        .route("/v1/auth/reset-password", post(reset_password))
        .route("/v1/auth/verify-email", post(verify_email))
        .route("/v1/auth/resend-verification", post(resend_verification))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_register,
        ));

    // OAuth endpoints share the login limiter: the redirect mints state
    // cookies and the callback performs an outbound token exchange plus DB
    // writes per hit — neither should be free to flood.
    let oauth_routes = Router::new()
        .route("/v1/auth/oauth/{provider}", get(oauth_redirect))
        .route("/v1/auth/callback/{provider}", get(oauth_callback))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_login,
        ));

    Router::new()
        // Public routes (no rate limit needed)
        .route("/v1/auth/config", get(get_auth_config))
        .route("/v1/auth/logout", post(logout))
        // Protected routes
        .route("/v1/auth/me", get(get_current_user))
        // Merge rate-limited routes
        .merge(login_route)
        .merge(register_route)
        .merge(refresh_route)
        .merge(recovery_routes)
        .merge(oauth_routes)
        .with_state(state)
}

/// GET /v1/auth/config - Get authentication configuration
pub async fn get_auth_config(State(state): State<BuiltinAuthBackend>) -> Json<AuthConfigResponse> {
    Json(AuthConfigResponse {
        mode: state.config.mode.as_str().to_string(),
        login_origin: state.config.login_origin.clone(),
        password_auth_enabled: state.config.password_auth_enabled(),
        oauth_providers: oauth_providers(&state.config),
        signup_enabled: state.config.signup_enabled(),
        signup_email_confirm: state.config.signup_email_confirm,
        captcha: state
            .config
            .turnstile
            .as_ref()
            .map(|t| CaptchaConfigResponse {
                provider: "turnstile".to_string(),
                site_key: t.site_key.clone(),
            }),
    })
}

/// Enforce the configured captcha on an abuse-prone auth endpoint. No-op when
/// no captcha is configured. Fail closed: a missing/invalid token is a
/// generic 403 (independent of account existence, so not an enumeration
/// oracle); a siteverify outage is a 503 the client may retry.
async fn enforce_auth_captcha(
    state: &BuiltinAuthBackend,
    token: Option<&str>,
    remote_ip: Option<std::net::IpAddr>,
) -> Result<(), AuthError> {
    let Some(turnstile) = state.config.turnstile.as_ref() else {
        return Ok(());
    };
    let outcome = crate::api::turnstile::TurnstileVerifier::default()
        .verify(&turnstile.secret_key, token.unwrap_or(""), remote_ip)
        .await;
    match outcome {
        crate::api::turnstile::TurnstileOutcome::Allowed => Ok(()),
        crate::api::turnstile::TurnstileOutcome::Rejected => Err(AuthError::forbidden(
            "Verification failed. Please try again.",
        )),
        crate::api::turnstile::TurnstileOutcome::Unavailable => Err(AuthError::internal(
            "Verification is temporarily unavailable. Please try again.",
        )),
    }
}

/// POST /v1/auth/login - Login with email and password
pub async fn login(
    State(state): State<BuiltinAuthBackend>,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> Result<(CookieJar, Json<TokenResponse>), AuthError> {
    let ip = audit::client_ip_from_connect_info(connect_info, &headers);

    // In admin mode, check admin credentials directly (no database lookup)
    if state.config.mode == AuthMode::Admin {
        // Constant-time comparison of both email and password, combined with a
        // non-short-circuiting `&`, so login timing does not reveal which field
        // matched or how many leading bytes of the password were correct.
        if let Some(admin) = &state.config.admin
            && (constant_time_eq(req.email.as_bytes(), admin.email.as_bytes())
                & constant_time_eq(req.password.as_bytes(), admin.password.as_bytes()))
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

    // TM-AUTH-001: per-account throttle across ALL source IPs — the per-IP
    // middleware alone leaves a single account open to distributed credential
    // stuffing. Keyed on the submitted email (no DB read, so unknown emails
    // are throttled identically — no enumeration signal).
    if state
        .rate_limiter
        .check_account_login(&req.email.trim().to_lowercase())
        .await
        .is_err()
    {
        audit::emit(
            state.db.clone(),
            DEFAULT_ORG_ID,
            None,
            "auth.login.rate_limited",
            ip,
            serde_json::json!({"scope": "account"}),
        );
        return Err(AuthError::too_many_requests(
            "Too many attempts. Please try again later.",
        ));
    }

    // Cap pre-hash work: no legitimate password exceeds PASSWORD_MAX_LENGTH
    // (register enforces it), so never feed oversized inputs to Argon2.
    // Same generic failure as any bad credential — no oracle.
    if req.password.len() > PASSWORD_MAX_BYTES {
        audit::emit(
            state.db.clone(),
            DEFAULT_ORG_ID,
            None,
            "auth.login.failure",
            ip,
            serde_json::json!({"reason": "password_too_long"}),
        );
        return Err(AuthError::unauthorized("Invalid email or password"));
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

    // EVE-452 / TM-AUTH-019: account-enumeration via login error differences.
    // OAuth-only accounts (no `password_hash`) used to receive a distinct
    // "Password login not available for this account" message, letting an
    // attacker tell OAuth-registered emails apart from unknown emails or
    // password-backed login failures. Return the same generic
    // `Invalid email or password` for *all* credential failure paths so the
    // UI cannot leak the difference even if it renders the raw message.
    let Some(password_hash) = user.password_hash.as_ref() else {
        audit::emit(
            state.db.clone(),
            DEFAULT_ORG_ID,
            Some(user.id),
            "auth.login.failure",
            ip,
            serde_json::json!({"reason": "no_password_hash"}),
        );
        return Err(AuthError::unauthorized("Invalid email or password"));
    };

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
        organizations,
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
/// commitment in `knowledge/security/authentication.md` and the UI's `minLength={8}` on
/// the register form (TM-AUTH-004 / EVE-453). UI validation is convenience;
/// this server-side check is the trust boundary.
const PASSWORD_MIN_LENGTH: usize = 12;

/// Maximum password length (characters) accepted on register / reset.
/// Argon2's per-hash work grows with input size, so unbounded passwords are a
/// cheap DoS lever; 128 chars comfortably exceeds any real passphrase (NIST
/// asks that at least 64 be allowed).
const PASSWORD_MAX_LENGTH: usize = 128;

/// Byte-level guard for the login path (UTF-8 can be up to 4 bytes/char).
/// Anything larger cannot be a registered password, so it is rejected with
/// the generic credential failure before any hashing work.
const PASSWORD_MAX_BYTES: usize = PASSWORD_MAX_LENGTH * 4;

/// Policy for any NEWLY SET password (signup + reset): at least 12 codepoints,
/// at most 128, and at least one ASCII digit. Existing passwords are never
/// re-validated — login is unaffected. `chars().count()` so multi-byte
/// padding cannot cheat the minimum.
fn validate_new_password(password: &str) -> Result<(), AuthError> {
    if password.chars().count() < PASSWORD_MIN_LENGTH {
        return Err(AuthError::unprocessable(
            "Password must be at least 12 characters",
        ));
    }
    if password.chars().count() > PASSWORD_MAX_LENGTH {
        return Err(AuthError::unprocessable(
            "Password must be at most 128 characters",
        ));
    }
    if !password.chars().any(|c| c.is_ascii_digit()) {
        return Err(AuthError::unprocessable(
            "Password must include at least one number",
        ));
    }
    Ok(())
}

/// Generic registration-failure message returned to the client on every
/// signup rejection that touches an account record — password register and
/// OAuth signup alike. It must never disclose whether an account with a given
/// email already exists, otherwise the signup endpoints become an
/// account-enumeration oracle (EVE-632 / TM-AUTH-014). The precise reason is
/// logged server-side only.
const GENERIC_REGISTRATION_FAILED: &str = "Registration failed";

/// Outcome of `register`, shaped by `signup_email_confirm`:
/// - `Session`: classic instant-session signup (201 + cookies + tokens).
/// - `ConfirmationSent`: confirm mode — the account may or may not have been
///   created, no session exists yet, and the body is the same generic
///   `{ok:true}` either way (anti-enumeration; the emailed link is the only
///   place the two cases diverge).
#[derive(Debug)]
pub enum RegisterOutcome {
    Session(StatusCode, CookieJar, Json<TokenResponse>),
    ConfirmationSent(Json<OkResponse>),
}

impl axum::response::IntoResponse for RegisterOutcome {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::Session(status, jar, json) => (status, jar, json).into_response(),
            Self::ConfirmationSent(json) => (StatusCode::OK, json).into_response(),
        }
    }
}

/// POST /v1/auth/register - Register a new user
pub async fn register(
    State(state): State<BuiltinAuthBackend>,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<RegisterRequest>,
) -> Result<RegisterOutcome, AuthError> {
    let ip = audit::client_ip_from_connect_info(connect_info, &headers);

    // Check if signup is enabled. Admin mode accepts password login for the
    // configured administrator, but must not expose public self-registration.
    if !state.config.signup_enabled() {
        return Err(AuthError::forbidden("Registration is disabled"));
    }

    // Check if password auth is enabled
    if !state.config.password_auth_enabled() {
        return Err(AuthError::forbidden("Password registration is disabled"));
    }

    // Bot gate (when configured) before any account work.
    enforce_auth_captcha(&state, req.captcha_token.as_deref(), None).await?;

    // TM-AUTH-004: enforce the password policy server-side so direct API
    // callers cannot bypass the UI's client-side checks. Runs before the
    // email lookup and password hash — input validation only, no account
    // record touched, so 422 here is not an enumeration signal.
    validate_new_password(&req.password)?;

    // Hash password first to make timing consistent whether or not the email exists.
    // This prevents account enumeration via response-time differences (TM-AUTH-014).
    let password_hash = hash_password(&req.password).map_err(|e| {
        tracing::error!("Password hashing error: {}", e);
        AuthError::unauthorized(GENERIC_REGISTRATION_FAILED)
    })?;

    // Check if user already exists — generic error to prevent account enumeration
    let existing = state.db.get_user_by_email(&req.email).await.map_err(|e| {
        tracing::error!("Database error during registration: {}", e);
        AuthError::unauthorized(GENERIC_REGISTRATION_FAILED)
    })?;

    if existing.is_some() {
        if state.config.signup_email_confirm {
            // Same generic success as a fresh signup; the divergence lives in
            // the email body only. Budgeted like all account emails.
            if state
                .rate_limiter
                .check_account_email_send(&req.email.trim().to_lowercase())
                .await
                .is_ok()
            {
                send_account_exists_email(&state, &req.email).await;
            }
            audit::emit(
                state.db.clone(),
                DEFAULT_ORG_ID,
                None,
                "auth.register.existing_email",
                ip.clone(),
                serde_json::json!({}),
            );
            return Ok(RegisterOutcome::ConfirmationSent(OkResponse::ok()));
        }
        return Err(AuthError::unauthorized(GENERIC_REGISTRATION_FAILED));
    }

    // Minimal signup: name is optional. When absent or blank, derive a display
    // name from the email local-part so the account still has a friendly name.
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| display_name_from_email(&req.email));

    // Create user
    let user = state
        .db
        .create_user(CreateUserRow {
            email: req.email.clone(),
            name: name.clone(),
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
            AuthError::unauthorized(GENERIC_REGISTRATION_FAILED)
        })?;

    // Add user to the default organization — single-tenant convenience only.
    // Gated on `auto_join_default_org` (off by default): in a multi-tenant
    // deployment a fresh signup must own no org so the zero-org onboarding flow
    // creates the user's own org, instead of every tenant landing in the shared
    // default organization. See `knowledge/security/authentication.md`.
    if state.config.auto_join_default_org {
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
        // EVE-390 and `knowledge/security/authentication.md`.
        if let Err(e) = crate::org_init::initialize_org_harnesses_with_definitions(
            &state.db,
            DEFAULT_ORG_ID,
            state.built_in_harnesses.as_slice(),
        )
        .await
        {
            tracing::warn!(error = %e, "Failed to ensure default org harnesses (non-fatal)");
        }
    }

    // Email verification: on successful signup, issue a single-use verification
    // token and email a confirmation link. Best-effort — a delivery failure or
    // unconfigured email provider must never fail registration, so the user can
    // still log in and verify later via /v1/auth/resend-verification. Called
    // before `user.email` is moved into `auth_user` below.
    issue_verification_email(&state, user.id, &user.email).await;

    if state.config.signup_email_confirm {
        // No session is minted during confirm-mode signup. The emailed link
        // only verifies the address; users sign in explicitly afterward.
        audit::emit(
            state.db.clone(),
            DEFAULT_ORG_ID,
            Some(user.id),
            "auth.register.pending_confirmation",
            ip.clone(),
            serde_json::json!({}),
        );
        return Ok(RegisterOutcome::ConfirmationSent(OkResponse::ok()));
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
        organizations,
    };

    audit::emit(
        state.db.clone(),
        DEFAULT_ORG_ID,
        Some(auth_user.id),
        "auth.register.success",
        ip,
        serde_json::json!({}),
    );

    let (jar, json) = generate_token_response(&state, jar, &auth_user).await?;
    Ok(RegisterOutcome::Session(StatusCode::CREATED, jar, json))
}

/// POST /v1/auth/refresh - Refresh access token
///
/// Accepts the refresh token from either the JSON body (`{ "refresh_token": "..." }`)
/// or the `refresh_token` HttpOnly cookie (set at login). Cookie-based is the
/// primary flow for browser clients since the cookie is HttpOnly.
pub async fn refresh_token(
    State(state): State<BuiltinAuthBackend>,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Option<Json<RefreshTokenRequest>>,
) -> Result<(CookieJar, Json<TokenResponse>), AuthError> {
    let ip = audit::client_ip_from_connect_info(connect_info, &headers);

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
        organizations,
    };

    audit::emit(
        state.db.clone(),
        DEFAULT_ORG_ID,
        Some(auth_user.id),
        "auth.token_refresh.success",
        ip,
        serde_json::json!({}),
    );

    generate_token_response(&state, jar, &auth_user).await
}

/// POST /v1/auth/logout - Logout (clear cookies + revoke the refresh token)
///
/// The refresh-token row is deleted server-side so a captured cookie is dead
/// after logout, not merely absent from this browser. The short-lived access
/// token (15 min) expires on its own (TM-AUTH-003). Best-effort: cookie
/// clearing must succeed even if the DB is unavailable.
pub async fn logout(State(state): State<BuiltinAuthBackend>, jar: CookieJar) -> CookieJar {
    if let Some(cookie) = jar.get("refresh_token") {
        let token_hash = hash_token(cookie.value());
        if let Err(e) = state.db.consume_refresh_token_by_hash(&token_hash).await {
            tracing::warn!(error = %e, "failed to revoke refresh token on logout");
        }
    }
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
                .max_age(super::middleware::ORG_COOKIE_MAX_AGE)
                .build();
            jar.add(cookie)
        } else {
            jar
        }
    } else {
        jar
    };

    // Fetch the verified flag from the DB row (AuthUser does not carry it).
    // Default to `true` when the row is absent (e.g. the anonymous user in
    // `none` mode) so we never nag a principal whose mailbox we can't check.
    let email_verified = state
        .db
        .get_user(user.id)
        .await
        .ok()
        .flatten()
        .map(|u| u.email_verified)
        .unwrap_or(true);

    (
        jar,
        Json(UserInfoResponse {
            id: user.id.to_string(),
            email: user.email,
            name: user.name,
            roles: user.roles,
            avatar_url: None,
            email_verified,
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

/// Build the branded failure redirect for the OAuth callback: back to the
/// unified login door with a coarse error category (never provider detail —
/// specifics stay in logs/audit only).
fn oauth_failure_redirect(config: &super::config::AuthConfig, category: &str) -> Redirect {
    let url = format!(
        "{}/login?error={category}",
        config.login_origin().trim_end_matches('/')
    );
    Redirect::to(&url)
}

/// GET /v1/auth/callback/:provider - OAuth callback
///
/// This endpoint is only ever hit by a browser redirected from the provider,
/// so failures redirect back to `/login?error=<category>` where the UI shows
/// a friendly generic message — a raw JSON error page here is a dead end.
/// Categories: `oauth_cancelled` (user denied consent), `oauth_not_permitted`
/// (identity gate: unverified email / domain not allowed),
/// `oauth_account_exists` (unsafe or conflicting identity link), `oauth_failed`
/// (everything else). The state cookie is cleared on every outcome.
pub async fn oauth_callback(
    State(state): State<BuiltinAuthBackend>,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Query(query): Query<OAuthCallbackQuery>,
    jar: CookieJar,
) -> (CookieJar, Redirect) {
    let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let cleared_jar = jar
        .clone()
        .remove(Cookie::build(OAUTH_STATE_COOKIE).path("/"));

    // Provider bounced back with an explicit error (e.g. the user cancelled).
    if let Some(err) = query.error.as_deref() {
        audit::emit(
            state.db.clone(),
            DEFAULT_ORG_ID,
            None,
            "auth.oauth.failure",
            audit::client_ip(peer_addr, &headers),
            serde_json::json!({"provider": provider, "reason": "provider_error", "error": err}),
        );
        let category = if err == "access_denied" {
            "oauth_cancelled"
        } else {
            "oauth_failed"
        };
        return (cleared_jar, oauth_failure_redirect(&state.config, category));
    }

    let (Some(code), Some(cb_state)) = (query.code.clone(), query.state.clone()) else {
        tracing::warn!("OAuth callback missing code or state parameter");
        return (
            cleared_jar,
            oauth_failure_redirect(&state.config, "oauth_failed"),
        );
    };

    match oauth_callback_inner(
        &state, peer_addr, &headers, &provider, &code, &cb_state, jar,
    )
    .await
    {
        Ok(ok) => ok,
        Err(e) => {
            // Map the failure onto a coarse, safe category the login page renders
            // as fixed copy. CONFLICT is the "your verified email already has an
            // account — use your original method" case: a PERMANENT condition, so
            // it must not fall into the generic "didn't complete, try again"
            // bucket that implies a transient error (auth-flow dead-end audit).
            let category = match e.status {
                StatusCode::FORBIDDEN => "oauth_not_permitted",
                StatusCode::CONFLICT => "oauth_account_exists",
                _ => "oauth_failed",
            };
            (cleared_jar, oauth_failure_redirect(&state.config, category))
        }
    }
}

/// The original callback body: validates CSRF state, exchanges the code,
/// applies identity gates, links or creates the account, and mints the
/// session. Errors bubble to `oauth_callback`, which maps them onto the
/// branded `/login?error=…` redirect.
async fn oauth_callback_inner(
    state: &BuiltinAuthBackend,
    peer_addr: Option<SocketAddr>,
    headers: &HeaderMap,
    provider: &str,
    code: &str,
    callback_state: &str,
    jar: CookieJar,
) -> Result<(CookieJar, Redirect), AuthError> {
    ensure_oauth_enabled(&state.config)?;

    let provider_enum = OAuthProvider::parse(provider)
        .ok_or_else(|| AuthError::unauthorized("Unknown OAuth provider"))?;

    // TM-AUTH-007: Validate CSRF state parameter
    let stored_state = jar
        .get(OAUTH_STATE_COOKIE)
        .map(|c| c.value().to_string())
        .ok_or_else(|| {
            tracing::warn!("OAuth callback missing state cookie (possible CSRF attempt)");
            AuthError::unauthorized("Invalid OAuth state")
        })?;

    if stored_state != callback_state {
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
            service.exchange_code(code).await
        }
        OAuthProvider::GitHub => {
            let config = state
                .config
                .github
                .as_ref()
                .ok_or_else(|| AuthError::unauthorized("GitHub OAuth not configured"))?;
            let service = GitHubOAuthService::new(config)
                .map_err(|_| AuthError::unauthorized("OAuth configuration error"))?;
            service.exchange_code(code).await
        }
    }
    .map_err(|e| {
        tracing::error!("OAuth exchange failed: {}", e);
        audit::emit(
            state.db.clone(),
            DEFAULT_ORG_ID,
            None,
            "auth.oauth.failure",
            audit::client_ip(peer_addr, headers),
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
            audit::client_ip(peer_addr, headers),
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

        if let Some(existing) = existing_user {
            // Only auto-link to an existing account whose email was already
            // verified by this service. Otherwise an attacker could pre-create
            // a local account for a victim email and retain its password/session
            // after the real mailbox owner later completes OAuth.
            if let Some(reason) = existing_oauth_link_rejection_reason(&existing, provider_str) {
                tracing::warn!(
                    provider = %provider_str,
                    existing_provider = existing.auth_provider.as_deref().unwrap_or(""),
                    reason = reason,
                    "OAuth login blocked: existing same-email account is not safe to auto-link"
                );
                // 409, not the generic 401: the caller completed the provider
                // handshake and thus owns this mailbox, so we can safely tell them
                // the account already exists and to use their original method,
                // rather than the misleading transient "try again" (dead-end
                // audit). Still no auto-link — that remains refused (TM-AUTH-012).
                return Err(AuthError::conflict(
                    "This email already has an Everruns account. Sign in with your original method.",
                ));
            }

            let linked = state
                .db
                .link_oauth_identity(existing.id, provider_str, &user_info.provider_id)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to link OAuth identity: {}", e);
                    AuthError::unauthorized("OAuth authentication failed")
                })?
                .ok_or_else(|| {
                    tracing::warn!(
                        provider = %provider_str,
                        "OAuth identity conflicts with an existing provider binding"
                    );
                    AuthError::conflict(
                        "This OAuth identity cannot be linked to the existing account.",
                    )
                })?;

            if let Err(e) = state.db.delete_user_refresh_tokens(linked.id).await {
                tracing::warn!(
                    error = %e,
                    user_id = %linked.id,
                    "failed to revoke refresh tokens after OAuth identity link"
                );
            }

            audit::emit(
                state.db.clone(),
                DEFAULT_ORG_ID,
                Some(linked.id),
                "auth.oauth.linked",
                audit::client_ip(peer_addr, headers),
                serde_json::json!({"provider": provider}),
            );
            tracing::info!(
                provider = %provider_str,
                "Linked OAuth identity to existing account by verified email"
            );
            linked
        } else {
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

            // Add newly created user to the default organization — single-tenant
            // convenience only, gated on `auto_join_default_org` (off by default).
            // In a multi-tenant deployment a first-time-OAuth user must own no org
            // so zero-org onboarding creates their own. See `register` and
            // `knowledge/security/authentication.md`.
            if state.config.auto_join_default_org {
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
                    state.built_in_harnesses.as_slice(),
                )
                .await
                {
                    tracing::warn!(error = %e, "Failed to ensure default org harnesses (non-fatal)");
                }
            }

            created_user
        }
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
        organizations,
    };

    audit::emit(
        state.db.clone(),
        DEFAULT_ORG_ID,
        Some(auth_user.id),
        "auth.oauth.success",
        audit::client_ip(peer_addr, headers),
        serde_json::json!({"provider": provider}),
    );

    // Generate tokens and set cookies
    let (jar, _) = generate_token_response(state, jar, &auth_user).await?;

    // Redirect to frontend (different origin in dev)
    let redirect_url = format!("{}/", state.config.frontend_url.trim_end_matches('/'));
    Ok((jar, Redirect::to(&redirect_url)))
}

// ============================================================================
// Password reset + email verification (native auth)
// ============================================================================
//
// Token model: each emailed link carries a 32-byte random token (hex). Only its
// SHA-256 hash is persisted (reusing `hash_invite_token`); the raw token never
// touches the database. Tokens are single-use and short-lived (claimed via an
// atomic `consume_*` UPDATE). Both "start" endpoints are enumeration-safe: they
// always return 200 with a generic body whether or not the email is registered,
// so an attacker cannot use them to discover accounts. Email delivery is
// best-effort — a disabled/unconfigured sender or a transport failure is logged
// but never surfaced, matching the org-invitation delivery contract.

/// Password reset links are valid for one hour: long enough to act on the
/// email, short enough to bound the value of a leaked link.
const PASSWORD_RESET_TTL: std::time::Duration = std::time::Duration::from_secs(60 * 60);
/// Verification links are valid for 24 hours so a new user has ample time.
const EMAIL_VERIFICATION_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
/// Token entropy: 32 random bytes, hex-encoded into the link.
const RECOVERY_TOKEN_BYTES: usize = 32;
/// Minimum response time for account-recovery start endpoints. This keeps
/// generic `200 {"ok":true}` responses from becoming an account-state timing
/// oracle while actual email delivery runs off the request path.
const RECOVERY_START_MIN_RESPONSE_TIME: std::time::Duration = std::time::Duration::from_millis(50);

/// Generate a fresh recovery token and its storage hash. The raw token is shown
/// once (embedded in the emailed URL) and never persisted.
fn generate_recovery_token() -> (String, String) {
    let mut rng = rand::rng();
    let bytes: [u8; RECOVERY_TOKEN_BYTES] = rng.random();
    let token = hex::encode(bytes);
    let hash = crate::api::org_invitations::hash_invite_token(&token);
    (token, hash)
}

/// True when this account authenticates with a local password (vs OAuth-only).
/// Verification only applies to password accounts because OAuth providers own
/// their own email proof. Password reset can add a password to OAuth-only
/// accounts after the user proves inbox control with the emailed token.
fn is_local_password_user(user: &crate::storage::models::UserRow) -> bool {
    user.password_hash.is_some()
        || user.auth_provider.as_deref() == Some("local")
        || user.auth_provider.is_none()
}

/// Best-effort send of the password-reset email. Never errors; delivery
/// problems are logged so a missing/disabled sender does not break the flow.
async fn send_password_reset_email(state: &BuiltinAuthBackend, to: &str, raw_token: &str) {
    let url = format!(
        "{}/reset-password?token={raw_token}",
        state.config.frontend_url.trim_end_matches('/')
    );
    let subject = "Reset your Everruns password";
    let text = format!(
        "We received a request to reset your Everruns password.Reset it here:{url}This link expires in 1 hour. If you didn't request a password reset, you can safely ignore this email."
    );
    let html = format!(
        "<p>We received a request to reset your Everruns password.</p>\
         <p><a href=\"{url}\">Reset your password</a></p>\
         <p>This link expires in 1 hour. If you didn't request a password reset, you can safely ignore this email.</p>"
    );
    deliver_account_email(state, to, subject, text, html).await;
}

/// Best-effort send of the email-verification email.
async fn send_verification_email(state: &BuiltinAuthBackend, to: &str, raw_token: &str) {
    // Carry the (URL-encoded) email so the verify-email page can offer a
    // one-click "resend" without an active session. The raw token is server-
    // generated hex; the email is user-controlled, so it must be encoded.
    let url = format!(
        "{}/verify-email?token={raw_token}&email={}",
        state.config.frontend_url.trim_end_matches('/'),
        urlencoding::encode(to),
    );
    let subject = "Verify your Everruns email";
    let text = format!(
        "Welcome to Everruns!Please confirm your email address:{url}If you didn't create an Everruns account, you can ignore this email."
    );
    let html = format!(
        "<p>Welcome to <strong>Everruns</strong>!</p>\
         <p>Please confirm your email address:</p>\
         <p><a href=\"{url}\">Verify your email</a></p>\
         <p>If you didn't create an Everruns account, you can ignore this email.</p>"
    );
    deliver_account_email(state, to, subject, text, html).await;
}

/// Confirm-mode signup with an already-registered address: the on-screen
/// response is identical to a fresh signup, and THIS email is the only place
/// the user learns they already have an account (anti-enumeration).
async fn send_account_exists_email(state: &BuiltinAuthBackend, to: &str) {
    let url = format!(
        "{}/login",
        state.config.login_origin().trim_end_matches('/')
    );
    let subject = "You already have an Everruns account";
    let text = format!(
        "Someone (probably you) tried to create an Everruns account with this email — but you already have one.Log in here:{url}Forgot your password? Use \"Reset your password\" on the login page. If this wasn't you, you can safely ignore this email."
    );
    let html = format!(
        "<p>Someone (probably you) tried to create an Everruns account with this email — but you already have one.</p>\
         <p><a href=\"{url}\">Log in to Everruns</a></p>\
         <p>Forgot your password? Use \"Reset your password\" on the login page. If this wasn't you, you can safely ignore this email.</p>"
    );
    deliver_account_email(state, to, subject, text, html).await;
}

/// Shared best-effort delivery. The raw token is the only secret in the URL and
/// is generated server-side (hex), so no user-controlled value is interpolated
/// into the HTML here — escaping is therefore unnecessary.
async fn deliver_account_email(
    state: &BuiltinAuthBackend,
    to: &str,
    subject: &str,
    text: String,
    html: String,
) {
    use everruns_platform::email::{EmailError, EmailMessage};
    let sender = state.email_sender.clone();
    let message = EmailMessage::basic(to, subject, text, html);
    match sender.send_email(message).await {
        Ok(_) => {}
        // Disabled/unconfigured sender: expected in OSS without an email
        // provider. Not an error condition.
        Err(EmailError::Configuration(_)) => {
            tracing::debug!("account email not sent: email delivery is not configured");
        }
        Err(err) => {
            tracing::warn!(error = %err, "account recovery email delivery failed");
        }
    }
}

fn recovery_start_response_delay(elapsed: std::time::Duration) -> std::time::Duration {
    RECOVERY_START_MIN_RESPONSE_TIME.saturating_sub(elapsed)
}

async fn finish_recovery_start_response(started_at: tokio::time::Instant) -> Json<OkResponse> {
    let delay = recovery_start_response_delay(started_at.elapsed());
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    OkResponse::ok()
}

async fn issue_password_reset_email(state: &BuiltinAuthBackend, user_id: Uuid, email: &str) {
    let (raw_token, token_hash) = generate_recovery_token();
    let expires_at =
        Utc::now() + Duration::from_std(PASSWORD_RESET_TTL).unwrap_or_else(|_| Duration::hours(1));
    match state
        .db
        .create_password_reset_token(user_id, &token_hash, expires_at)
        .await
    {
        Ok(()) => send_password_reset_email(state, email, &raw_token).await,
        Err(e) => tracing::error!(error = %e, "failed to create password reset token"),
    }
}

/// POST /v1/auth/forgot-password - Begin a password reset.
///
/// Enumeration-safe: always returns 200 `{ "ok": true }`. If an account exists
/// for the email, a single-use reset token (1h TTL) is created and emailed.
/// Completing the reset sets a local password, including for OAuth-created
/// accounts, so users never land in a "check your inbox" flow that sends no
/// email for an existing account.
pub async fn forgot_password(
    State(state): State<BuiltinAuthBackend>,
    Json(req): Json<EmailOnlyRequest>,
) -> Result<Json<OkResponse>, AuthError> {
    let started_at = tokio::time::Instant::now();
    // Bot gate (when configured) before any account work. A captcha failure
    // is account-independent, so surfacing it is not an enumeration signal.
    enforce_auth_captcha(&state, req.captcha_token.as_deref(), None).await?;

    // Email-bombing guard: per-address send budget (shared with
    // resend-verification). Over budget → the same generic success, silently
    // skipping token creation and send, so the throttle is not an oracle.
    if state
        .rate_limiter
        .check_account_email_send(&req.email.trim().to_lowercase())
        .await
        .is_err()
    {
        return Ok(finish_recovery_start_response(started_at).await);
    }
    if let Ok(Some(user)) = state.db.get_user_by_email(&req.email).await {
        let state = state.clone();
        tokio::spawn(async move {
            issue_password_reset_email(&state, user.id, &user.email).await;
        });
    }
    // Generic response regardless of outcome — never reveal account existence.
    Ok(finish_recovery_start_response(started_at).await)
}

/// POST /v1/auth/reset-password - Complete a password reset.
///
/// Consumes the token (atomic single-use), enforces the same password policy as
/// registration, updates the hash, and revokes all refresh tokens so any
/// previously stolen sessions are invalidated.
pub async fn reset_password(
    State(state): State<BuiltinAuthBackend>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<Json<OkResponse>, AuthError> {
    // Validate password before consuming the token so an invalid password does
    // not burn a single-use token.
    validate_new_password(&req.password)?;

    let token_hash = crate::api::org_invitations::hash_invite_token(&req.token);
    let user_id = state
        .db
        .consume_password_reset_token(&token_hash)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to consume password reset token");
            AuthError::internal("Password reset failed")
        })?
        // Invalid / expired / already-used: generic 400, no detail.
        .ok_or_else(|| AuthError::bad_request("Invalid or expired reset token"))?;

    let password_hash = hash_password(&req.password).map_err(|e| {
        tracing::error!(error = %e, "password hashing error during reset");
        AuthError::internal("Password reset failed")
    })?;

    state
        .db
        .update_user(
            user_id,
            crate::storage::models::UpdateUser {
                password_hash: Some(password_hash),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to update password during reset");
            AuthError::internal("Password reset failed")
        })?;

    // Revoke all refresh tokens: a reset implies the account may be compromised,
    // so existing sessions must not survive it.
    if let Err(e) = state.db.delete_user_refresh_tokens(user_id).await {
        tracing::warn!(error = %e, "failed to revoke refresh tokens after password reset");
    }

    Ok(OkResponse::ok())
}

/// POST /v1/auth/verify-email - Mark the user's email verified.
pub async fn verify_email(
    State(state): State<BuiltinAuthBackend>,
    Json(req): Json<VerifyEmailRequest>,
) -> Result<Json<OkResponse>, AuthError> {
    let token_hash = crate::api::org_invitations::hash_invite_token(&req.token);
    let user_id = state
        .db
        .consume_email_verification_token(&token_hash)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to consume email verification token");
            AuthError::internal("Email verification failed")
        })?
        .ok_or_else(|| AuthError::bad_request("Invalid or expired verification token"))?;

    state
        .db
        .update_user(
            user_id,
            crate::storage::models::UpdateUser {
                email_verified: Some(true),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to mark email verified");
            AuthError::internal("Email verification failed")
        })?;

    Ok(OkResponse::ok())
}

/// POST /v1/auth/resend-verification - Re-send a verification email.
///
/// Enumeration-safe like `forgot_password`. Only issues a token for an existing
/// local account whose email is not already verified.
pub async fn resend_verification(
    State(state): State<BuiltinAuthBackend>,
    Json(req): Json<EmailOnlyRequest>,
) -> Result<Json<OkResponse>, AuthError> {
    let started_at = tokio::time::Instant::now();
    // Bot gate (when configured), then the same per-address send budget as
    // forgot-password (email-bombing guard); over budget → generic success
    // without sending.
    enforce_auth_captcha(&state, req.captcha_token.as_deref(), None).await?;
    if state
        .rate_limiter
        .check_account_email_send(&req.email.trim().to_lowercase())
        .await
        .is_err()
    {
        return Ok(finish_recovery_start_response(started_at).await);
    }
    if let Ok(Some(user)) = state.db.get_user_by_email(&req.email).await
        && is_local_password_user(&user)
        && !user.email_verified
    {
        let state = state.clone();
        tokio::spawn(async move {
            issue_verification_email(&state, user.id, &user.email).await;
        });
    }
    Ok(finish_recovery_start_response(started_at).await)
}

/// Create + send a verification token for a user. Best-effort; logs on failure.
/// Shared by `register` (auto-send on signup) and `resend_verification`.
async fn issue_verification_email(state: &BuiltinAuthBackend, user_id: Uuid, email: &str) {
    let (raw_token, token_hash) = generate_recovery_token();
    let expires_at = Utc::now()
        + Duration::from_std(EMAIL_VERIFICATION_TTL).unwrap_or_else(|_| Duration::hours(24));
    match state
        .db
        .create_email_verification_token(user_id, &token_hash, expires_at)
        .await
    {
        Ok(()) => send_verification_email(state, email, &raw_token).await,
        Err(e) => tracing::error!(error = %e, "failed to create email verification token"),
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

    // Ensure org context for subsequent API calls, but never clobber a
    // selection that still maps to one of the user's organizations: this
    // helper runs on every login AND every silent refresh (~each access-token
    // lifetime), so unconditional re-minting silently reset the selected org
    // to the first (alphabetical) one.
    let keep_existing_org = jar
        .get(ORG_COOKIE_NAME)
        .is_some_and(|c| user.organizations.iter().any(|o| o.public_id == c.value()));
    if !keep_existing_org && let Some(org) = user.organizations.first() {
        let org_cookie = Cookie::build((ORG_COOKIE_NAME, org.public_id.clone()))
            .path("/")
            .http_only(false) // Allow JS to read for UI state
            .secure(true)
            .same_site(SameSite::Lax)
            .max_age(super::middleware::ORG_COOKIE_MAX_AGE)
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
        let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();
        if !roles.iter().any(|role| role == "admin") {
            tracing::error!(
                user_id = %user.id,
                email = %user.email,
                "Configured admin email collides with a non-admin account"
            );
            return Err(AuthError::unauthorized("Login failed"));
        }
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
            state.built_in_harnesses.as_slice(),
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
        organizations,
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

    // EVE-632 / TM-AUTH-014: password registration must not disclose that an
    // account exists. OAuth-link conflicts are different: that caller has
    // already proved mailbox ownership, so the callback may safely return its
    // dedicated 409 category.
    #[tokio::test]
    async fn registration_existing_account_message_does_not_leak_existence() {
        use axum::response::IntoResponse;

        let response = AuthError::unauthorized(GENERIC_REGISTRATION_FAILED).into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let body = String::from_utf8(bytes.to_vec()).unwrap().to_lowercase();

        for leak in [
            "already exists",
            "account with this email",
            "existing credentials",
        ] {
            assert!(
                !body.contains(leak),
                "OAuth signup error must not disclose account existence (leaked '{leak}'): {body}"
            );
        }
        assert_eq!(GENERIC_REGISTRATION_FAILED, "Registration failed");
    }

    fn oauth_link_candidate(verified: bool, provider: Option<&str>) -> UserRow {
        let now = Utc::now();
        UserRow {
            id: Uuid::now_v7(),
            email: "victim@example.com".to_string(),
            name: "Victim".to_string(),
            avatar_url: None,
            roles: serde_json::json!(["user"]),
            password_hash: Some("argon2-hash".to_string()),
            email_verified: verified,
            auth_provider: provider.map(str::to_string),
            auth_provider_id: None,
            created_at: now,
            updated_at: now,
            external_id: None,
        }
    }

    #[test]
    fn oauth_link_rejects_unverified_existing_email_account() {
        let user = oauth_link_candidate(false, Some("local"));

        assert_eq!(
            existing_oauth_link_rejection_reason(&user, "google"),
            Some("existing_email_unverified")
        );
    }

    #[test]
    fn oauth_link_allows_verified_local_account() {
        let user = oauth_link_candidate(true, Some("local"));

        assert_eq!(existing_oauth_link_rejection_reason(&user, "google"), None);
    }

    #[test]
    fn oauth_link_rejects_verified_account_from_another_provider() {
        let user = oauth_link_candidate(true, Some("github"));

        assert_eq!(
            existing_oauth_link_rejection_reason(&user, "google"),
            Some("email_bound_to_different_provider")
        );
    }

    #[test]
    fn oauth_link_allows_verified_account_from_same_provider() {
        let user = oauth_link_candidate(true, Some("github"));

        assert_eq!(existing_oauth_link_rejection_reason(&user, "github"), None);
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

    // EVE-702: GitHub identities with an unverified email are rejected, mirroring
    // Google, so a hardcoded email_verified=true can no longer pre-empt accounts.
    #[test]
    fn test_github_rejects_unverified_email() {
        let config = AuthConfig::default();
        let user = make_google_user("user@example.com", false);
        assert_eq!(
            oauth_identity_rejection_reason(
                super::super::oauth::OAuthProvider::GitHub,
                &config,
                &user
            ),
            Some("email_unverified")
        );
    }

    // EVE-702: a verified GitHub email passes the gate.
    #[test]
    fn test_github_accepts_verified_email() {
        let config = AuthConfig::default();
        let user = make_google_user("user@example.com", true);
        assert!(
            oauth_identity_rejection_reason(
                super::super::oauth::OAuthProvider::GitHub,
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

    // EVE-702 replaced EVE-451's "GitHub has no gates" assertion: GitHub now
    // gates on email_verified. See test_github_rejects_unverified_email and
    // test_github_accepts_verified_email above.

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

    // Minimal signup derives a display name from the email local-part when the
    // client omits `name`.
    #[test]
    fn test_display_name_from_email() {
        assert_eq!(display_name_from_email("eli@acme.com"), "Eli");
        assert_eq!(display_name_from_email("eli.wong@x.com"), "Eli Wong");
        assert_eq!(display_name_from_email("eli_wong@x.com"), "Eli Wong");
        assert_eq!(display_name_from_email("eli-wong@x.com"), "Eli Wong");
        // Degenerate local-parts fall back to the raw email rather than "".
        assert_eq!(display_name_from_email("@x.com"), "@x.com");
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

    // ========================================================================
    // Password reset + email verification
    // ========================================================================

    use crate::auth::backend::AuthBackend;
    use crate::auth::config::AuthConfig;
    use crate::storage::StorageBackend;
    use crate::storage::models::CreateUserRow;
    use async_trait::async_trait;
    use everruns_core::PlatformDefinition;
    use everruns_platform::email::{EmailMessage, EmailResult, EmailSender, SentEmail};
    use std::sync::Arc;
    use std::sync::Mutex;

    fn test_backend() -> BuiltinAuthBackend {
        BuiltinAuthBackend::new(
            AuthConfig::default(),
            Arc::new(StorageBackend::in_memory()),
            Arc::new(crate::platform::oss_platform_definition()),
        )
    }

    #[derive(Default)]
    struct RecordingEmailSender {
        messages: Mutex<Vec<EmailMessage>>,
    }

    #[async_trait]
    impl EmailSender for RecordingEmailSender {
        async fn send_email(&self, message: EmailMessage) -> EmailResult<SentEmail> {
            self.messages.lock().unwrap().push(message);
            Ok(SentEmail {
                provider: "recording",
                id: "recording".to_string(),
            })
        }
    }

    fn backend_with_email_sender(
        sender: Arc<dyn EmailSender>,
    ) -> (BuiltinAuthBackend, Arc<StorageBackend>) {
        let db = Arc::new(StorageBackend::in_memory());
        let platform = PlatformDefinition::builder().build();
        (
            BuiltinAuthBackend::new(AuthConfig::default(), db.clone(), Arc::new(platform))
                .with_email_sender(sender),
            db,
        )
    }

    #[derive(Debug)]
    struct SlowEmailSender(std::time::Duration);

    #[async_trait]
    impl EmailSender for SlowEmailSender {
        async fn send_email(&self, _message: EmailMessage) -> EmailResult<SentEmail> {
            tokio::time::sleep(self.0).await;
            Ok(SentEmail {
                provider: "slow-test",
                id: "slow-test".to_string(),
            })
        }

        fn name(&self) -> &'static str {
            "SlowEmailSender"
        }
    }

    async fn wait_for_recorded_messages(sender: &RecordingEmailSender, expected: usize) {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if sender.messages.lock().unwrap().len() >= expected {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("background email delivery timed out");
    }

    #[test]
    fn recovery_start_response_delay_enforces_minimum() {
        assert_eq!(
            recovery_start_response_delay(std::time::Duration::ZERO),
            RECOVERY_START_MIN_RESPONSE_TIME
        );
        assert_eq!(
            recovery_start_response_delay(RECOVERY_START_MIN_RESPONSE_TIME),
            std::time::Duration::ZERO
        );
        assert_eq!(
            recovery_start_response_delay(RECOVERY_START_MIN_RESPONSE_TIME * 2),
            std::time::Duration::ZERO
        );
    }

    // Full-mode backend so password registration is enabled.
    fn full_mode_backend() -> BuiltinAuthBackend {
        let config = AuthConfig {
            mode: AuthMode::Full,
            ..Default::default()
        };
        BuiltinAuthBackend::new(
            config,
            Arc::new(StorageBackend::in_memory()),
            Arc::new(crate::platform::oss_platform_definition()),
        )
    }

    // Minimal signup: registering without a name derives the display name from
    // the email local-part.
    #[tokio::test]
    async fn register_without_name_derives_display_name_from_email() {
        let state = full_mode_backend();
        let db = state.db.clone();
        let outcome = register(
            State(state.clone()),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(RegisterRequest {
                email: "eli.wong@example.com".to_string(),
                password: "password12345".to_string(),
                name: None,
                captcha_token: None,
            }),
        )
        .await
        .expect("register should succeed");
        let RegisterOutcome::Session(status, _jar, _json) = outcome else {
            panic!("default mode must return an instant session");
        };
        assert_eq!(status, StatusCode::CREATED);

        let user = db
            .get_user_by_email("eli.wong@example.com")
            .await
            .unwrap()
            .expect("user created");
        assert_eq!(user.name, "Eli Wong");
    }

    // An explicit name is preserved (not overridden by the email derivation).
    #[tokio::test]
    async fn register_with_name_keeps_supplied_name() {
        let state = full_mode_backend();
        let db = state.db.clone();
        let _ = register(
            State(state.clone()),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(RegisterRequest {
                email: "someone@example.com".to_string(),
                password: "password12345".to_string(),
                name: Some("Ada Lovelace".to_string()),
                captcha_token: None,
            }),
        )
        .await
        .expect("register should succeed");

        let user = db
            .get_user_by_email("someone@example.com")
            .await
            .unwrap()
            .expect("user created");
        assert_eq!(user.name, "Ada Lovelace");
    }

    // Multi-tenant safety: with auto_join_default_org off (the default), a fresh
    // signup owns NO org, so zero-org onboarding creates the user's own org
    // instead of dumping every tenant into the shared default organization.
    #[tokio::test]
    async fn register_does_not_join_default_org_by_default() {
        let state = full_mode_backend();
        let db = state.db.clone();
        let outcome = register(
            State(state.clone()),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(RegisterRequest {
                email: "solo@example.com".to_string(),
                password: "password12345".to_string(),
                name: None,
                captcha_token: None,
            }),
        )
        .await
        .expect("register should succeed");

        let RegisterOutcome::Session(status, jar, Json(tokens)) = outcome else {
            panic!("default mode must return an instant session");
        };
        assert_eq!(status, StatusCode::CREATED);
        assert!(
            jar.get(ORG_COOKIE_NAME).is_none(),
            "zero-org signup must not receive a synthetic org cookie"
        );

        let auth_user = state
            .validate_token(&tokens.access_token)
            .await
            .expect("issued access token should validate");
        assert!(
            auth_user.organizations.is_empty(),
            "issued token must preserve zero-org memberships, got {:?}",
            auth_user.organizations
        );

        let user = db
            .get_user_by_email("solo@example.com")
            .await
            .unwrap()
            .expect("user created");
        let orgs = db.list_user_organizations(user.id).await.unwrap();
        assert!(
            orgs.is_empty(),
            "fresh signup must have zero org memberships by default, got {orgs:?}"
        );
    }

    // Single-tenant opt-in: AUTH_AUTO_JOIN_DEFAULT_ORG=true restores the shared
    // default-org membership for a single-binary / small self-host.
    #[tokio::test]
    async fn register_joins_default_org_when_opted_in() {
        use crate::storage::models::CreateOrganizationRow;
        let config = AuthConfig {
            mode: AuthMode::Full,
            auto_join_default_org: true,
            ..Default::default()
        };
        let db = Arc::new(StorageBackend::in_memory());
        // Membership can only attach if the default org exists.
        db.create_organization_with_id(
            DEFAULT_ORG_ID,
            CreateOrganizationRow {
                public_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
                name: "Default Organization".to_string(),
                created_by: None,
            },
        )
        .await
        .expect("seed default org");
        let state = BuiltinAuthBackend::new(
            config,
            db.clone(),
            Arc::new(crate::platform::oss_platform_definition()),
        );
        register(
            State(state.clone()),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(RegisterRequest {
                email: "joiner@example.com".to_string(),
                password: "password12345".to_string(),
                name: None,
                captcha_token: None,
            }),
        )
        .await
        .expect("register should succeed");

        let user = db
            .get_user_by_email("joiner@example.com")
            .await
            .unwrap()
            .expect("user created");
        assert!(
            db.is_organization_member(DEFAULT_ORG_ID, user.id)
                .await
                .unwrap(),
            "opted-in signup should join the default org"
        );
    }

    async fn seed_local_user(db: &StorageBackend, email: &str, password: &str) -> Uuid {
        let user = db
            .create_user(CreateUserRow {
                email: email.to_string(),
                name: "Test User".to_string(),
                avatar_url: None,
                roles: vec!["user".to_string()],
                password_hash: Some(hash_password(password).unwrap()),
                email_verified: false,
                auth_provider: Some("local".to_string()),
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .expect("create user");
        user.id
    }

    async fn seed_oauth_user(db: &StorageBackend, email: &str, verified: bool) -> Uuid {
        let user = db
            .create_user(CreateUserRow {
                email: email.to_string(),
                name: "OAuth User".to_string(),
                avatar_url: None,
                roles: vec!["user".to_string()],
                password_hash: None,
                email_verified: verified,
                auth_provider: Some("github".to_string()),
                auth_provider_id: Some(format!("github:{email}")),
                external_id: None,
            })
            .await
            .expect("create oauth user");
        user.id
    }

    #[tokio::test]
    async fn admin_mode_register_is_disabled() {
        let config = AuthConfig {
            mode: AuthMode::Admin,
            admin: Some(super::super::config::AdminConfig {
                email: "admin@example.com".to_string(),
                password: "password12345".to_string(),
            }),
            ..Default::default()
        };
        let state = BuiltinAuthBackend::new(
            config,
            Arc::new(StorageBackend::in_memory()),
            Arc::new(crate::platform::oss_platform_definition()),
        );

        let err = register(
            State(state),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(RegisterRequest {
                email: "attacker@example.com".to_string(),
                password: "password12345".to_string(),
                name: None,
                captcha_token: None,
            }),
        )
        .await
        .expect_err("admin mode must not allow self-registration");
        assert_eq!(err.status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_login_rejects_non_admin_email_collision() {
        let config = AuthConfig {
            mode: AuthMode::Admin,
            admin: Some(super::super::config::AdminConfig {
                email: "admin@example.com".to_string(),
                password: "password12345".to_string(),
            }),
            ..Default::default()
        };
        let db = Arc::new(StorageBackend::in_memory());
        let attacker_id = seed_local_user(&db, " ADMIN@example.com ", "attacker12345").await;
        let state = BuiltinAuthBackend::new(
            config,
            db.clone(),
            Arc::new(crate::platform::oss_platform_definition()),
        );

        let err = login(
            State(state),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(LoginRequest {
                email: "admin@example.com".to_string(),
                password: "password12345".to_string(),
            }),
        )
        .await
        .expect_err("admin bootstrap must fail closed on non-admin collision");
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);
        assert!(
            db.list_user_organizations(attacker_id)
                .await
                .unwrap()
                .is_empty(),
            "colliding user must not be promoted into any org"
        );
    }

    #[tokio::test]
    async fn password_reset_token_create_consume_is_single_use() {
        let db = StorageBackend::in_memory();
        let user_id = seed_local_user(&db, "reset@example.com", "password12345").await;
        let (raw, hash) = generate_recovery_token();
        db.create_password_reset_token(user_id, &hash, Utc::now() + Duration::hours(1))
            .await
            .unwrap();

        // Happy path: first consume returns the owner.
        let hash_again = crate::api::org_invitations::hash_invite_token(&raw);
        assert_eq!(
            db.consume_password_reset_token(&hash_again).await.unwrap(),
            Some(user_id)
        );
        // Single-use: second consume returns None.
        assert_eq!(
            db.consume_password_reset_token(&hash_again).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn password_reset_token_expired_and_unknown_return_none() {
        let db = StorageBackend::in_memory();
        let user_id = seed_local_user(&db, "exp@example.com", "password12345").await;
        let (raw, hash) = generate_recovery_token();
        // Already expired.
        db.create_password_reset_token(user_id, &hash, Utc::now() - Duration::minutes(1))
            .await
            .unwrap();
        let hash_again = crate::api::org_invitations::hash_invite_token(&raw);
        assert_eq!(
            db.consume_password_reset_token(&hash_again).await.unwrap(),
            None
        );
        // Unknown token.
        assert_eq!(
            db.consume_password_reset_token("deadbeef").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn email_verification_token_create_consume_is_single_use() {
        let db = StorageBackend::in_memory();
        let user_id = seed_local_user(&db, "verify@example.com", "password12345").await;
        let (raw, hash) = generate_recovery_token();
        db.create_email_verification_token(user_id, &hash, Utc::now() + Duration::hours(1))
            .await
            .unwrap();
        let hash_again = crate::api::org_invitations::hash_invite_token(&raw);
        assert_eq!(
            db.consume_email_verification_token(&hash_again)
                .await
                .unwrap(),
            Some(user_id)
        );
        assert_eq!(
            db.consume_email_verification_token(&hash_again)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn reset_password_updates_hash_and_revokes_refresh_tokens() {
        let state = test_backend();
        let db = state.db.clone();
        let user_id = seed_local_user(&db, "rp@example.com", "oldpassword").await;

        // A live refresh token that the reset must revoke.
        db.create_refresh_token(CreateRefreshTokenRow {
            user_id,
            token_hash: "some-refresh-hash".to_string(),
            expires_at: Utc::now() + Duration::days(30),
        })
        .await
        .unwrap();

        let (raw, hash) = generate_recovery_token();
        db.create_password_reset_token(user_id, &hash, Utc::now() + Duration::hours(1))
            .await
            .unwrap();

        let _ = reset_password(
            State(state.clone()),
            Json(ResetPasswordRequest {
                token: raw,
                password: "newpassword12".to_string(),
            }),
        )
        .await
        .expect("reset should succeed");

        let user = db.get_user(user_id).await.unwrap().unwrap();
        let stored = user.password_hash.unwrap();
        // Old password no longer verifies; new one does.
        assert!(!verify_password("oldpassword", &stored).unwrap());
        assert!(verify_password("newpassword12", &stored).unwrap());
        // Refresh tokens were revoked.
        assert_eq!(
            db.consume_refresh_token_by_hash("some-refresh-hash")
                .await
                .unwrap()
                .map(|t| t.user_id),
            None
        );
    }

    #[tokio::test]
    async fn reset_password_rejects_invalid_token() {
        let state = test_backend();
        let err = reset_password(
            State(state),
            Json(ResetPasswordRequest {
                token: "nope".to_string(),
                password: "newpassword12".to_string(),
            }),
        )
        .await
        .expect_err("invalid token must be rejected");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    // Time-expired tokens (not just malformed ones) must be rejected with the
    // same generic 400 as invalid/used tokens.
    #[tokio::test]
    async fn reset_password_rejects_expired_token() {
        let state = test_backend();
        let user_id = seed_local_user(&state.db, "expired@example.com", "password12345").await;
        let (raw, hash) = generate_recovery_token();
        state
            .db
            .create_password_reset_token(user_id, &hash, Utc::now() - Duration::minutes(1))
            .await
            .unwrap();
        let err = reset_password(
            State(state),
            Json(ResetPasswordRequest {
                token: raw,
                password: "newpassword123".to_string(),
            }),
        )
        .await
        .expect_err("expired token must be rejected");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn verify_email_rejects_expired_token() {
        let state = test_backend();
        let user_id = seed_local_user(&state.db, "expired2@example.com", "password12345").await;
        let (raw, hash) = generate_recovery_token();
        state
            .db
            .create_email_verification_token(user_id, &hash, Utc::now() - Duration::minutes(1))
            .await
            .unwrap();
        let err = verify_email(State(state), Json(VerifyEmailRequest { token: raw }))
            .await
            .expect_err("expired token must be rejected");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn reset_password_rejects_short_password() {
        let state = test_backend();
        let err = reset_password(
            State(state),
            Json(ResetPasswordRequest {
                token: "whatever".to_string(),
                password: "short".to_string(),
            }),
        )
        .await
        .expect_err("short password must be rejected");
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn verify_email_sets_email_verified() {
        let state = test_backend();
        let db = state.db.clone();
        let user_id = seed_local_user(&db, "ve@example.com", "password12345").await;
        assert!(!db.get_user(user_id).await.unwrap().unwrap().email_verified);

        let (raw, hash) = generate_recovery_token();
        db.create_email_verification_token(user_id, &hash, Utc::now() + Duration::hours(1))
            .await
            .unwrap();

        let _ = verify_email(
            State(state.clone()),
            Json(VerifyEmailRequest { token: raw }),
        )
        .await
        .expect("verify should succeed");

        assert!(db.get_user(user_id).await.unwrap().unwrap().email_verified);
    }

    #[tokio::test]
    async fn verify_email_rejects_invalid_token() {
        let state = test_backend();
        let err = verify_email(
            State(state),
            Json(VerifyEmailRequest {
                token: "bad".to_string(),
            }),
        )
        .await
        .expect_err("invalid token must be rejected");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn forgot_password_is_enumeration_safe_for_unknown_email() {
        let state = test_backend();
        // No user exists; must still return 200 ok without error.
        let resp = forgot_password(
            State(state),
            Json(EmailOnlyRequest {
                email: "ghost@example.com".to_string(),
                captcha_token: None,
            }),
        )
        .await
        .expect("enumeration-safe success");
        assert!(resp.0.ok);
    }

    #[tokio::test]
    async fn forgot_password_sends_reset_email_for_oauth_only_account() {
        let sender = Arc::new(RecordingEmailSender::default());
        let (state, db) = backend_with_email_sender(sender.clone());
        seed_oauth_user(&db, "oauth-only@example.com", true).await;

        let resp = forgot_password(
            State(state),
            Json(EmailOnlyRequest {
                email: "oauth-only@example.com".to_string(),
                captcha_token: None,
            }),
        )
        .await
        .expect("enumeration-safe success");
        assert!(resp.0.ok);

        wait_for_recorded_messages(&sender, 1).await;
        let messages = sender.messages.lock().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].subject, "Reset your Everruns password");
        assert_eq!(messages[0].to[0].email, "oauth-only@example.com");
    }

    #[tokio::test]
    async fn resend_verification_is_enumeration_safe_for_unknown_email() {
        let state = test_backend();
        let resp = resend_verification(
            State(state),
            Json(EmailOnlyRequest {
                email: "ghost@example.com".to_string(),
                captcha_token: None,
            }),
        )
        .await
        .expect("enumeration-safe success");
        assert!(resp.0.ok);
    }

    #[tokio::test]
    async fn forgot_password_does_not_wait_for_slow_email_delivery() {
        let (state, _) = backend_with_email_sender(Arc::new(SlowEmailSender(
            std::time::Duration::from_millis(250),
        )));
        seed_local_user(&state.db, "recover@example.com", "password12345").await;
        seed_oauth_user(&state.db, "oauth@example.com", false).await;

        for email in [
            "recover@example.com",
            "oauth@example.com",
            "ghost@example.com",
        ] {
            let started = tokio::time::Instant::now();
            let resp = forgot_password(
                State(state.clone()),
                Json(EmailOnlyRequest {
                    email: email.to_string(),
                    captcha_token: None,
                }),
            )
            .await
            .expect("always generic success");
            let elapsed = started.elapsed();
            assert!(resp.0.ok);
            assert!(
                elapsed >= RECOVERY_START_MIN_RESPONSE_TIME,
                "forgot-password response for {email} bypassed timing normalization"
            );
            assert!(
                elapsed < std::time::Duration::from_millis(200),
                "forgot-password response for {email} waited for slow email delivery"
            );
        }
    }

    #[tokio::test]
    async fn resend_verification_does_not_wait_for_slow_email_delivery() {
        let (state, _) = backend_with_email_sender(Arc::new(SlowEmailSender(
            std::time::Duration::from_millis(250),
        )));
        seed_local_user(&state.db, "unverified@example.com", "password12345").await;
        let verified_id = seed_local_user(&state.db, "verified@example.com", "password12345").await;
        state
            .db
            .update_user(
                verified_id,
                crate::storage::models::UpdateUser {
                    email_verified: Some(true),
                    ..Default::default()
                },
            )
            .await
            .expect("mark verified");
        seed_oauth_user(&state.db, "oauth@example.com", false).await;

        for email in [
            "unverified@example.com",
            "verified@example.com",
            "oauth@example.com",
            "ghost@example.com",
        ] {
            let started = tokio::time::Instant::now();
            let resp = resend_verification(
                State(state.clone()),
                Json(EmailOnlyRequest {
                    email: email.to_string(),
                    captcha_token: None,
                }),
            )
            .await
            .expect("always generic success");
            let elapsed = started.elapsed();
            assert!(resp.0.ok);
            assert!(
                elapsed >= RECOVERY_START_MIN_RESPONSE_TIME,
                "resend-verification response for {email} bypassed timing normalization"
            );
            assert!(
                elapsed < std::time::Duration::from_millis(200),
                "resend-verification response for {email} waited for slow email delivery"
            );
        }
    }

    // --- Auth hardening tests (abuse limits, password cap, logout revoke,
    // captcha gate, OAuth failure redirect) ---

    #[tokio::test]
    async fn register_rejects_oversized_password() {
        let state = full_mode_backend();
        let err = register(
            State(state),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(RegisterRequest {
                email: "big@example.com".to_string(),
                password: "x".repeat(PASSWORD_MAX_LENGTH + 1),
                name: None,
                captcha_token: None,
            }),
        )
        .await
        .expect_err("oversized password must be rejected");
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn login_rejects_oversized_password_with_generic_error() {
        let state = full_mode_backend();
        seed_local_user(&state.db, "cap@example.com", "password12345").await;
        let err = login(
            State(state),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(LoginRequest {
                email: "cap@example.com".to_string(),
                password: "x".repeat(PASSWORD_MAX_BYTES + 1),
            }),
        )
        .await
        .expect_err("oversized password must fail");
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);
        assert_eq!(err.error, "Invalid email or password");
    }

    #[tokio::test]
    async fn reset_password_rejects_oversized_password() {
        let state = test_backend();
        let err = reset_password(
            State(state),
            Json(ResetPasswordRequest {
                token: "whatever".to_string(),
                password: "x".repeat(PASSWORD_MAX_LENGTH + 1),
            }),
        )
        .await
        .expect_err("oversized password must be rejected before token consume");
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    // Per-account throttle: after the cross-IP budget is exhausted for one
    // email, login returns 429 regardless of credentials; other accounts are
    // unaffected.
    #[tokio::test]
    async fn login_per_account_throttle_returns_429() {
        let state = full_mode_backend();
        seed_local_user(&state.db, "stuffed@example.com", "password12345").await;
        let mut last_status = None;
        for _ in 0..25 {
            let result = login(
                State(state.clone()),
                None,
                HeaderMap::new(),
                CookieJar::new(),
                Json(LoginRequest {
                    email: "stuffed@example.com".to_string(),
                    password: "wrong-password".to_string(),
                }),
            )
            .await;
            last_status = result.err().map(|e| e.status);
        }
        assert_eq!(
            last_status,
            Some(StatusCode::TOO_MANY_REQUESTS),
            "per-account budget must trip after repeated failures"
        );

        // A different account still gets the normal generic 401.
        seed_local_user(&state.db, "fresh@example.com", "password12345").await;
        let err = login(
            State(state),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(LoginRequest {
                email: "fresh@example.com".to_string(),
                password: "wrong-password".to_string(),
            }),
        )
        .await
        .expect_err("wrong password fails");
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);
    }

    // Logout must revoke the refresh token server-side, not just clear the
    // cookie (TM-AUTH-003).
    #[tokio::test]
    async fn logout_revokes_refresh_token_server_side() {
        let state = full_mode_backend();
        let db = state.db.clone();
        let RegisterOutcome::Session(_status, jar, _json) = register(
            State(state.clone()),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(RegisterRequest {
                email: "bye@example.com".to_string(),
                password: "password12345".to_string(),
                name: None,
                captcha_token: None,
            }),
        )
        .await
        .expect("register succeeds") else {
            panic!("default mode must return an instant session");
        };

        let refresh_cookie = jar
            .get("refresh_token")
            .expect("refresh cookie set")
            .value()
            .to_string();
        let token_hash = hash_token(&refresh_cookie);

        let _cleared = logout(State(state), jar).await;

        let consumed = db
            .consume_refresh_token_by_hash(&token_hash)
            .await
            .expect("storage reachable");
        assert!(
            consumed.is_none(),
            "refresh token must already be revoked by logout"
        );
    }

    // Email budget: the second forgot-password for the same address within a
    // minute is silently skipped but still returns the generic success.
    #[tokio::test]
    async fn forgot_password_email_budget_stays_enumeration_safe() {
        let state = test_backend();
        for _ in 0..2 {
            let resp = forgot_password(
                State(state.clone()),
                Json(EmailOnlyRequest {
                    email: "budget@example.com".to_string(),
                    captcha_token: None,
                }),
            )
            .await
            .expect("always generic success");
            assert!(resp.0.ok);
        }
    }

    // Captcha gate: when Turnstile is configured, a missing token is a
    // generic 403 before any account work (empty token short-circuits in the
    // verifier without a network call).
    #[tokio::test]
    async fn register_requires_captcha_when_configured() {
        let config = AuthConfig {
            mode: AuthMode::Full,
            turnstile: Some(super::super::config::TurnstileAuthConfig {
                site_key: "site".to_string(),
                secret_key: "secret".to_string(),
            }),
            ..Default::default()
        };
        let state = BuiltinAuthBackend::new(
            config,
            Arc::new(StorageBackend::in_memory()),
            Arc::new(crate::platform::oss_platform_definition()),
        );
        let err = register(
            State(state),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(RegisterRequest {
                email: "bot@example.com".to_string(),
                password: "password12345".to_string(),
                name: None,
                captcha_token: None,
            }),
        )
        .await
        .expect_err("missing captcha token must be rejected");
        assert_eq!(err.status, StatusCode::FORBIDDEN);
    }

    // OAuth callback failures land on the login door with a coarse category,
    // never a raw JSON error (browser-only endpoint).
    #[tokio::test]
    async fn oauth_callback_provider_error_redirects_to_login() {
        use axum::response::IntoResponse;
        let state = full_mode_backend();
        let frontend = state.config.frontend_url.trim_end_matches('/').to_string();
        let (_jar, redirect) = oauth_callback(
            State(state),
            None,
            HeaderMap::new(),
            Path("google".to_string()),
            Query(OAuthCallbackQuery {
                code: None,
                state: None,
                error: Some("access_denied".to_string()),
            }),
            CookieJar::new(),
        )
        .await;
        let response = redirect.into_response();
        let location = response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert_eq!(location, format!("{frontend}/login?error=oauth_cancelled"));
    }

    #[tokio::test]
    async fn oauth_callback_missing_params_redirects_to_login() {
        use axum::response::IntoResponse;
        let state = full_mode_backend();
        let frontend = state.config.frontend_url.trim_end_matches('/').to_string();
        let (_jar, redirect) = oauth_callback(
            State(state),
            None,
            HeaderMap::new(),
            Path("google".to_string()),
            Query(OAuthCallbackQuery {
                code: None,
                state: None,
                error: None,
            }),
            CookieJar::new(),
        )
        .await;
        let response = redirect.into_response();
        let location = response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert_eq!(location, format!("{frontend}/login?error=oauth_failed"));
    }

    #[tokio::test]
    async fn oauth_callback_failure_honors_configured_login_origin() {
        use axum::response::IntoResponse;
        let mut state = full_mode_backend();
        state.config.login_origin = Some("https://id.example.com".to_string());
        let (_jar, redirect) = oauth_callback(
            State(state),
            None,
            HeaderMap::new(),
            Path("google".to_string()),
            Query(OAuthCallbackQuery {
                code: None,
                state: None,
                error: Some("access_denied".to_string()),
            }),
            CookieJar::new(),
        )
        .await;
        let response = redirect.into_response();
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some("https://id.example.com/login?error=oauth_cancelled")
        );
    }

    // --- signup email-confirm mode (AUTH_SIGNUP_EMAIL_CONFIRM) ---

    fn confirm_mode_backend() -> BuiltinAuthBackend {
        let config = AuthConfig {
            mode: AuthMode::Full,
            signup_email_confirm: true,
            ..Default::default()
        };
        BuiltinAuthBackend::new(
            config,
            Arc::new(StorageBackend::in_memory()),
            Arc::new(crate::platform::oss_platform_definition()),
        )
    }

    #[tokio::test]
    async fn confirm_mode_register_creates_account_without_session() {
        let state = confirm_mode_backend();
        let db = state.db.clone();
        let outcome = register(
            State(state),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(RegisterRequest {
                email: "pending@example.com".to_string(),
                password: "password12345".to_string(),
                name: None,
                captcha_token: None,
            }),
        )
        .await
        .expect("register ok");
        assert!(
            matches!(outcome, RegisterOutcome::ConfirmationSent(_)),
            "confirm mode must not return a session"
        );
        let user = db
            .get_user_by_email("pending@example.com")
            .await
            .unwrap()
            .expect("account created");
        assert!(!user.email_verified);
    }

    // Existing address: identical generic outcome, no duplicate account, no
    // on-screen enumeration signal.
    #[tokio::test]
    async fn confirm_mode_register_existing_email_is_indistinguishable() {
        let state = confirm_mode_backend();
        seed_local_user(&state.db, "taken@example.com", "password12345").await;
        let outcome = register(
            State(state.clone()),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(RegisterRequest {
                email: "taken@example.com".to_string(),
                password: "password12345".to_string(),
                name: None,
                captcha_token: None,
            }),
        )
        .await
        .expect("must be generic success");
        assert!(matches!(outcome, RegisterOutcome::ConfirmationSent(_)));
    }

    #[tokio::test]
    async fn register_rejects_password_without_digit() {
        let state = full_mode_backend();
        let err = register(
            State(state),
            None,
            HeaderMap::new(),
            CookieJar::new(),
            Json(RegisterRequest {
                email: "nodigit@example.com".to_string(),
                password: "longenoughpassword".to_string(),
                name: None,
                captcha_token: None,
            }),
        )
        .await
        .expect_err("digit-less password must be rejected");
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}

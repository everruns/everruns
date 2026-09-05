// MCP OAuth 2.1 endpoints (backend-agnostic)
//
// Spec: knowledge/integrations/mcp.md
//
// Decision: Same pattern as CLI auth — authorize endpoint uses AuthUser extractor,
// works with any auth backend (builtin, PropelAuth, Auth0, etc.).
// Decision: PKCE mandatory (S256 only). No implicit grant.
// Decision: Dynamic client registration per RFC 7591.
// Decision: MCP access tokens are JWTs with token_type="mcp_access".

use super::{
    audit,
    config::AuthMode,
    jwt::JwtService,
    middleware::{AuthError, AuthState, AuthUser},
    rate_limit::{AuthRateLimiter, extract_client_ip},
};
use crate::security::constant_time_eq;
use crate::storage::StorageBackend;
use crate::storage::models::{
    CreateOAuthAuthorizationCodeRow, CreateOAuthClientRow, CreateOAuthRefreshTokenRow,
};
use axum::{
    Form, Json, Router,
    body::Body,
    extract::{ConnectInfo, Extension, FromRef, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use axum_extra::extract::CookieJar;
use chrono::{Duration, Utc};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::sync::Arc;
use utoipa::ToSchema;

/// Authorization code TTL (5 minutes)
const AUTH_CODE_TTL_SECS: i64 = 300;

/// MCP refresh token lifetime (30 days)
const MCP_REFRESH_TOKEN_LIFETIME_SECS: i64 = 30 * 24 * 3600;

/// Generate a random hex string (32 bytes = 64 hex chars)
fn generate_random_hex() -> String {
    let mut rng = rand::rng();
    let bytes: [u8; 32] = rng.random();
    hex::encode(bytes)
}

/// SHA-256 hash for storage
fn hash_value(value: &str) -> String {
    let hash = Sha256::digest(value.as_bytes());
    hex::encode(hash)
}

/// Verify PKCE S256 challenge: BASE64URL(SHA256(verifier)) == challenge
fn verify_pkce_s256(verifier: &str, challenge: &str) -> bool {
    use base64::Engine;
    let hash = Sha256::digest(verifier.as_bytes());
    let computed = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);
    constant_time_eq(computed.as_bytes(), challenge.as_bytes())
}

/// Validate a registered redirect URI for an MCP OAuth client.
///
/// Policy (per spec/threat-model OAuth open-redirect prevention):
/// - Allow `https://` to any host (with absolute URL form, no fragment).
/// - Allow `http://` only for native loopback callbacks: any IPv4 address in
///   `127.0.0.0/8`, the IPv6 `[::1]` address, and the literal `localhost`
///   host. Any port is fine.
/// - Reject every other scheme — explicitly including `javascript:`, `data:`,
///   `file:`, `vbscript:`, custom app schemes, and unparseable/relative URIs.
/// - Reject URIs with a fragment component (RFC 6749 §3.1.2).
fn validate_redirect_uri(raw: &str) -> Result<(), &'static str> {
    let parsed = url::Url::parse(raw).map_err(|_| "redirect_uri must be an absolute URL")?;
    if parsed.fragment().is_some() {
        return Err("redirect_uri must not contain a fragment");
    }
    match parsed.scheme() {
        "https" => {
            if parsed.host().is_none() {
                return Err("https redirect_uri must include a host");
            }
            Ok(())
        }
        "http" => match parsed.host() {
            Some(url::Host::Domain("localhost")) => Ok(()),
            Some(url::Host::Ipv4(ip)) if ip.is_loopback() => Ok(()),
            Some(url::Host::Ipv6(ip)) if ip.is_loopback() => Ok(()),
            _ => Err("http redirect_uri is only allowed for loopback hosts"),
        },
        _ => Err("redirect_uri scheme is not allowed"),
    }
}

/// Whether a redirect URI is an `http://` loopback callback — the shape only a
/// native client can serve.
fn is_loopback_http_uri(raw: &str) -> bool {
    let Ok(parsed) = url::Url::parse(raw) else {
        return false;
    };
    if parsed.scheme() != "http" {
        return false;
    }
    match parsed.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    }
}

// ============================================
// Request/Response types
// ============================================

/// OIDC Dynamic Client Registration `application_type`.
///
/// Everruns is not an OIDC provider, and the MCP spec notes that non-OIDC
/// servers may ignore the parameter — but accepting it lets a client state its
/// intent, and a client that says `web` should not then register a loopback
/// callback. It is deliberately *not* defaulted to `web` the way OIDC does:
/// every MCP client in the field today omits it and registers a loopback URI,
/// so defaulting would reject all of them. Absent means "unstated", which keeps
/// the pre-existing permissive behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum OAuthApplicationType {
    /// Desktop, mobile, CLI, or locally-hosted client — loopback callbacks are
    /// the norm.
    Native,
    /// Remote browser-based client served from a non-local host.
    Web,
}

/// POST /oauth/register request
#[derive(Debug, Deserialize, ToSchema)]
pub struct OAuthRegisterRequest {
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    /// Optional OIDC application type. See [`OAuthApplicationType`].
    #[serde(default)]
    pub application_type: Option<OAuthApplicationType>,
}

/// POST /oauth/register response
#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthRegisterResponse {
    pub client_id: String,
    pub client_secret: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
}

/// GET /oauth/authorize query parameters
#[derive(Debug, Deserialize)]
pub struct OAuthAuthorizeQuery {
    pub client_id: String,
    pub redirect_uri: String,
    pub response_type: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    /// RFC 9728 resource indicator — ignored but accepted so clients like Cursor
    /// don't get a deserialization error.
    #[serde(default)]
    pub resource: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OAuthAuthorizeConfirmForm {
    pub client_id: String,
    pub redirect_uri: String,
    pub response_type: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    pub csrf_token: String,
}

fn default_scope() -> String {
    "mcp".to_string()
}

/// POST /oauth/token request (form-encoded per OAuth spec)
#[derive(Debug, Deserialize)]
pub struct OAuthTokenRequest {
    pub grant_type: String,
    pub code: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
}

/// POST /oauth/token response
#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

/// OAuth error response (RFC 6749 §5.2)
#[derive(Debug, Serialize)]
pub struct OAuthErrorResponse {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

impl IntoResponse for OAuthErrorResponse {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(self)).into_response()
    }
}

/// Protected resource metadata (RFC 9728 / MCP spec)
#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthProtectedResourceMetadata {
    pub resource: String,
    pub authorization_servers: Vec<String>,
    pub bearer_methods_supported: Vec<String>,
}

/// Authorization server metadata (RFC 8414)
#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: String,
    pub response_types_supported: Vec<String>,
    pub grant_types_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub scopes_supported: Vec<String>,
    /// RFC 9207 §2.3 — declares that authorization responses carry `iss`.
    /// Setting this obliges the server to send `iss` on every authorization
    /// response, success and error alike, and lets clients reject a response
    /// that omits it.
    pub authorization_response_iss_parameter_supported: bool,
}

// ============================================
// State
// ============================================

/// State for MCP OAuth routes — decoupled from any specific AuthBackend.
#[derive(Clone)]
pub struct McpOAuthState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
    pub jwt_service: Arc<JwtService>,
    /// Public issuer URL without the API prefix (e.g. `https://app.example.com`).
    pub issuer_url: String,
    /// Frontend URL for login redirects (e.g. `http://localhost:9300`).
    pub frontend_url: String,
    /// Optional trusted origin hosting the login page.
    pub login_origin: Option<String>,
    /// Per-IP rate limiter shared with the other auth endpoints. Used to throttle
    /// the unauthenticated dynamic client registration endpoint (TM-DOS).
    pub rate_limiter: AuthRateLimiter,
}

impl McpOAuthState {
    /// Canonical MCP resource URL (`{issuer}/mcp`) used as the `aud` for minted
    /// MCP access tokens (RFC 8707). Must match the `resource` advertised in the
    /// protected-resource metadata and the audience the `/mcp` endpoint checks
    /// (TM-MCP-006).
    fn mcp_resource(&self) -> String {
        format!("{}/mcp", self.issuer_url.trim_end_matches('/'))
    }
}

impl FromRef<McpOAuthState> for AuthState {
    fn from_ref(state: &McpOAuthState) -> Self {
        state.auth.clone()
    }
}

// ============================================
// Routes
// ============================================

/// Create root-level MCP OAuth routes (metadata + all OAuth endpoints).
///
/// All OAuth routes live at the server root — not under the API prefix —
/// because the authorize endpoint is browser-facing and the MCP spec
/// discovers them via `/.well-known/oauth-authorization-server`.
pub fn mcp_oauth_routes(state: McpOAuthState) -> Router {
    Router::new()
        // RFC 9728 §3.1 path-derived discovery: resource `{root}/mcp` →
        // PRM at `{root}/.well-known/oauth-protected-resource/mcp`. Real MCP
        // providers (Atlassian, etc.) only serve the path-specific URL, so
        // OSS canonicalises on it too and SaaS no longer needs a root alias.
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(oauth_protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_server_metadata),
        )
        // TM-DOS: the registration endpoint is unauthenticated by design (RFC 7591),
        // so an attacker could create unbounded `oauth_clients` rows. Apply the same
        // per-IP "register" rate limit the builtin signup endpoint uses.
        .route(
            "/oauth/register",
            post(oauth_register).route_layer(middleware::from_fn_with_state(
                state.clone(),
                rate_limit_register,
            )),
        )
        .route(
            "/oauth/authorize",
            get(oauth_authorize).post(oauth_authorize_confirm),
        )
        .route("/oauth/token", post(oauth_token))
        .with_state(state)
}

/// Per-IP rate limit middleware for the unauthenticated `/oauth/register`
/// endpoint (TM-DOS). Reuses the shared `register` limit and the same trusted-proxy
/// client-IP extraction the rest of the auth endpoints use, so the limit cannot be
/// bypassed by spoofing forwarding headers from an untrusted peer. On breach it
/// returns an OAuth-shaped error with HTTP 429.
async fn rate_limit_register(
    State(state): State<McpOAuthState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let ip = extract_client_ip(&req);
    if state.rate_limiter.check_register(ip).await.is_err() {
        tracing::warn!(%ip, "MCP OAuth: registration rate limit exceeded");
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "60")],
            Json(OAuthErrorResponse {
                error: "too_many_requests".to_string(),
                error_description: Some(
                    "Too many client registrations. Please try again later.".to_string(),
                ),
            }),
        )
            .into_response();
    }
    next.run(req).await
}

// ============================================
// Helpers
// ============================================

/// Try to resolve an authenticated user from the cookie jar.
/// Returns `None` when there is no valid session (no cookie or invalid token).
/// In `AuthMode::None`, returns the anonymous user.
async fn try_resolve_user(state: &McpOAuthState, jar: &CookieJar) -> Option<AuthUser> {
    if state.auth.config.mode == AuthMode::None {
        return Some(AuthUser::anonymous());
    }
    let token = jar.get("access_token")?.value().to_owned();
    state.auth.backend.validate_token(&token).await.ok()
}

// ============================================
// Handlers
// ============================================

/// GET /.well-known/oauth-protected-resource/mcp — Protected resource metadata (RFC 9728)
///
/// MCP clients fetch this first to discover which authorization server protects
/// the resource. Path-derived per RFC 9728 §3.1 for the `/mcp` resource.
async fn oauth_protected_resource_metadata(
    State(state): State<McpOAuthState>,
) -> Json<OAuthProtectedResourceMetadata> {
    tracing::debug!("MCP OAuth: protected resource metadata requested");
    let issuer = state.issuer_url.trim_end_matches('/');
    Json(OAuthProtectedResourceMetadata {
        resource: format!("{issuer}/mcp"),
        authorization_servers: vec![issuer.to_string()],
        bearer_methods_supported: vec!["header".to_string()],
    })
}

/// GET /.well-known/oauth-authorization-server — Server metadata
async fn oauth_server_metadata(State(state): State<McpOAuthState>) -> Json<OAuthServerMetadata> {
    tracing::debug!("MCP OAuth: authorization server metadata requested");
    let issuer = state.issuer_url.trim_end_matches('/');
    Json(OAuthServerMetadata {
        issuer: issuer.to_string(),
        authorization_endpoint: format!("{issuer}/oauth/authorize"),
        token_endpoint: format!("{issuer}/oauth/token"),
        registration_endpoint: format!("{issuer}/oauth/register"),
        response_types_supported: vec!["code".to_string()],
        grant_types_supported: vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ],
        code_challenge_methods_supported: vec!["S256".to_string()],
        token_endpoint_auth_methods_supported: vec![
            "none".to_string(),
            "client_secret_post".to_string(),
        ],
        scopes_supported: vec!["mcp".to_string()],
        authorization_response_iss_parameter_supported: true,
    })
}

/// POST /oauth/register — Dynamic Client Registration (RFC 7591).
///
/// DCR is deprecated as of MCP 2026-07-28 in favor of Client ID Metadata
/// Documents, but stays supported through the deprecation window because it is
/// how every MCP client in the field registers today. `registration_endpoint`
/// therefore remains advertised in server metadata.
async fn oauth_register(
    State(state): State<McpOAuthState>,
    Json(req): Json<OAuthRegisterRequest>,
) -> Result<(StatusCode, Json<OAuthRegisterResponse>), OAuthErrorResponse> {
    tracing::info!(client_name = %req.client_name, "MCP OAuth: client registration");
    // Validate
    if req.client_name.is_empty() || req.client_name.len() > 255 {
        return Err(OAuthErrorResponse {
            error: "invalid_client_metadata".to_string(),
            error_description: Some("client_name must be 1-255 characters".to_string()),
        });
    }
    if req.redirect_uris.is_empty() {
        return Err(OAuthErrorResponse {
            error: "invalid_client_metadata".to_string(),
            error_description: Some("At least one redirect_uri is required".to_string()),
        });
    }
    for uri in &req.redirect_uris {
        if let Err(reason) = validate_redirect_uri(uri) {
            tracing::warn!(client_name = %req.client_name, reason, "MCP OAuth: rejected redirect_uri");
            return Err(OAuthErrorResponse {
                error: "invalid_redirect_uri".to_string(),
                error_description: Some(reason.to_string()),
            });
        }
        // A client that declares itself `web` has no local process to receive a
        // loopback callback; such a registration is a misconfiguration at best
        // and an attempt to borrow native-client leniency at worst.
        if req.application_type == Some(OAuthApplicationType::Web) && is_loopback_http_uri(uri) {
            tracing::warn!(
                client_name = %req.client_name,
                "MCP OAuth: rejected loopback redirect_uri for a web application_type"
            );
            return Err(OAuthErrorResponse {
                error: "invalid_redirect_uri".to_string(),
                error_description: Some(
                    "loopback redirect_uri requires application_type \"native\"".to_string(),
                ),
            });
        }
    }

    // Generate client credentials
    let client_id = format!("mcp_client_{}", generate_random_hex());
    let client_secret = format!("mcp_secret_{}", generate_random_hex());
    let client_secret_hash = hash_value(&client_secret);

    let redirect_uris_json =
        serde_json::to_value(&req.redirect_uris).map_err(|_| OAuthErrorResponse {
            error: "invalid_client_metadata".to_string(),
            error_description: Some("Invalid redirect_uris".to_string()),
        })?;

    state
        .db
        .create_oauth_client(CreateOAuthClientRow {
            client_id: client_id.clone(),
            client_secret_hash,
            client_name: req.client_name.clone(),
            redirect_uris: redirect_uris_json,
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to create OAuth client: {}", e);
            OAuthErrorResponse {
                error: "server_error".to_string(),
                error_description: Some("Failed to register client".to_string()),
            }
        })?;

    Ok((
        StatusCode::CREATED,
        Json(OAuthRegisterResponse {
            client_id,
            client_secret,
            client_name: req.client_name,
            redirect_uris: req.redirect_uris,
        }),
    ))
}

/// GET /oauth/authorize — Authorization endpoint (requires authenticated user)
///
/// If the user has no valid session, redirect to the frontend login page with
/// `return_to` pointing back here so the browser lands on the authorize flow
/// after authentication.
async fn oauth_authorize(
    State(state): State<McpOAuthState>,
    original_uri: axum::extract::OriginalUri,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    jar: CookieJar,
    Query(query): Query<OAuthAuthorizeQuery>,
) -> Result<Response, AuthError> {
    tracing::debug!(client_id = %query.client_id, "MCP OAuth: authorize request");
    // Try to resolve user from cookie session (browser flow)
    let user = match try_resolve_user(&state, &jar).await {
        Some(u) => u,
        None => {
            tracing::debug!("MCP OAuth: no session, redirecting to login");
            // Preserve the full original URI (including `resource` and any other
            // query params) so nothing is lost across the login redirect.
            let authorize_path = original_uri
                .path_and_query()
                .map(|pq| pq.as_str())
                .unwrap_or("/oauth/authorize");
            let frontend = state
                .login_origin
                .as_deref()
                .unwrap_or(&state.frontend_url)
                .trim_end_matches('/');
            let login_url = format!(
                "{}/login?return_to={}",
                frontend,
                urlencoding::encode(authorize_path)
            );
            return Ok(Redirect::temporary(&login_url).into_response());
        }
    };
    let ip = audit::client_ip_from_connect_info(connect_info, &headers);

    // Validate response_type
    if query.response_type != "code" {
        return Err(AuthError::unauthorized("Unsupported response_type"));
    }

    // Validate PKCE method
    if query.code_challenge_method != "S256" {
        return Err(AuthError::unauthorized(
            "Only S256 code_challenge_method is supported",
        ));
    }

    // Look up client
    let client = state
        .db
        .get_oauth_client_by_client_id(&query.client_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to look up OAuth client: {}", e);
            AuthError::unauthorized("Invalid client_id")
        })?
        .ok_or_else(|| AuthError::unauthorized("Invalid client_id"))?;

    // Validate redirect_uri against registered URIs
    let registered_uris: Vec<String> =
        serde_json::from_value(client.redirect_uris).unwrap_or_default();
    if !registered_uris.contains(&query.redirect_uri) {
        return Err(AuthError::unauthorized("Invalid redirect_uri"));
    }
    // Defense-in-depth: reject unsafe schemes even if a legacy client managed to
    // register one before scheme validation existed.
    if validate_redirect_uri(&query.redirect_uri).is_err() {
        return Err(AuthError::unauthorized("Invalid redirect_uri"));
    }

    // Anti-CSRF: bind the confirmation form to this session with a short-lived
    // signed consent token instead of a separate cookie. The session cookie
    // that authenticates the POST reliably round-trips from real MCP-client
    // browser contexts; a freshly-set second cookie does not always (popup /
    // embedded webview / partitioned storage), which surfaced as a spurious
    // "Missing CSRF cookie" 401 on confirm. See `generate_oauth_consent_token`.
    let csrf_token = state
        .jwt_service
        .generate_oauth_consent_token(user.id)
        .map_err(|e| {
            tracing::error!("Failed to mint OAuth consent token: {}", e);
            AuthError::unauthorized("Failed to start authorization")
        })?;

    let confirm_page = render_authorize_confirm_page(
        &query,
        &client.client_name,
        &user,
        &csrf_token,
        state.issuer_url.trim_end_matches('/'),
    );
    audit::emit(
        state.db.clone(),
        user.organizations
            .first()
            .map(|o| o.org_id)
            .unwrap_or(everruns_core::DEFAULT_ORG_ID),
        Some(user.id),
        "auth.mcp_oauth.authorize.prompt",
        ip,
        serde_json::json!({"client_id": query.client_id}),
    );
    let mut response = Html(confirm_page).into_response();
    // Chrome checks `form-action` against every hop of a form-submission
    // redirect chain, so the baseline `form-action 'self'` silently blocks the
    // confirm POST's 302 to the client callback (e.g. a native client's
    // `http://localhost:<port>/callback`). Extend it, for this page only, with
    // the redirect origin — already validated against the registered client.
    if let Some(csp) = url::Url::parse(&query.redirect_uri)
        .ok()
        .map(|u| u.origin().ascii_serialization())
        .and_then(|origin| {
            axum::http::HeaderValue::from_str(&format!(
                "{} {origin}",
                crate::app_builder::BASE_CONTENT_SECURITY_POLICY
            ))
            .ok()
        })
    {
        response
            .headers_mut()
            .insert(axum::http::header::CONTENT_SECURITY_POLICY, csp);
    }
    Ok(response)
}

async fn oauth_authorize_confirm(
    State(state): State<McpOAuthState>,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    jar: CookieJar,
    Form(form): Form<OAuthAuthorizeConfirmForm>,
) -> Result<Response, AuthError> {
    let ip = audit::client_ip_from_connect_info(connect_info, &headers);
    let user = try_resolve_user(&state, &jar)
        .await
        .ok_or_else(|| AuthError::unauthorized("Authentication required"))?;

    // Anti-CSRF: the consent token must be one this server signed (proves it
    // originated from our rendered confirm page) and must be bound to the same
    // user the session authenticates as. Both checks are required: the
    // signature alone proves issuance, the `sub` match prevents replaying a
    // token minted for a different session. See `generate_oauth_consent_token`.
    let consent = state
        .jwt_service
        .validate_oauth_consent_token(&form.csrf_token)
        .map_err(|_| AuthError::unauthorized("Invalid or expired authorization request"))?;
    if !constant_time_eq(consent.sub.as_bytes(), user.id.to_string().as_bytes()) {
        return Err(AuthError::unauthorized(
            "Authorization request user mismatch",
        ));
    }

    if form.response_type != "code" {
        return Err(AuthError::unauthorized("Unsupported response_type"));
    }
    if form.code_challenge_method != "S256" {
        return Err(AuthError::unauthorized(
            "Only S256 code_challenge_method is supported",
        ));
    }

    let query = OAuthAuthorizeQuery {
        client_id: form.client_id,
        redirect_uri: form.redirect_uri,
        response_type: form.response_type,
        code_challenge: form.code_challenge,
        code_challenge_method: form.code_challenge_method,
        state: form.state,
        scope: form.scope,
        resource: None,
    };

    validate_authorize_client(&state, &query).await?;
    let redirect_url = issue_authorization_code(&state, &query, &user, ip).await?;

    Ok(Redirect::to(&redirect_url).into_response())
}

async fn validate_authorize_client(
    state: &McpOAuthState,
    query: &OAuthAuthorizeQuery,
) -> Result<(), AuthError> {
    let client = state
        .db
        .get_oauth_client_by_client_id(&query.client_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to look up OAuth client: {}", e);
            AuthError::unauthorized("Invalid client_id")
        })?
        .ok_or_else(|| AuthError::unauthorized("Invalid client_id"))?;
    let registered_uris: Vec<String> =
        serde_json::from_value(client.redirect_uris).unwrap_or_default();
    if !registered_uris.contains(&query.redirect_uri) {
        return Err(AuthError::unauthorized("Invalid redirect_uri"));
    }
    if validate_redirect_uri(&query.redirect_uri).is_err() {
        return Err(AuthError::unauthorized("Invalid redirect_uri"));
    }
    Ok(())
}

async fn issue_authorization_code(
    state: &McpOAuthState,
    query: &OAuthAuthorizeQuery,
    user: &AuthUser,
    ip: Option<String>,
) -> Result<String, AuthError> {
    let org_id = user
        .organizations
        .first()
        .map(|o| o.org_id)
        .unwrap_or(everruns_core::DEFAULT_ORG_ID);

    let code = generate_random_hex();
    let code_hash = hash_value(&code);
    let expires_at = Utc::now() + Duration::seconds(AUTH_CODE_TTL_SECS);

    state
        .db
        .create_oauth_authorization_code(CreateOAuthAuthorizationCodeRow {
            code_hash,
            client_id: query.client_id.clone(),
            user_id: user.id,
            org_id,
            redirect_uri: query.redirect_uri.clone(),
            code_challenge: query.code_challenge.clone(),
            code_challenge_method: query.code_challenge_method.clone(),
            scope: query.scope.clone(),
            expires_at,
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to create authorization code: {}", e);
            AuthError::unauthorized("Failed to create authorization code")
        })?;

    audit::emit(
        state.db.clone(),
        org_id,
        Some(user.id),
        "auth.mcp_oauth.authorize",
        ip,
        serde_json::json!({"client_id": query.client_id}),
    );

    tracing::info!(user_id = %user.id, client_id = %query.client_id, redirect_uri = %query.redirect_uri, "MCP OAuth: auth code issued, redirecting");
    // RFC 6749 §3.1.2 — the registered redirect URI MAY contain a query
    // component, which must be retained when appending `code` and `state`.
    // Use `Url::query_pairs_mut` so a redirect like `https://app/cb?next=1`
    // becomes `https://app/cb?next=1&code=...&state=...`, not the malformed
    // `...?next=1?code=...` that naive string concatenation would produce.
    // RFC 9207 — `iss` lets the client confirm which authorization server
    // answered, defeating mix-up attacks when it talks to several. Advertised
    // via `authorization_response_iss_parameter_supported` in server metadata,
    // which obliges us to send it on every authorization response.
    build_oauth_redirect_url(
        &query.redirect_uri,
        &[
            ("code", &code),
            ("state", &query.state),
            ("iss", state.issuer_url.trim_end_matches('/')),
        ],
    )
    .map_err(|_| AuthError::unauthorized("Invalid redirect_uri"))
}

fn build_oauth_redirect_url(
    redirect_uri: &str,
    params: &[(&str, &str)],
) -> Result<String, url::ParseError> {
    let mut redirect = url::Url::parse(redirect_uri)?;
    {
        let mut pairs = redirect.query_pairs_mut();
        for (key, value) in params {
            pairs.append_pair(key, value);
        }
    }
    Ok(redirect.into())
}

fn render_authorize_confirm_page(
    query: &OAuthAuthorizeQuery,
    client_name: &str,
    user: &AuthUser,
    csrf_token: &str,
    issuer: &str,
) -> String {
    let normalized_scope = normalize_scope(&query.scope);
    // RFC 9207 applies to error responses too — a client that validates `iss`
    // must be able to validate the denial it acts on.
    let cancel_url = build_oauth_redirect_url(
        &query.redirect_uri,
        &[
            ("error", "access_denied"),
            ("state", &query.state),
            ("iss", issuer),
        ],
    )
    .unwrap_or_else(|_| "#".to_string());
    let scope_chips = render_scope_chips(&normalized_scope);

    let stylesheet = r#"
:root {
  color-scheme: light;
  font-family: Geist, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  color: #0a0a0a;
  background: #f5f5f5;
}
* { box-sizing: border-box; }
body {
  margin: 0;
  min-height: 100vh;
  display: grid;
  place-items: center;
  padding: 32px 16px;
}
.shell {
  width: min(100%, 720px);
  background: #ffffff;
  border: 1px solid #d9d9d9;
  box-shadow: 0 24px 80px rgba(10, 10, 10, 0.12);
}
.header {
  padding: 32px 36px 24px;
  border-top: 4px solid #d4a43a;
  border-bottom: 1px solid #e5e5e5;
}
.eyebrow {
  margin: 0 0 12px;
  color: #666666;
  font-size: 13px;
  font-weight: 600;
  letter-spacing: 0;
  text-transform: uppercase;
}
h1 {
  margin: 0;
  font-size: 30px;
  line-height: 1.15;
  font-weight: 650;
  letter-spacing: 0;
}
.client {
  margin-top: 14px;
  color: #404040;
  font-size: 15px;
  line-height: 1.5;
}
.content { padding: 28px 36px 32px; }
.summary {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
  margin-bottom: 28px;
}
.field {
  min-width: 0;
  padding: 14px 16px;
  border: 1px solid #e5e5e5;
  background: #fafafa;
}
.field-label {
  display: block;
  margin-bottom: 6px;
  color: #666666;
  font-size: 12px;
  font-weight: 650;
  text-transform: uppercase;
}
.field-value {
  display: block;
  overflow-wrap: anywhere;
  font-size: 14px;
  line-height: 1.4;
}
.scope-row {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.scope {
  display: inline-flex;
  align-items: center;
  min-height: 28px;
  padding: 3px 10px;
  border: 1px solid #d4a43a;
  background: rgba(212, 164, 58, 0.12);
  color: #4f3700;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 13px;
}
h2 {
  margin: 0 0 12px;
  font-size: 16px;
  font-weight: 650;
}
.grant-list {
  display: grid;
  gap: 10px;
  margin: 0 0 28px;
  padding: 0;
  list-style: none;
}
.grant-list li {
  padding: 14px 16px;
  border-left: 3px solid #d4a43a;
  background: #f8f8f8;
}
.grant-title {
  display: block;
  margin-bottom: 4px;
  font-weight: 650;
}
.grant-copy {
  display: block;
  color: #404040;
  font-size: 14px;
  line-height: 1.45;
}
.notice {
  margin: 0 0 28px;
  padding: 14px 16px;
  border: 1px solid #d9d9d9;
  color: #404040;
  font-size: 14px;
  line-height: 1.5;
}
.actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px;
}
button,
.cancel {
  min-height: 44px;
  padding: 0 18px;
  border: 1px solid #0a1636;
  font: inherit;
  font-size: 15px;
  font-weight: 650;
  text-decoration: none;
}
button {
  background: #0a1636;
  color: #ffffff;
  cursor: pointer;
}
.cancel {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: #0a1636;
  background: #ffffff;
}
@media (max-width: 640px) {
  body { padding: 0; place-items: stretch; }
  .shell { width: 100%; min-height: 100vh; border-width: 0; }
  .header, .content { padding-left: 20px; padding-right: 20px; }
  .summary { grid-template-columns: 1fr; }
  h1 { font-size: 25px; }
  button, .cancel { width: 100%; }
}
"#;

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Authorize MCP Client</title>
  <style>{style}</style>
</head>
<body>
  <main class="shell">
    <section class="header">
      <p class="eyebrow">Everruns MCP OAuth</p>
      <h1>Authorize this MCP client?</h1>
      <p class="client"><strong>{client_name}</strong> is requesting access to Everruns as <strong>{user_name}</strong>.</p>
    </section>
    <section class="content">
      <div class="summary" aria-label="Authorization details">
        <div class="field">
          <span class="field-label">Client ID</span>
          <span class="field-value">{client_id}</span>
        </div>
        <div class="field">
          <span class="field-label">Signed in as</span>
          <span class="field-value">{user_name} &lt;{user_email}&gt;</span>
        </div>
        <div class="field">
          <span class="field-label">Redirect URI</span>
          <span class="field-value">{redirect_uri}</span>
        </div>
        <div class="field">
          <span class="field-label">Requested scope</span>
          <span class="field-value scope-row">{scope_chips}</span>
        </div>
      </div>

      <h2>Approving allows this client to</h2>
      <ul class="grant-list">
        <li>
          <span class="grant-title">Use Everruns MCP</span>
          <span class="grant-copy">Call MCP tools and read MCP resources across the organizations you can access.</span>
        </li>
        <li>
          <span class="grant-title">Act as your signed-in user</span>
          <span class="grant-copy">Requests are authorized as {user_name} using your access to each organization you are a member of.</span>
        </li>
        <li>
          <span class="grant-title">Keep a connection token</span>
          <span class="grant-copy">The client receives an access token and refresh token. It does not receive your Everruns password.</span>
        </li>
      </ul>

      <p class="notice">Only approve clients you recognize. The client will return to its registered redirect URI after this step.</p>

      <form method="post" action="/oauth/authorize">
        <input type="hidden" name="client_id" value="{client_id}">
        <input type="hidden" name="redirect_uri" value="{redirect_uri}">
        <input type="hidden" name="response_type" value="{response_type}">
        <input type="hidden" name="code_challenge" value="{code_challenge}">
        <input type="hidden" name="code_challenge_method" value="{code_challenge_method}">
        <input type="hidden" name="state" value="{state}">
        <input type="hidden" name="scope" value="{scope}">
        <input type="hidden" name="csrf_token" value="{csrf_token}">
        <div class="actions">
          <button type="submit">Authorize client</button>
          <a class="cancel" href="{cancel_url}">Cancel</a>
        </div>
      </form>
    </section>
  </main>
</body>
</html>"#,
        style = stylesheet,
        client_name = escape_html(client_name),
        client_id = escape_html(&query.client_id),
        user_name = escape_html(&user.name),
        user_email = escape_html(&user.email),
        redirect_uri = escape_html(&query.redirect_uri),
        scope_chips = scope_chips,
        response_type = escape_html(&query.response_type),
        code_challenge = escape_html(&query.code_challenge),
        code_challenge_method = escape_html(&query.code_challenge_method),
        state = escape_html(&query.state),
        scope = escape_html(&normalized_scope),
        csrf_token = escape_html(csrf_token),
        cancel_url = escape_html(&cancel_url),
    )
}

fn normalize_scope(scope: &str) -> String {
    let normalized = scope.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        default_scope()
    } else {
        normalized
    }
}

fn render_scope_chips(scope: &str) -> String {
    scope
        .split_whitespace()
        .map(|scope| format!(r#"<span class="scope">{}</span>"#, escape_html(scope)))
        .collect::<Vec<_>>()
        .join("")
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// POST /oauth/token — Token exchange (authorization_code or refresh_token grant)
///
/// Accepts both `application/x-www-form-urlencoded` (per OAuth spec) and
/// `application/json` (sent by some MCP clients like Cursor).
async fn oauth_token(
    State(state): State<McpOAuthState>,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<OAuthTokenResponse>, OAuthErrorResponse> {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    tracing::debug!(content_type, "MCP OAuth: token request received");

    let req: OAuthTokenRequest = if content_type.contains("application/json") {
        serde_json::from_slice(&body).map_err(|e| {
            tracing::warn!(%e, "Failed to parse JSON token request");
            OAuthErrorResponse {
                error: "invalid_request".to_string(),
                error_description: Some(format!("Invalid JSON body: {e}")),
            }
        })?
    } else {
        serde_urlencoded::from_bytes(&body).map_err(|e| {
            tracing::warn!(%e, "Failed to parse form token request");
            OAuthErrorResponse {
                error: "invalid_request".to_string(),
                error_description: Some(format!("Invalid form body: {e}")),
            }
        })?
    };
    let ip = audit::client_ip_from_connect_info(connect_info, &headers);

    tracing::debug!(grant_type = %req.grant_type, "MCP OAuth: processing token grant");

    let result = match req.grant_type.as_str() {
        "authorization_code" => handle_authorization_code_grant(&state, &req, ip).await,
        "refresh_token" => handle_refresh_token_grant(&state, &req, ip).await,
        _ => Err(OAuthErrorResponse {
            error: "unsupported_grant_type".to_string(),
            error_description: Some("Supported: authorization_code, refresh_token".to_string()),
        }),
    };
    match &result {
        Ok(_) => tracing::info!(grant_type = %req.grant_type, "MCP OAuth: token grant succeeded"),
        Err(e) => {
            tracing::warn!(grant_type = %req.grant_type, error = %e.error, desc = ?e.error_description, "MCP OAuth: token grant failed")
        }
    }
    result
}

async fn handle_authorization_code_grant(
    state: &McpOAuthState,
    req: &OAuthTokenRequest,
    ip: Option<String>,
) -> Result<Json<OAuthTokenResponse>, OAuthErrorResponse> {
    let code = req.code.as_deref().ok_or_else(|| OAuthErrorResponse {
        error: "invalid_request".to_string(),
        error_description: Some("code is required".to_string()),
    })?;
    let client_id = req.client_id.as_deref().ok_or_else(|| OAuthErrorResponse {
        error: "invalid_request".to_string(),
        error_description: Some("client_id is required".to_string()),
    })?;
    let redirect_uri = req
        .redirect_uri
        .as_deref()
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_request".to_string(),
            error_description: Some("redirect_uri is required".to_string()),
        })?;
    let code_verifier = req
        .code_verifier
        .as_deref()
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_request".to_string(),
            error_description: Some("code_verifier is required".to_string()),
        })?;

    // Validate client exists
    let client = state
        .db
        .get_oauth_client_by_client_id(client_id)
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_client".to_string(),
            error_description: Some("Unknown client_id".to_string()),
        })?;

    // Verify client secret if provided (confidential client).
    // Public clients (most MCP clients) rely on PKCE alone.
    if let Some(client_secret) = req.client_secret.as_deref() {
        let secret_hash = hash_value(client_secret);
        if !constant_time_eq(secret_hash.as_bytes(), client.client_secret_hash.as_bytes()) {
            return Err(OAuthErrorResponse {
                error: "invalid_client".to_string(),
                error_description: Some("Invalid client_secret".to_string()),
            });
        }
    }

    // Look up and consume authorization code
    let code_hash = hash_value(code);
    let auth_code = state
        .db
        .get_oauth_authorization_code_by_hash(&code_hash)
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("Invalid or expired authorization code".to_string()),
        })?;

    // Check not already consumed
    if auth_code.consumed {
        return Err(OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("Authorization code already used".to_string()),
        });
    }

    // Validate client_id matches
    if auth_code.client_id != client_id {
        return Err(OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("client_id mismatch".to_string()),
        });
    }

    // Validate redirect_uri matches
    if auth_code.redirect_uri != redirect_uri {
        return Err(OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("redirect_uri mismatch".to_string()),
        });
    }

    // Verify PKCE
    if !verify_pkce_s256(code_verifier, &auth_code.code_challenge) {
        return Err(OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("PKCE verification failed".to_string()),
        });
    }

    // Consume the code (one-time use). This is an atomic
    // `UPDATE ... WHERE consumed = FALSE RETURNING ...` that returns `false`
    // when a concurrent request already consumed this code. The earlier
    // `auth_code.consumed` check is a stale read taken before PKCE validation,
    // so relying on it alone lets two concurrent `authorization_code` grants
    // for the same code both pass and mint tokens (auth-code replay TOCTOU).
    // Treat losing the atomic consume as `invalid_grant` so only the first
    // redemption succeeds — mirrors the refresh-token rotation gate above.
    let consumed = state
        .db
        .consume_oauth_authorization_code(auth_code.id)
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?;
    if !consumed {
        return Err(OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("Authorization code already used".to_string()),
        });
    }

    // Fetch user to populate token claims
    let user = state
        .db
        .get_user(auth_code.user_id)
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?
        .ok_or_else(|| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: Some("User not found".to_string()),
        })?;

    let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();

    // THREAT[TM-MCP-006]: mint a resource-bound MCP access token (token_type
    // "mcp_access", aud = `{root}/mcp`) rather than a full-API access token, so
    // it is accepted only at `/mcp` and rejected on `/api/*`.
    let access_token = state
        .jwt_service
        .generate_mcp_access_token(
            auth_code.user_id,
            &user.email,
            &user.name,
            &roles,
            &state.mcp_resource(),
        )
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: Some("Failed to generate token".to_string()),
        })?;

    // Generate refresh token
    let refresh_token_raw = generate_random_hex();
    let refresh_token_hash = hash_value(&refresh_token_raw);

    state
        .db
        .create_oauth_refresh_token(CreateOAuthRefreshTokenRow {
            token_hash: refresh_token_hash,
            client_id: client_id.to_string(),
            user_id: auth_code.user_id,
            org_id: auth_code.org_id,
            scope: auth_code.scope,
            expires_at: Utc::now() + Duration::seconds(MCP_REFRESH_TOKEN_LIFETIME_SECS),
        })
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?;

    // Cleanup expired codes (fire and forget)
    let db = state.db.clone();
    tokio::spawn(async move {
        let _ = db.delete_expired_oauth_authorization_codes().await;
    });

    audit::emit(
        state.db.clone(),
        auth_code.org_id,
        Some(auth_code.user_id),
        "auth.mcp_oauth.token",
        ip,
        serde_json::json!({"client_id": client_id, "grant_type": "authorization_code"}),
    );

    Ok(Json(OAuthTokenResponse {
        access_token,
        token_type: "Bearer".to_string(),
        expires_in: state.jwt_service.access_token_lifetime_secs(),
        refresh_token: Some(refresh_token_raw),
    }))
}

async fn handle_refresh_token_grant(
    state: &McpOAuthState,
    req: &OAuthTokenRequest,
    ip: Option<String>,
) -> Result<Json<OAuthTokenResponse>, OAuthErrorResponse> {
    let refresh_token = req
        .refresh_token
        .as_deref()
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_request".to_string(),
            error_description: Some("refresh_token is required".to_string()),
        })?;
    let client_id = req.client_id.as_deref().ok_or_else(|| OAuthErrorResponse {
        error: "invalid_request".to_string(),
        error_description: Some("client_id is required".to_string()),
    })?;
    // Validate client exists
    let client = state
        .db
        .get_oauth_client_by_client_id(client_id)
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_client".to_string(),
            error_description: Some("Unknown client_id".to_string()),
        })?;

    // Verify client secret if provided (confidential client).
    if let Some(client_secret) = req.client_secret.as_deref() {
        let secret_hash = hash_value(client_secret);
        if !constant_time_eq(secret_hash.as_bytes(), client.client_secret_hash.as_bytes()) {
            return Err(OAuthErrorResponse {
                error: "invalid_client".to_string(),
                error_description: Some("Invalid client_secret".to_string()),
            });
        }
    }

    // Look up refresh token
    let token_hash = hash_value(refresh_token);
    let stored_token = state
        .db
        .get_oauth_refresh_token_by_hash(&token_hash)
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("Invalid or expired refresh token".to_string()),
        })?;

    // Validate client_id matches
    if stored_token.client_id != client_id {
        return Err(OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("client_id mismatch".to_string()),
        });
    }

    // Fetch user
    let user = state
        .db
        .get_user(stored_token.user_id)
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?
        .ok_or_else(|| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: Some("User not found".to_string()),
        })?;

    let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();

    // Generate new resource-bound MCP access token (see TM-MCP-006 above).
    let access_token = state
        .jwt_service
        .generate_mcp_access_token(
            stored_token.user_id,
            &user.email,
            &user.name,
            &roles,
            &state.mcp_resource(),
        )
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: Some("Failed to generate token".to_string()),
        })?;

    // Generate new refresh token (rotation)
    let new_refresh_token = generate_random_hex();
    let new_refresh_token_hash = hash_value(&new_refresh_token);

    // Atomically consume the old refresh token before issuing the replacement.
    // The earlier lookup lets us validate client/user and build the response
    // without burning the token on unrelated transient failures; this consume
    // is the single-use gate that closes concurrent refresh races.
    let consumed_token = state
        .db
        .consume_oauth_refresh_token_by_hash(&token_hash)
        .await
        .map_err(|e| {
            tracing::error!("Failed to consume OAuth refresh token: {}", e);
            OAuthErrorResponse {
                error: "server_error".to_string(),
                error_description: None,
            }
        })?
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("Invalid or expired refresh token".to_string()),
        })?;
    if consumed_token.client_id != client_id {
        return Err(OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("client_id mismatch".to_string()),
        });
    }

    state
        .db
        .create_oauth_refresh_token(CreateOAuthRefreshTokenRow {
            token_hash: new_refresh_token_hash,
            client_id: client_id.to_string(),
            user_id: stored_token.user_id,
            org_id: stored_token.org_id,
            scope: stored_token.scope,
            expires_at: Utc::now() + Duration::seconds(MCP_REFRESH_TOKEN_LIFETIME_SECS),
        })
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?;

    audit::emit(
        state.db.clone(),
        stored_token.org_id,
        Some(stored_token.user_id),
        "auth.mcp_oauth.token",
        ip,
        serde_json::json!({"client_id": client_id, "grant_type": "refresh_token"}),
    );

    Ok(Json(OAuthTokenResponse {
        access_token,
        token_type: "Bearer".to_string(),
        expires_in: state.jwt_service.access_token_lifetime_secs(),
        refresh_token: Some(new_refresh_token),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authorize_query() -> OAuthAuthorizeQuery {
        OAuthAuthorizeQuery {
            client_id: "mcp_client_test".to_string(),
            redirect_uri: "https://client.example/callback?next=1".to_string(),
            response_type: "code".to_string(),
            code_challenge: "challenge".to_string(),
            code_challenge_method: "S256".to_string(),
            state: "state value".to_string(),
            scope: "mcp".to_string(),
            resource: None,
        }
    }

    fn auth_user_for_render() -> AuthUser {
        AuthUser {
            id: uuid::Uuid::nil(),
            email: "ava@example.com".to_string(),
            name: "Ava Root".to_string(),
            roles: vec!["admin".to_string()],
            is_platform_user: false,
            auth_method: crate::auth::middleware::AuthMethod::Jwt,
            organizations: vec![everruns_platform::OrgMembership {
                org_id: 42,
                public_id: "org_test".to_string(),
                name: "User Personal".to_string(),
                role: everruns_core::OrgRole::Admin,
            }],
        }
    }

    #[test]
    fn test_generate_random_hex_uniqueness() {
        let a = generate_random_hex();
        let b = generate_random_hex();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn token_hash_matches_sha256_known_vectors() {
        for (input, expected) in [
            (
                "",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                "abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
        ] {
            assert_eq!(hash_value(input), expected);
        }
    }

    #[test]
    fn test_pkce_s256_verification() {
        use base64::Engine;

        // Generate a verifier and compute the challenge
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let hash = Sha256::digest(verifier.as_bytes());
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);

        assert!(verify_pkce_s256(verifier, &challenge));
        assert!(!verify_pkce_s256("wrong-verifier", &challenge));
    }

    #[test]
    fn test_validate_redirect_uri_accepts_safe_schemes() {
        for uri in [
            "https://example.com/cb",
            "https://example.com:8443/cb?next=1",
            "http://localhost/cb",
            "http://localhost:9999/cb",
            "http://127.0.0.1:9999/cb",
            "http://[::1]:9999/cb",
        ] {
            assert!(
                validate_redirect_uri(uri).is_ok(),
                "expected {uri} to be accepted",
            );
        }
    }

    #[test]
    fn test_redirect_url_with_existing_query_preserves_pairs() {
        let s = build_oauth_redirect_url(
            "https://app.example.com/cb?next=1",
            &[("code", "C&D"), ("state", "x y")],
        )
        .unwrap();
        assert!(s.starts_with("https://app.example.com/cb?next=1&"));
        assert!(s.contains("code=C%26D"));
        assert!(s.contains("state=x+y"));
        // No double `?` or naive concatenation.
        assert_eq!(s.matches('?').count(), 1);
    }

    #[test]
    fn test_validate_redirect_uri_rejects_unsafe_schemes() {
        for uri in [
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///tmp/cb",
            "vbscript:msgbox(1)",
            "myapp://callback",
            "http://example.com/cb",     // non-loopback http
            "http://10.0.0.1:9999/cb",   // non-loopback IPv4
            "http://[2001:db8::1]/cb",   // non-loopback IPv6
            "http://localhost.evil.com", // suffix attack
            "//example.com/cb",          // protocol-relative
            "/relative",
            "",
            "https://example.com/cb#frag", // fragment forbidden
            "not a url",
        ] {
            assert!(
                validate_redirect_uri(uri).is_err(),
                "expected {uri} to be rejected",
            );
        }
    }

    #[test]
    fn test_pkce_s256_rfc_example() {
        // RFC 7636 Appendix B test vector
        // verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        // challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(verify_pkce_s256(verifier, challenge));
    }

    #[test]
    fn test_authorize_confirm_page_lists_grant_details() {
        let html = render_authorize_confirm_page(
            &authorize_query(),
            "Cursor",
            &auth_user_for_render(),
            "csrf",
            "https://app.example.com",
        );

        assert!(html.contains("Authorize this MCP client?"));
        assert!(html.contains("<strong>Cursor</strong>"));
        assert!(html.contains("Ava Root &lt;ava@example.com&gt;"));
        // The consent page is intentionally org-free: MCP OAuth tokens are
        // user-scoped and resolve org per request, so showing a single org
        // here would be misleading.
        assert!(!html.contains("Organization"));
        assert!(html.contains("Call MCP tools and read MCP resources"));
        assert!(html.contains("access token and refresh token"));
        assert!(html.contains(r#"<span class="scope">mcp</span>"#));
        assert!(html.contains(r#"name="client_id" value="mcp_client_test""#));
        assert!(
            html.contains(
                "href=\"https://client.example/callback?next=1&amp;error=access_denied&amp;state=state+value&amp;iss=https%3A%2F%2Fapp.example.com\""
            )
        );
    }

    #[test]
    fn test_authorize_confirm_page_escapes_dynamic_values() {
        let mut query = authorize_query();
        query.client_id = "client_<id>".to_string();
        query.redirect_uri = "https://client.example/callback?next=<bad>&ok=1".to_string();
        query.state = "a&b".to_string();
        query.scope = "mcp custom<scope>".to_string();
        let mut user = auth_user_for_render();
        user.name = "Ava \"Root\" <Ops>".to_string();
        user.email = "ava+ops@example.com".to_string();

        let html = render_authorize_confirm_page(
            &query,
            "Cursor <script>alert(\"x\")</script>",
            &user,
            "csrf&token",
            "https://app.example.com",
        );

        assert!(!html.contains("<script>alert"));
        assert!(html.contains("Cursor &lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"));
        assert!(html.contains("client_&lt;id&gt;"));
        assert!(html.contains("Ava &quot;Root&quot; &lt;Ops&gt;"));
        assert!(html.contains("custom&lt;scope&gt;"));
        assert!(html.contains("csrf&amp;token"));
        assert!(html.contains(
            "next=%3Cbad%3E&amp;ok=1&amp;error=access_denied&amp;state=a%26b&amp;iss=https%3A%2F%2Fapp.example.com"
        ));
    }

    #[test]
    fn test_authorize_confirm_page_normalizes_empty_scope() {
        let mut query = authorize_query();
        query.scope = " \t ".to_string();

        let html = render_authorize_confirm_page(
            &query,
            "ChatGPT",
            &auth_user_for_render(),
            "csrf",
            "https://app.example.com",
        );

        assert!(html.contains(r#"<span class="scope">mcp</span>"#));
        assert!(html.contains(r#"name="scope" value="mcp""#));
    }

    #[test]
    fn test_loopback_http_uri_detection() {
        assert!(is_loopback_http_uri("http://localhost:8080/cb"));
        assert!(is_loopback_http_uri("http://127.0.0.1:1455/cb"));
        assert!(is_loopback_http_uri("http://[::1]:9000/cb"));
        // https loopback is a normal web callback, not a native one.
        assert!(!is_loopback_http_uri("https://localhost/cb"));
        assert!(!is_loopback_http_uri("http://evil.example/cb"));
        assert!(!is_loopback_http_uri("not a url"));
    }

    #[test]
    fn test_application_type_deserializes_from_oidc_values() {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            #[serde(default)]
            application_type: Option<OAuthApplicationType>,
        }

        let native: Wrapper = serde_json::from_str(r#"{"application_type":"native"}"#).unwrap();
        assert_eq!(native.application_type, Some(OAuthApplicationType::Native));
        let web: Wrapper = serde_json::from_str(r#"{"application_type":"web"}"#).unwrap();
        assert_eq!(web.application_type, Some(OAuthApplicationType::Web));
        // Omitted stays unstated rather than defaulting to `web`, so the MCP
        // clients in the field today keep registering loopback callbacks.
        let absent: Wrapper = serde_json::from_str("{}").unwrap();
        assert_eq!(absent.application_type, None);
        assert!(serde_json::from_str::<Wrapper>(r#"{"application_type":"desktop"}"#).is_err());
    }

    #[test]
    fn test_authorize_redirect_carries_rfc_9207_issuer() {
        let url = build_oauth_redirect_url(
            "https://client.example/callback",
            &[
                ("code", "abc"),
                ("state", "xyz"),
                ("iss", "https://app.example.com"),
            ],
        )
        .unwrap();
        assert!(
            url.contains("iss=https%3A%2F%2Fapp.example.com"),
            "authorization response must identify the issuer: {url}"
        );
    }
}

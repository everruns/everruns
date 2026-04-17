// CLI authentication endpoints (localhost OAuth callback flow)
//
// Decision: CLI auth uses a trampoline pattern — CLI starts a localhost HTTP server,
// server redirects browser back to localhost after login, CLI exchanges code for API key.
// Decision: CLI auth sessions are short-lived (5 min TTL), stored in DB.
// Decision: Exchange code is one-time use — consumed on exchange.

use super::{
    api_key::generate_api_key,
    audit,
    middleware::{AuthError, AuthState, AuthUser},
    routes::OrgMembershipResponse,
};
use crate::storage::StorageBackend;
use crate::storage::models::{CreateApiKeyRow, CreateCliAuthSessionRow};
use axum::{
    Json, Router,
    extract::{FromRef, Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

/// CLI auth session TTL (5 minutes)
const CLI_AUTH_SESSION_TTL_SECS: i64 = 300;

/// Generate a random hex string (32 chars = 16 bytes)
fn generate_random_hex() -> String {
    let mut rng = rand::rng();
    let bytes: [u8; 16] = rng.random();
    hex::encode(bytes)
}

/// Build the relative `return_to` path that navigates the browser to the CLI
/// auth callback endpoint after login.
///
/// The CLI callback is mounted on the API (`/v1/auth/cli/callback`). We derive
/// the API path prefix (e.g. `/api`) by stripping `frontend_url` from
/// `base_url`. When both live on the same origin with a path prefix, the
/// browser can reach the backend through the standard frontend proxy. When
/// they differ (rare dev setup), no prefix is added and operators must ensure
/// the backend is reachable from the frontend origin.
///
/// The result is always a relative path starting with `/`. We only accept the
/// stripped remainder when it is empty or starts with `/` — otherwise the strip
/// landed mid-path-segment (e.g. `frontend_url=https://app` against
/// `base_url=https://app.example.com/api` would yield `.example.com/api`),
/// which would produce a `return_to` rejected by the UI sanitizer.
fn build_cli_callback_path(frontend_url: &str, base_url: &str, auth_state: &str) -> String {
    let stripped = base_url
        .strip_prefix(frontend_url)
        .map(|s| s.trim_end_matches('/'));
    let api_path_prefix = match stripped {
        Some(s) if s.is_empty() || s.starts_with('/') => s,
        _ => "",
    };
    format!(
        "{}/v1/auth/cli/callback?state={}",
        api_path_prefix, auth_state
    )
}

// ============================================
// Request/Response types
// ============================================

/// POST /v1/auth/cli/start request
#[derive(Debug, Deserialize, ToSchema)]
pub struct CliAuthStartRequest {
    /// Port the CLI is listening on for the callback
    pub redirect_port: u16,
}

/// POST /v1/auth/cli/start response
#[derive(Debug, Serialize, ToSchema)]
pub struct CliAuthStartResponse {
    /// URL to open in the browser for login
    pub auth_url: String,
    /// State token (for correlation)
    pub state: String,
}

/// GET /v1/auth/cli/callback query parameters
#[derive(Debug, Deserialize)]
pub struct CliCallbackQuery {
    pub state: String,
}

/// POST /v1/auth/cli/exchange request
#[derive(Debug, Deserialize, ToSchema)]
pub struct CliExchangeRequest {
    /// One-time exchange code from the callback
    pub code: String,
    /// Client hostname for API key metadata
    #[serde(default)]
    pub hostname: Option<String>,
    /// Client OS for API key metadata
    #[serde(default)]
    pub os: Option<String>,
}

/// POST /v1/auth/cli/exchange response
#[derive(Debug, Serialize, ToSchema)]
pub struct CliExchangeResponse {
    /// Full API key (shown only once)
    pub api_key: String,
    /// User info
    pub user: CliUserInfo,
    /// User's organizations
    pub orgs: Vec<OrgMembershipResponse>,
}

/// User info returned from CLI exchange
#[derive(Debug, Serialize, ToSchema)]
pub struct CliUserInfo {
    pub id: String,
    pub email: String,
    pub name: String,
}

// ============================================
// State
// ============================================

/// State for CLI auth routes — decoupled from any specific AuthBackend.
///
/// Embedders construct this with their own URLs and mount via `cli_auth_routes()`.
#[derive(Clone)]
pub struct CliAuthState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
    /// Frontend URL for login redirects (e.g. `https://app.example.com`)
    pub frontend_url: String,
    /// Full backend base URL including any path prefix (e.g. `https://app.example.com/api`).
    /// Used directly to construct callback URLs — no env-var lookup is performed.
    pub base_url: String,
}

impl FromRef<CliAuthState> for AuthState {
    fn from_ref(state: &CliAuthState) -> Self {
        state.auth.clone()
    }
}

// ============================================
// Routes
// ============================================

/// Create CLI auth routes
pub fn cli_auth_routes(state: CliAuthState) -> Router {
    Router::new()
        .route("/v1/auth/cli/start", post(cli_auth_start))
        .route("/v1/auth/cli/callback", get(cli_auth_callback))
        .route("/v1/auth/cli/exchange", post(cli_auth_exchange))
        .with_state(state)
}

/// Create root-level CLI auth routes.
pub fn cli_auth_public_routes(state: CliAuthState) -> Router {
    Router::new()
        .route("/cli/login-success", get(cli_login_success))
        .with_state(state)
}

/// POST /v1/auth/cli/start — Begin CLI auth flow
///
/// Creates a pending CLI auth session and returns the login URL.
/// The CLI should open this URL in the user's browser.
async fn cli_auth_start(
    State(state): State<CliAuthState>,
    headers: HeaderMap,
    Json(req): Json<CliAuthStartRequest>,
) -> Result<Json<CliAuthStartResponse>, AuthError> {
    let ip = audit::client_ip(&headers);

    // Generate random state and exchange code
    let auth_state = generate_random_hex();
    let exchange_code = generate_random_hex();

    let expires_at = Utc::now() + Duration::seconds(CLI_AUTH_SESSION_TTL_SECS);

    // Store pending session
    state
        .db
        .create_cli_auth_session(CreateCliAuthSessionRow {
            state: auth_state.clone(),
            exchange_code,
            redirect_port: req.redirect_port as i32,
            expires_at,
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to create CLI auth session: {}", e);
            AuthError::unauthorized("Failed to start CLI login")
        })?;

    // Build auth URL — points to the server's login page with a `return_to`
    // pointing back to our CLI callback. `return_to` is the single public
    // login-page contract for auth resume (see specs/authentication.md).
    //
    // We derive a browser-relative path rather than leaking the full backend URL
    // into `return_to`, because `return_to` is restricted to relative paths on
    // the frontend origin. The API path prefix is derived by stripping
    // `frontend_url` from `base_url` (e.g. `/api`) — in the rare case the two
    // live on different origins (dev without reverse proxy), there is no prefix
    // and the browser would need the backend mounted under the frontend origin
    // for CLI login to work.
    let callback_path = build_cli_callback_path(
        state.frontend_url.trim_end_matches('/'),
        state.base_url.trim_end_matches('/'),
        &auth_state,
    );
    let auth_url = format!(
        "{}/login?return_to={}",
        state.frontend_url.trim_end_matches('/'),
        urlencoding::encode(&callback_path)
    );

    audit::emit(
        state.db.clone(),
        everruns_core::DEFAULT_ORG_ID,
        None,
        "auth.cli.start",
        ip,
        serde_json::json!({"redirect_port": req.redirect_port}),
    );

    Ok(Json(CliAuthStartResponse {
        auth_url,
        state: auth_state,
    }))
}

/// GET /v1/auth/cli/callback?state=... — Server-side callback after login
///
/// Called after the user completes login. Associates the authenticated user
/// with the pending CLI session and redirects to the CLI's localhost server.
async fn cli_auth_callback(
    State(state): State<CliAuthState>,
    headers: HeaderMap,
    user: AuthUser,
    Query(query): Query<CliCallbackQuery>,
) -> Result<Response, AuthError> {
    let ip = audit::client_ip(&headers);

    // Look up pending session by state
    let session = state
        .db
        .get_cli_auth_session_by_state(&query.state)
        .await
        .map_err(|e| {
            tracing::error!("Failed to look up CLI auth session: {}", e);
            AuthError::unauthorized("Invalid CLI login session")
        })?
        .ok_or_else(|| AuthError::unauthorized("Invalid or expired CLI login session"))?;

    if session.completed {
        return Err(AuthError::unauthorized("CLI login session already used"));
    }

    // Associate user with session
    state
        .db
        .complete_cli_auth_session(session.id, user.id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to complete CLI auth session: {}", e);
            AuthError::unauthorized("Failed to complete CLI login")
        })?;

    audit::emit(
        state.db.clone(),
        everruns_core::DEFAULT_ORG_ID,
        Some(user.id),
        "auth.cli.callback",
        ip,
        serde_json::json!({}),
    );

    // Redirect browser to CLI's localhost server with the exchange code
    let redirect_url = format!(
        "http://localhost:{}/callback?code={}",
        session.redirect_port, session.exchange_code
    );

    Ok(Redirect::to(&redirect_url).into_response())
}

/// POST /v1/auth/cli/exchange — Exchange code for API key
///
/// CLI sends the exchange code received via localhost callback.
/// Server creates an API key and returns it with user/org info.
async fn cli_auth_exchange(
    State(state): State<CliAuthState>,
    headers: HeaderMap,
    Json(req): Json<CliExchangeRequest>,
) -> Result<(StatusCode, Json<CliExchangeResponse>), AuthError> {
    let ip = audit::client_ip(&headers);

    // Look up session by exchange code
    let session = state
        .db
        .get_cli_auth_session_by_exchange_code(&req.code)
        .await
        .map_err(|e| {
            tracing::error!("Failed to look up CLI auth session: {}", e);
            AuthError::unauthorized("Invalid exchange code")
        })?
        .ok_or_else(|| AuthError::unauthorized("Invalid or expired exchange code"))?;

    if !session.completed {
        return Err(AuthError::unauthorized(
            "Login not yet completed. Please complete login in your browser.",
        ));
    }

    let user_id = session
        .user_id
        .ok_or_else(|| AuthError::unauthorized("Login session has no associated user"))?;

    // Fetch user info
    let user = state
        .db
        .get_user(user_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch user: {}", e);
            AuthError::unauthorized("Failed to fetch user")
        })?
        .ok_or_else(|| AuthError::unauthorized("User not found"))?;

    // Fetch user's organizations (for response, not for key creation)
    let orgs = state
        .db
        .list_user_organizations(user_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch user organizations: {}", e);
            AuthError::unauthorized("Failed to fetch organizations")
        })?;

    // Generate API key with CLI metadata (user-scoped, no org)
    let generated = generate_api_key();
    let hostname = req.hostname.unwrap_or_else(|| "unknown".to_string());
    let os = req.os.unwrap_or_else(|| "unknown".to_string());

    let metadata = serde_json::json!({
        "source": "cli_login",
        "hostname": hostname,
        "os": os,
        "created_via": "everruns login",
        "ip": ip.clone().unwrap_or_else(|| "unknown".to_string()),
    });

    state
        .db
        .create_api_key(CreateApiKeyRow {
            user_id,
            name: format!("CLI login ({})", hostname),
            key_hash: generated.key_hash.clone(),
            key_prefix: generated.key_prefix.clone(),
            scopes: vec!["*".to_string()],
            expires_at: None,
            metadata: metadata.clone(),
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to create API key: {}", e);
            AuthError::unauthorized("Failed to create API key")
        })?;

    // Delete the used session (one-time use)
    let _ = state.db.delete_expired_cli_auth_sessions().await;

    let audit_org_id = orgs
        .first()
        .map(|o| o.org_id)
        .unwrap_or(everruns_core::DEFAULT_ORG_ID);
    audit::emit(
        state.db.clone(),
        audit_org_id,
        Some(user_id),
        "auth.cli.exchange",
        ip,
        serde_json::json!({"hostname": hostname, "os": os}),
    );

    let org_responses: Vec<OrgMembershipResponse> = orgs
        .iter()
        .map(|o| OrgMembershipResponse {
            public_id: o.public_id.clone(),
            name: o.name.clone(),
            role: o.role.clone(),
        })
        .collect();

    Ok((
        StatusCode::OK,
        Json(CliExchangeResponse {
            api_key: generated.key,
            user: CliUserInfo {
                id: user_id.to_string(),
                email: user.email,
                name: user.name,
            },
            orgs: org_responses,
        }),
    ))
}

/// GET /cli/login-success — Branded success page
///
/// Shown in the browser after the CLI receives the exchange code.
/// Static HTML — no auth required.
async fn cli_login_success() -> Html<&'static str> {
    Html(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>CLI Login Successful - Everruns</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            display: flex;
            align-items: center;
            justify-content: center;
            min-height: 100vh;
            margin: 0;
            background: #f8fafc;
            color: #1e293b;
        }
        .container {
            text-align: center;
            padding: 2rem;
            max-width: 400px;
        }
        .checkmark {
            font-size: 3rem;
            margin-bottom: 1rem;
        }
        h1 {
            font-size: 1.5rem;
            margin-bottom: 0.5rem;
        }
        p {
            color: #64748b;
            line-height: 1.6;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="checkmark">&#10003;</div>
        <h1>You're logged in!</h1>
        <p>Authentication successful. You can close this tab and return to your terminal.</p>
    </div>
</body>
</html>"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_random_hex_uniqueness() {
        let a = generate_random_hex();
        let b = generate_random_hex();
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn test_generate_random_hex_format() {
        let hex = generate_random_hex();
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_build_cli_callback_path_with_api_prefix() {
        // Standard production/local layout: base_url = frontend_url + "/api"
        let path = build_cli_callback_path(
            "https://app.example.com",
            "https://app.example.com/api",
            "abcd1234",
        );
        assert_eq!(path, "/api/v1/auth/cli/callback?state=abcd1234");
    }

    #[test]
    fn test_build_cli_callback_path_same_origin_no_prefix() {
        // Backend mounted at the frontend origin root
        let path = build_cli_callback_path(
            "https://app.example.com",
            "https://app.example.com",
            "abcd1234",
        );
        assert_eq!(path, "/v1/auth/cli/callback?state=abcd1234");
    }

    #[test]
    fn test_build_cli_callback_path_different_origins() {
        // Rare dev layout without a reverse proxy — no prefix we can safely
        // derive. Result is still a relative path (no host leak into return_to).
        let path =
            build_cli_callback_path("http://localhost:9300", "http://localhost:9301", "abcd1234");
        assert_eq!(path, "/v1/auth/cli/callback?state=abcd1234");
    }

    #[test]
    fn test_build_cli_callback_path_strip_lands_mid_segment() {
        // `frontend_url` is a string-prefix of `base_url` but not a path-prefix
        // (the strip would land mid-segment). We must fall back to no prefix
        // rather than emit a `return_to` like ".example.com/api/v1/...".
        let path =
            build_cli_callback_path("https://app", "https://app.example.com/api", "abcd1234");
        assert_eq!(path, "/v1/auth/cli/callback?state=abcd1234");
    }

    #[test]
    fn test_build_cli_callback_path_returns_relative() {
        // `return_to` must always be a relative path (no absolute URL)
        let path = build_cli_callback_path(
            "https://app.example.com",
            "https://app.example.com/api",
            "state123",
        );
        assert!(path.starts_with('/'), "path must be relative: {}", path);
        assert!(
            !path.starts_with("//"),
            "path must not be protocol-relative: {}",
            path
        );
        assert!(
            !path.contains("://"),
            "path must not contain scheme: {}",
            path
        );
    }
}

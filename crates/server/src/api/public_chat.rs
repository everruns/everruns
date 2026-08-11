// Public Chat channel — isolated, public-facing chat web app bound to one App.
//
// Design Decision: Public Chat reuses the AG-UI streaming core
// (`ag_ui::run_app_agent_stream`) and the shared App endpoint auth verifier,
// but mounts its own routes, its own rate-limiter namespace, and its own
// bot-mitigation gate (Cloudflare Turnstile). Session routing tags are scoped
// with a `public_chat:` prefix so a thread ID can never collide with AG-UI or
// another app/channel (TM-TENANT-009). See `knowledge/integrations/public-chat.md`.
//
// Design Decision: Anonymous visitors must pass a Turnstile challenge before a
// session is created or any turn runs. Visitors authenticated via the channel's
// `auth` config (e.g. Google sign-in) bypass the challenge.

use std::sync::LazyLock;

use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, Path, Request, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{AUTHORIZATION, COOKIE, SET_COOKIE},
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use everruns_platform::{
    App, AppEndpointAuthMode, AppEndpointAuthProviderConfig, AppStatus, PublicChatChannelConfig,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::api::ag_ui::{AgUiState, run_app_agent_stream};
use crate::api::app_endpoint_auth::{
    AppEndpointAuthError, AppEndpointAuthPrincipal, LegacyEndpointAuth,
};
use crate::api::common::ErrorResponse;
use crate::api::turnstile::{TurnstileOutcome, TurnstileVerifier};
use crate::auth::rate_limit::extract_client_ip_from_parts;
use crate::middleware::RequestId;

/// Headers a client may use to carry the Turnstile token.
const TURNSTILE_TOKEN_HEADERS: [&str; 2] = ["cf-turnstile-response", "x-everruns-turnstile-token"];
const PUBLIC_CHAT_TOKEN_HEADER: &str = "x-everruns-public-chat-token";
const ROUTING_TAG_PREFIX: &str = "public_chat";
const VISITOR_COOKIE: &str = "everruns_public_chat_visitor";

/// Deployment-level gate for the Public Chat feature. When the deployment flag
/// is off, every public endpoint behaves as if no chat exists (sanitized 404),
/// regardless of any channel rows. Org-level opt-in additionally gates channel
/// creation and the builder UI. Resolved once at startup from the env.
static PUBLIC_CHAT_ENABLED: LazyLock<bool> =
    LazyLock::new(|| everruns_platform::FeatureFlags::current().public_chat);

pub fn routes(state: AgUiState) -> Router {
    Router::new()
        .route("/v1/apps/{app_id}/public-chat", post(run_public_chat))
        .route("/v1/apps/{app_id}/public-chat/config", get(get_config))
        .with_state(state)
}

/// Non-secret bootstrap configuration the public web app needs to render the
/// chat: branding, whether anonymous access is allowed, the sign-in method (if
/// any), and the Turnstile site key (if enforced). No secrets are ever
/// returned — not the shared token, the Turnstile secret, or auth client
/// secrets.
#[derive(Debug, Serialize)]
struct PublicChatBootstrap {
    app_id: String,
    /// Display name (branding override or the App name).
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    logo_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    primary_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    welcome_message: Option<String>,
    anonymous: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    sign_in: Option<PublicChatSignIn>,
    #[serde(skip_serializing_if = "Option::is_none")]
    captcha: Option<PublicChatCaptchaBootstrap>,
}

#[derive(Debug, Serialize)]
struct PublicChatSignIn {
    /// Auth mode (e.g. `google_oidc`).
    mode: String,
    /// Public Google OAuth client ID, present for the `google_oidc` mode so the
    /// client can render the Google sign-in widget.
    #[serde(skip_serializing_if = "Option::is_none")]
    google_client_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct PublicChatCaptchaBootstrap {
    provider: String,
    site_key: String,
}

async fn get_config(
    State(state): State<AgUiState>,
    Path(app_id): Path<String>,
) -> Result<Json<PublicChatBootstrap>, Response> {
    let (app, config) = resolve_published_channel(&state, &app_id).await?;

    let branding = &config.branding;
    let name = branding
        .display_name
        .clone()
        .unwrap_or_else(|| app.name.clone());

    let sign_in = config
        .auth
        .as_ref()
        .filter(|auth| auth.mode != AppEndpointAuthMode::Anonymous)
        .map(|auth| {
            let google_client_id = match auth.provider.as_ref() {
                Some(AppEndpointAuthProviderConfig::GoogleOidc { client_id, .. }) => {
                    Some(client_id.clone())
                }
                _ => None,
            };
            PublicChatSignIn {
                mode: auth_mode_str(&auth.mode).to_string(),
                google_client_id,
            }
        });

    // Only advertise the captcha when it is actually enforced, and only the
    // public site key — never the secret.
    let captcha =
        config
            .captcha
            .as_ref()
            .filter(|c| c.enabled)
            .map(|c| PublicChatCaptchaBootstrap {
                provider: "turnstile".to_string(),
                site_key: c.site_key.clone(),
            });

    Ok(Json(PublicChatBootstrap {
        app_id: app.public_id.to_string(),
        name,
        logo_url: branding.logo_url.clone(),
        primary_color: branding.primary_color.clone(),
        welcome_message: branding.welcome_message.clone(),
        anonymous: config.anonymous,
        sign_in,
        captcha,
    }))
}

async fn run_public_chat(
    State(state): State<AgUiState>,
    Path(app_id): Path<String>,
    req_id: Option<Extension<RequestId>>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, Response> {
    let request_id = req_id.map(|Extension(r)| r.0);
    let peer_addr = connect_info.map(|Extension(ConnectInfo(addr))| addr);
    let (app, config) = resolve_published_channel(&state, &app_id).await?;

    // 1. Authentication. A channel with a real inline `auth` provider requires
    //    a verified credential (e.g. Google sign-in); those visitors are
    //    "signed in" and bypass the captcha. `auth.mode = anonymous` is not a
    //    real gate (the verifier always passes it), so it is treated exactly
    //    like the anonymous path — it must not silently bypass Turnstile or the
    //    `anonymous = false` lock.
    let real_auth = config
        .auth
        .as_ref()
        .filter(|auth| auth.mode != AppEndpointAuthMode::Anonymous);
    let (authenticated, visitor_binding, set_visitor_cookie) = if let Some(auth) = real_auth {
        let principal = state
            .auth_verifier
            .verify_principal(
                auth,
                &headers,
                LegacyEndpointAuth {
                    shared_secret: config.token.as_deref(),
                    api_key: None,
                },
            )
            .await
            .map_err(auth_error_response)?;
        let principal = principal.ok_or_else(unauthorized)?;
        (true, signed_in_visitor_tag(&principal), None)
    } else {
        if !config.anonymous {
            return Err(forbidden("Anonymous access is disabled for this chat"));
        }
        if let Some(expected) = config.token.as_deref()
            && !expected.is_empty()
        {
            let provided = extract_token(&headers).ok_or_else(unauthorized)?;
            if !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
                return Err(unauthorized());
            }
        }
        let (visitor_id, set_cookie) = anonymous_visitor_id(&headers);
        (false, anonymous_visitor_tag(&visitor_id), set_cookie)
    };

    // 2. Bot mitigation. Enforced only for anonymous (not-signed-in) visitors
    //    when a captcha is configured and enabled.
    if !authenticated && config.captcha_enforced() {
        verify_turnstile(&config, &headers, peer_addr).await?;
    }

    // 3. Per-app, per-IP rate limit (own namespace via the state's limiter).
    if let Some(limit) = config.rate_limit_per_minute
        && limit > 0
    {
        let client_ip = extract_client_ip_from_parts(peer_addr, &headers);
        if state
            .rate_limiter
            .check(&app.public_id.to_string(), client_ip, limit)
            .await
            .is_err()
        {
            return Err(too_many_requests("Rate limit exceeded for this chat"));
        }
    }

    // 4. Delegate to the shared AG-UI streaming core with a public_chat-scoped
    //    routing-tag prefix so sessions stay isolated from other channels.
    let sse = run_app_agent_stream(
        state,
        app,
        config.ag_ui_stream_config(),
        ROUTING_TAG_PREFIX,
        vec![visitor_binding],
        request,
        request_id,
    )
    .await?;
    let mut response = sse.into_response();
    if let Some(cookie) = set_visitor_cookie {
        response.headers_mut().append(SET_COOKIE, cookie);
    }
    Ok(response)
}

fn signed_in_visitor_tag(principal: &AppEndpointAuthPrincipal) -> String {
    visitor_tag(
        "signed",
        &[principal.issuer.as_str(), principal.subject.as_str()],
    )
}

fn anonymous_visitor_id(headers: &HeaderMap) -> (String, Option<HeaderValue>) {
    if let Some(existing) = cookie_value(headers, VISITOR_COOKIE).filter(is_valid_visitor_id) {
        return (existing.to_string(), None);
    }

    let visitor_id = Uuid::new_v4().to_string();
    let cookie = format!(
        "{VISITOR_COOKIE}={visitor_id}; Path=/; HttpOnly; SameSite=Lax; Secure; Max-Age=31536000"
    );
    let header = HeaderValue::from_str(&cookie).ok();
    (visitor_id, header)
}

fn anonymous_visitor_tag(visitor_id: &str) -> String {
    visitor_tag("anonymous", &[visitor_id])
}

fn visitor_tag(kind: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    for part in parts {
        hasher.update([0]);
        hasher.update(part.as_bytes());
    }
    format!(
        "{ROUTING_TAG_PREFIX}:visitor:{kind}:{}",
        hex::encode(hasher.finalize())
    )
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(cookie_name, value)| (cookie_name == name).then_some(value))
}

fn is_valid_visitor_id(value: &&str) -> bool {
    Uuid::parse_str(value).is_ok()
}

/// Verify the Turnstile challenge for an anonymous request. Maps the outcome to
/// a sanitized response: rejected → 403, service unavailable → 503.
async fn verify_turnstile(
    config: &PublicChatChannelConfig,
    headers: &HeaderMap,
    peer_addr: Option<std::net::SocketAddr>,
) -> Result<(), Response> {
    let captcha = config
        .captcha
        .as_ref()
        .ok_or_else(|| forbidden("Challenge required"))?;
    let secret = captcha
        .secret_key
        .as_deref()
        // A captcha enabled without a stored secret is a misconfiguration; fail
        // closed rather than letting traffic through unverified.
        .ok_or_else(|| service_unavailable("Challenge is temporarily unavailable"))?;
    let token = extract_turnstile_token(headers)
        .ok_or_else(|| forbidden("Challenge token missing or invalid"))?;
    let remote_ip = Some(extract_client_ip_from_parts(peer_addr, headers));

    match TurnstileVerifier::default()
        .verify(secret, &token, remote_ip)
        .await
    {
        TurnstileOutcome::Allowed => Ok(()),
        TurnstileOutcome::Rejected => Err(forbidden("Challenge token missing or invalid")),
        TurnstileOutcome::Unavailable => {
            Err(service_unavailable("Challenge is temporarily unavailable"))
        }
    }
}

/// Resolve a published App with an enabled Public Chat channel and its parsed
/// config. Errors are deliberately generic so a caller cannot distinguish "no
/// app" from "unpublished" from "channel disabled" (mirrors the FCP/AG-UI
/// public-surface contract).
async fn resolve_published_channel(
    state: &AgUiState,
    app_id: &str,
) -> Result<(App, PublicChatChannelConfig), Response> {
    // Feature-gated: when the deployment flag is off, the surface does not exist.
    if !*PUBLIC_CHAT_ENABLED {
        return Err(ErrorResponse::feature_not_enabled("public_chat").into_response());
    }
    let app = crate::domains::apps::queries::get_by_public_id_unscoped(
        &state.db,
        state.encryption.as_ref(),
        app_id,
    )
    .await
    .map_err(internal_error)?
    .ok_or_else(not_found)?;

    if app.status != AppStatus::Published {
        return Err(not_found());
    }

    let channel = app.public_chat_channel().ok_or_else(not_found)?;
    let config = channel.public_chat_config().ok_or_else(not_found)?;
    Ok((app, config))
}

fn auth_mode_str(mode: &AppEndpointAuthMode) -> &'static str {
    match mode {
        AppEndpointAuthMode::Anonymous => "anonymous",
        AppEndpointAuthMode::SharedSecret => "shared_secret",
        AppEndpointAuthMode::ApiKey => "api_key",
        AppEndpointAuthMode::GoogleOidc => "google_oidc",
        AppEndpointAuthMode::Oidc => "oidc",
        AppEndpointAuthMode::OAuth2Introspection => "oauth2_introspection",
        AppEndpointAuthMode::HttpBasic => "http_basic",
        AppEndpointAuthMode::Mtls => "mtls",
    }
}

fn extract_token(headers: &HeaderMap) -> Option<&str> {
    if let Some(value) = headers.get(PUBLIC_CHAT_TOKEN_HEADER) {
        return value.to_str().ok();
    }
    let auth = headers.get(AUTHORIZATION)?.to_str().ok()?;
    auth.strip_prefix("Bearer ")
}

fn extract_turnstile_token(headers: &HeaderMap) -> Option<String> {
    for name in TURNSTILE_TOKEN_HEADERS {
        if let Some(value) = headers.get(name)
            && let Ok(token) = value.to_str()
            && !token.trim().is_empty()
        {
            return Some(token.trim().to_string());
        }
    }
    None
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

fn auth_error_response(error: AppEndpointAuthError) -> Response {
    match error {
        AppEndpointAuthError::Unauthorized => unauthorized(),
        AppEndpointAuthError::Misconfigured => forbidden("Sign-in is misconfigured"),
        AppEndpointAuthError::ProviderUnavailable => {
            service_unavailable("Sign-in provider is unavailable")
        }
    }
}

fn internal_error(err: anyhow::Error) -> Response {
    tracing::error!(error = %err, "Public Chat route failed");
    ErrorResponse::new("Internal server error".to_string())
        .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        .into_response()
}

fn not_found() -> Response {
    ErrorResponse::new("Chat not found".to_string())
        .into_response(StatusCode::NOT_FOUND)
        .into_response()
}

fn forbidden(message: &str) -> Response {
    ErrorResponse::new(message.to_string())
        .into_response(StatusCode::FORBIDDEN)
        .into_response()
}

fn unauthorized() -> Response {
    ErrorResponse::new("Invalid or missing credentials".to_string())
        .into_response(StatusCode::UNAUTHORIZED)
        .into_response()
}

fn service_unavailable(message: &str) -> Response {
    ErrorResponse::new(message.to_string())
        .into_response(StatusCode::SERVICE_UNAVAILABLE)
        .into_response()
}

fn too_many_requests(message: &str) -> Response {
    let (status, json) =
        ErrorResponse::new(message.to_string()).into_response(StatusCode::TOO_MANY_REQUESTS);
    (status, [("retry-after", "60")], json).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn extract_turnstile_token_prefers_cf_header() {
        let mut headers = HeaderMap::new();
        headers.insert("cf-turnstile-response", HeaderValue::from_static("tok-123"));
        assert_eq!(
            extract_turnstile_token(&headers).as_deref(),
            Some("tok-123")
        );
    }

    #[test]
    fn extract_turnstile_token_accepts_everruns_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-everruns-turnstile-token",
            HeaderValue::from_static("tok-xyz"),
        );
        assert_eq!(
            extract_turnstile_token(&headers).as_deref(),
            Some("tok-xyz")
        );
    }

    #[test]
    fn extract_turnstile_token_none_when_absent_or_blank() {
        assert!(extract_turnstile_token(&HeaderMap::new()).is_none());
        let mut headers = HeaderMap::new();
        headers.insert("cf-turnstile-response", HeaderValue::from_static("   "));
        assert!(extract_turnstile_token(&headers).is_none());
    }

    #[test]
    fn extract_token_supports_bearer_and_custom_header() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer abc"));
        assert_eq!(extract_token(&headers), Some("abc"));

        let mut headers = HeaderMap::new();
        headers.insert(
            PUBLIC_CHAT_TOKEN_HEADER,
            HeaderValue::from_static("custom-tok"),
        );
        assert_eq!(extract_token(&headers), Some("custom-tok"));
    }

    #[test]
    fn constant_time_eq_matches_and_rejects() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secextra"));
        assert!(!constant_time_eq(b"secret", b"other"));
    }

    #[test]
    fn auth_mode_str_roundtrips_known_modes() {
        assert_eq!(
            auth_mode_str(&AppEndpointAuthMode::GoogleOidc),
            "google_oidc"
        );
        assert_eq!(auth_mode_str(&AppEndpointAuthMode::Anonymous), "anonymous");
    }

    #[test]
    fn signed_in_visitor_tags_are_stable_and_subject_scoped() {
        let alice = AppEndpointAuthPrincipal {
            issuer: "https://accounts.google.com".to_string(),
            subject: "alice".to_string(),
        };
        let alice_again = AppEndpointAuthPrincipal {
            issuer: "https://accounts.google.com".to_string(),
            subject: "alice".to_string(),
        };
        let bob = AppEndpointAuthPrincipal {
            issuer: "https://accounts.google.com".to_string(),
            subject: "bob".to_string(),
        };

        assert_eq!(
            signed_in_visitor_tag(&alice),
            signed_in_visitor_tag(&alice_again)
        );
        assert_ne!(signed_in_visitor_tag(&alice), signed_in_visitor_tag(&bob));
        assert!(signed_in_visitor_tag(&alice).starts_with("public_chat:visitor:signed:"));
        assert!(!signed_in_visitor_tag(&alice).contains("alice"));
    }

    #[test]
    fn anonymous_visitor_uses_existing_cookie_or_sets_http_only_cookie() {
        let visitor_id = Uuid::new_v4().to_string();
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&format!("other=1; {VISITOR_COOKIE}={visitor_id}")).unwrap(),
        );

        assert_eq!(anonymous_visitor_id(&headers), (visitor_id.clone(), None));

        let (new_id, set_cookie) = anonymous_visitor_id(&HeaderMap::new());
        assert!(Uuid::parse_str(&new_id).is_ok());
        let set_cookie = set_cookie.unwrap().to_str().unwrap().to_string();
        assert!(set_cookie.starts_with(&format!("{VISITOR_COOKIE}={new_id};")));
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("SameSite=Lax"));
    }
}

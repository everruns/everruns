//! OAuth provider connection (e.g. "Connect with OpenRouter").
//!
//! Split out of `providers.rs` to keep that file under the size guard.

use super::AppState;
use crate::auth::AuthUser;
use crate::auth::ResolvedOrg;
use crate::auth::config::AuthMode;
use crate::domains::common::Command;
use crate::domains::providers::record::Provider;
use crate::domains::providers::{LLM_PROVIDER_MANAGE, UpdateProvider};
use crate::kernel_imports::{
    Caller, Policy, contracts::driver_registry::DriverOAuthFlow, contracts::provider::DriverId,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Redirect,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use everruns_contracts::typed_id::ProviderId;
use everruns_contracts::url_validation::validate_safe_url;
use hmac::{Hmac, KeyInit, Mac};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
type HmacSha256 = Hmac<Sha256>;

// ============================================================================
// OAuth provider connection (e.g. "Connect with OpenRouter")
//
// Some drivers declare an interactive OAuth flow (DriverDescriptor::oauth) so an
// admin can connect a provider by authorizing in the browser instead of pasting
// an API key. The flow yields a long-lived credential that is stored in the same
// encrypted credentials field a hand-typed key would use, so model resolution is
// unchanged and non-admin users are unaffected. The pattern is driver-agnostic:
// adding OAuth to another driver only requires a DriverOAuthConfig on its
// descriptor, not new endpoints.
// ============================================================================

/// HttpOnly, browser-bound state persisted across the OAuth round-trip. Binds
/// the callback to the provider, org, admin, and the PKCE verifier this browser
/// generated, so a forged callback cannot inject an attacker's credential.
///
/// Decision: the callback is authenticated by this state, not by the session
/// cookie. The provider's consent page can outlive the 15-minute access-token
/// cookie (signing in or signing up at OpenRouter takes a while), and only XHRs
/// refresh that cookie, so a top-level redirect back routinely arrived with no
/// session and failed with a bare 401. The state is HMAC-signed with the JWT
/// secret, expires server-side, and names the admin who started the flow; the
/// callback reloads that admin's membership and re-checks `provider.manage`.
///
// THREAT[TM-API-021]: this binding plus the `provider.manage` re-check in the
// callback is the CSRF/forgery mitigation; do not relax it without updating
// `knowledge/security/threat-model.md`.
#[derive(Debug, Serialize, Deserialize)]
struct PendingProviderOAuth {
    /// CSRF token echoed via the callback URL and matched here.
    state: String,
    /// Prefixed public provider id this flow targets.
    provider_id: String,
    /// Org the provider belongs to.
    org_id: i64,
    /// Admin who started the flow; the callback acts as them. `None` only for
    /// the anonymous no-auth (local development) user.
    user_id: Option<uuid::Uuid>,
    /// Unix seconds after which the state is refused, whatever the browser kept.
    expires_at: i64,
    /// PKCE code verifier exchanged for the credential.
    code_verifier: String,
}

/// How long a started connect flow stays valid (state cookie and payload).
const PROVIDER_OAUTH_STATE_TTL_MINUTES: i64 = 10;

#[derive(Debug, Deserialize)]
pub struct ProviderOAuthCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    /// Set by the provider when the user declines consent.
    pub error: Option<String>,
}

/// OpenRouter's token-exchange response: the `key` is a user-controlled API key.
#[derive(Debug, Deserialize)]
struct OpenRouterKeyResponse {
    key: String,
}

/// GET /v1/providers/{id}/oauth/authorize — begin the driver's OAuth connect flow.
///
/// Looks up the provider's driver, and if it declares an OAuth flow, redirects
/// the admin's browser to the provider's authorization endpoint with PKCE.
pub(super) async fn oauth_authorize(
    org: ResolvedOrg,
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    let caller = require_provider_manage(&state, &org)?;
    let (provider, oauth) = resolve_oauth_provider(&state, &caller, &id).await?;
    // Fail fast: the callback can only store the obtained key when encryption is
    // configured. Surface the same message here so an admin does not complete an
    // OAuth consent only to hit a 500 on the callback.
    require_encryption_configured(&state)?;
    // Defense-in-depth: the authorize URL is driver-declared, but validate it so
    // a future misconfigured driver cannot redirect into the internal network.
    validate_safe_url(&oauth.authorize_url).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Authorization endpoint blocked: {e}"),
        )
    })?;

    let csrf_state = random_hex();
    let code_verifier = generate_pkce_verifier();
    let code_challenge = pkce_challenge(&code_verifier);

    let pending = PendingProviderOAuth {
        state: csrf_state.clone(),
        provider_id: id.clone(),
        org_id: org.org_id,
        user_id: (state.auth.config.mode != AuthMode::None)
            .then_some(caller.user_id)
            .flatten(),
        expires_at: chrono::Utc::now().timestamp() + PROVIDER_OAUTH_STATE_TTL_MINUTES * 60,
        code_verifier,
    };
    let cookie = Cookie::build((
        oauth_state_cookie_name(&id),
        encode_pending_oauth(&pending, &state.auth.config.jwt.secret)
            .map_err(|e| internal("OAuth provider connection", &e))?,
    ))
    .path("/")
    .http_only(true)
    .secure(true)
    .same_site(SameSite::Lax)
    .max_age(time::Duration::minutes(PROVIDER_OAUTH_STATE_TTL_MINUTES))
    .build();

    // The CSRF token rides in the callback URL so it round-trips regardless of
    // whether the provider echoes a `state` parameter of its own.
    let callback_url = format!(
        "{}/v1/providers/{}/oauth/callback?state={}",
        state.auth.config.base_url.trim_end_matches('/'),
        urlencoding::encode(&id),
        urlencoding::encode(&csrf_state),
    );

    let redirect_url = match oauth.flow {
        DriverOAuthFlow::ChatGptPlan => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Use the ChatGPT connection endpoint.".to_string(),
            ));
        }
        DriverOAuthFlow::OpenRouterPkce => openrouter_authorize_url(
            &oauth.authorize_url,
            &callback_url,
            &code_challenge,
            &org.name,
        )
        .map_err(|e| internal("OAuth provider connection", &e))?,
    };

    tracing::info!(
        org_id = org.org_id,
        provider = %provider.provider_type,
        "Starting OAuth provider connection"
    );
    Ok((jar.add(cookie), Redirect::to(&redirect_url)))
}

/// GET /v1/providers/{id}/oauth/callback — finish the OAuth connect flow.
///
/// Reached by the provider's browser redirect, so it carries no session of its
/// own: the signed state cookie set by `oauth_authorize` authenticates it (see
/// [`PendingProviderOAuth`]). Validates that state, exchanges the authorization
/// code for a credential, stores it on the provider (encrypted at rest), and
/// always returns the browser to the providers page: `?connected=<driver>` on
/// success, `?connect_error=<code>` otherwise.
pub(super) async fn oauth_callback(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Query(query): Query<ProviderOAuthCallbackQuery>,
) -> (CookieJar, Redirect) {
    // A `__Host-`-prefixed cookie is only deleted if the removal Set-Cookie also
    // carries Secure and Path=/, so build the removal cookie with those attributes
    // rather than relying on a bare name (which browsers would reject and ignore).
    let clear_cookie = jar.clone().remove(
        Cookie::build(oauth_state_cookie_name(&id))
            .path("/")
            .secure(true)
            .http_only(true)
            .build(),
    );
    let providers_page = format!(
        "{}/models?tab=providers",
        state.auth.config.frontend_url.trim_end_matches('/'),
    );
    match finish_oauth_callback(&state, &jar, &id, query).await {
        Ok(provider_type) => (
            clear_cookie,
            Redirect::to(&format!(
                "{providers_page}&connected={}",
                urlencoding::encode(provider_type.as_str()),
            )),
        ),
        Err(failure) => {
            tracing::warn!(
                provider_id = %id,
                reason = failure.reason,
                "OAuth provider connection failed"
            );
            (
                clear_cookie,
                Redirect::to(&format!("{providers_page}&connect_error={}", failure.code)),
            )
        }
    }
}

/// Why a provider OAuth callback failed: a stable `connect_error` code the UI
/// turns into a sentence, and a log-only reason.
#[derive(Debug)]
struct OAuthCallbackFailure {
    code: &'static str,
    reason: &'static str,
}

impl OAuthCallbackFailure {
    fn new(code: &'static str, reason: &'static str) -> Self {
        Self { code, reason }
    }
}

async fn finish_oauth_callback(
    state: &AppState,
    jar: &CookieJar,
    id: &str,
    query: ProviderOAuthCallbackQuery,
) -> Result<DriverId, OAuthCallbackFailure> {
    let pending = validate_pending_oauth(
        jar,
        id,
        query.state.as_deref(),
        &state.auth.config.jwt.secret,
        chrono::Utc::now().timestamp(),
    )
    .map_err(|_| OAuthCallbackFailure::new("expired", "invalid or expired OAuth state"))?;
    if query.error.is_some() {
        return Err(OAuthCallbackFailure::new(
            "provider_refused",
            "provider returned an error",
        ));
    }
    let code = query
        .code
        .filter(|code| !code.is_empty())
        .ok_or_else(|| OAuthCallbackFailure::new("provider_refused", "no authorization code"))?;

    let caller = oauth_callback_caller(state, &pending).await?;
    Policy::evaluate_with(
        &LLM_PROVIDER_MANAGE,
        state.auth.permission_resolver.as_ref(),
        &caller,
    )
    .map_err(|_| OAuthCallbackFailure::new("failed", "caller can no longer manage providers"))?;

    let (provider, oauth) = resolve_oauth_provider(state, &caller, id)
        .await
        .map_err(|_| OAuthCallbackFailure::new("failed", "provider or OAuth flow not found"))?;
    // Storing the obtained key requires encryption.
    require_encryption_configured(state)
        .map_err(|_| OAuthCallbackFailure::new("failed", "encryption not configured"))?;

    let api_key = match oauth.flow {
        DriverOAuthFlow::ChatGptPlan => {
            return Err(OAuthCallbackFailure::new(
                "failed",
                "ChatGPT uses its own connection endpoint",
            ));
        }
        DriverOAuthFlow::OpenRouterPkce => {
            exchange_openrouter_code(&oauth.token_url, &code, &pending.code_verifier)
                .await
                .map_err(|_| {
                    OAuthCallbackFailure::new("provider_refused", "token exchange failed")
                })?
        }
    };

    // Use the same credential update as the pasted-key flow, including its
    // bounded, best-effort model sync and bootstrap.
    UpdateProvider {
        id: provider.id.to_string(),
        api: None,
        name: None,
        provider_type: None,
        base_url: None,
        api_key: Some(api_key),
        status: None,
        trace: None,
        request_options: None,
    }
    .run(&state.ctx_for_caller(caller))
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to store OAuth provider credential");
        OAuthCallbackFailure::new("failed", "could not store credential")
    })?;

    tracing::info!(
        org_id = pending.org_id,
        provider = %provider.provider_type,
        "Stored credential from OAuth provider connection"
    );
    Ok(provider.provider_type)
}

/// Rebuild the caller who started the flow from the signed state: the admin's
/// current membership and role (so a removal or demotion since the start is
/// honored), or the anonymous owner in no-auth local development.
async fn oauth_callback_caller(
    state: &AppState,
    pending: &PendingProviderOAuth,
) -> Result<Caller, OAuthCallbackFailure> {
    if state.auth.config.mode == AuthMode::None {
        let org = state
            .db
            .get_organization(pending.org_id)
            .await
            .ok()
            .flatten()
            .ok_or_else(|| OAuthCallbackFailure::new("failed", "organization not found"))?;
        let anonymous = AuthUser::anonymous();
        return Ok(Caller {
            org_id: org.org_id,
            org_public_id: org.public_id,
            user_id: Some(anonymous.id),
            role: everruns_core::OrgRole::Owner,
            is_platform_user: anonymous.is_platform_user,
            is_internal: false,
            read_only: false,
        });
    }
    let user_id = pending
        .user_id
        .ok_or_else(|| OAuthCallbackFailure::new("expired", "state names no user"))?;
    crate::auth::caller_resolution::caller_for_user(&state.db, pending.org_id, user_id)
        .await
        .map_err(|_| OAuthCallbackFailure::new("failed", "user is no longer an org member"))
}

/// Resolve the provider and its driver's declared OAuth flow, or a client error
/// if the provider is missing or its driver does not support OAuth.
async fn resolve_oauth_provider(
    state: &AppState,
    caller: &Caller,
    id: &str,
) -> Result<
    (
        Provider,
        everruns_contracts::driver_registry::DriverOAuthConfig,
    ),
    (StatusCode, String),
> {
    let provider_id = id
        .parse::<ProviderId>()
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid provider ID: {e}")))?;
    let provider = state
        .service
        .get(caller, provider_id.uuid())
        .await
        .map_err(|e| internal("OAuth provider connection", &e))?
        .ok_or((StatusCode::NOT_FOUND, "Provider not found".to_string()))?;
    let oauth = state
        .driver_registry
        .descriptor(&provider.provider_type)
        .and_then(|d| d.oauth.clone())
        .ok_or((
            StatusCode::BAD_REQUEST,
            format!(
                "Provider driver '{}' does not support OAuth connection",
                provider.provider_type
            ),
        ))?;
    Ok((provider, oauth))
}

/// OpenRouter authorize URL for the PKCE connect flow.
///
/// `key_label` prefills the consent screen and becomes the created key's name.
fn openrouter_authorize_url(
    authorize_url: &str,
    callback_url: &str,
    code_challenge: &str,
    org_name: &str,
) -> Result<String, url::ParseError> {
    let mut url = reqwest::Url::parse(authorize_url)?;
    url.query_pairs_mut()
        .append_pair("callback_url", callback_url)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("key_label", &openrouter_key_label(org_name));
    Ok(url.to_string())
}

/// OpenRouter's authorize page prefills the created key's label from `key_label`.
/// The API caps that field at 100 characters.
const OPENROUTER_KEY_LABEL_LIMIT: usize = 100;

/// Label shown on OpenRouter's consent screen and stored on the created key:
/// `Everruns <org name>`, so the key is identifiable in the OpenRouter dashboard.
fn openrouter_key_label(org_name: &str) -> String {
    let name = org_name.trim();
    let label = if name.is_empty() {
        "Everruns".to_string()
    } else {
        format!("Everruns {name}")
    };
    let mut truncated = String::new();
    for ch in label.chars().take(OPENROUTER_KEY_LABEL_LIMIT) {
        truncated.push(ch);
    }
    truncated
}

/// Exchange an OpenRouter PKCE authorization code for a user-controlled API key.
async fn exchange_openrouter_code(
    token_url: &str,
    code: &str,
    code_verifier: &str,
) -> Result<String, (StatusCode, String)> {
    // token_url is driver-declared (openrouter.ai), not user input; validate
    // anyway as defense-in-depth against future misconfiguration.
    validate_safe_url(token_url).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Token endpoint blocked: {e}"),
        )
    })?;
    let response = reqwest::Client::new()
        .post(token_url)
        .json(&serde_json::json!({
            "code": code,
            "code_verifier": code_verifier,
            "code_challenge_method": "S256",
        }))
        .send()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "OAuth token exchange request failed");
            (StatusCode::BAD_GATEWAY, "Token exchange failed".to_string())
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::error!(%status, body_len = body.len(), "OAuth token exchange returned error");
        return Err((StatusCode::BAD_GATEWAY, "Token exchange failed".to_string()));
    }
    let parsed: OpenRouterKeyResponse = response.json().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to parse OAuth token exchange response");
        (StatusCode::BAD_GATEWAY, "Token exchange failed".to_string())
    })?;
    Ok(parsed.key)
}

/// Require server-side encryption to be configured. The OAuth flow always ends
/// by storing a credential, so without encryption it cannot succeed — surface
/// the same message `ProviderService` uses, before the external round-trip.
fn require_encryption_configured(state: &AppState) -> Result<(), (StatusCode, String)> {
    if state.encryption.is_none() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured. Cannot store API key.".to_string(),
        ));
    }
    Ok(())
}

/// Require the caller to hold the provider-manage permission for this org.
fn require_provider_manage(
    state: &AppState,
    org: &ResolvedOrg,
) -> Result<Caller, (StatusCode, String)> {
    let caller = Caller::from(org);
    Policy::evaluate_with(
        &LLM_PROVIDER_MANAGE,
        state.auth.permission_resolver.as_ref(),
        &caller,
    )
    .map_err(|_| {
        (
            StatusCode::FORBIDDEN,
            "You do not have permission to manage providers".to_string(),
        )
    })?;
    Ok(caller)
}

fn validate_pending_oauth(
    jar: &CookieJar,
    provider_id: &str,
    query_state: Option<&str>,
    signing_secret: &str,
    now: i64,
) -> Result<PendingProviderOAuth, (StatusCode, String)> {
    let cookie = jar.get(&oauth_state_cookie_name(provider_id)).ok_or((
        StatusCode::BAD_REQUEST,
        "Invalid or expired OAuth state".to_string(),
    ))?;
    let pending = decode_pending_oauth(cookie.value(), signing_secret)?;
    let callback_state = query_state.ok_or((
        StatusCode::BAD_REQUEST,
        "Missing state parameter".to_string(),
    ))?;
    if pending.state != callback_state || pending.provider_id != provider_id {
        return Err((StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()));
    }
    if now > pending.expires_at {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid or expired OAuth state".to_string(),
        ));
    }
    Ok(pending)
}

fn encode_pending_oauth(
    pending: &PendingProviderOAuth,
    signing_secret: &str,
) -> Result<String, serde_json::Error> {
    let payload = serde_json::to_vec(pending)?;
    let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes())
        .expect("HMAC accepts signing keys of any length");
    mac.update(&payload);
    let signature = mac.finalize().into_bytes();
    Ok(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(payload),
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn decode_pending_oauth(
    value: &str,
    signing_secret: &str,
) -> Result<PendingProviderOAuth, (StatusCode, String)> {
    let (payload, signature) = value
        .split_once('.')
        .ok_or((StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()))?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()))?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()))?;
    let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes())
        .expect("HMAC accepts signing keys of any length");
    mac.update(&payload);
    mac.verify_slice(&signature)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()))?;
    serde_json::from_slice(&payload)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()))
}

fn oauth_state_cookie_name(provider_id: &str) -> String {
    format!(
        "__Host-provider_oauth_state_{}",
        provider_id.replace([':', '/'], "_")
    )
}

fn random_hex() -> String {
    let bytes: [u8; 16] = rand::rng().random();
    hex::encode(bytes)
}

fn generate_pkce_verifier() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn internal(context: &str, err: &impl std::fmt::Display) -> (StatusCode, String) {
    tracing::error!("{context} error: {err}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
    )
}

#[cfg(test)]
mod oauth_tests {
    use super::*;
    use crate::auth::AuthState;
    use crate::kernel_imports::contracts::driver_registry::DriverRegistry;
    use crate::storage::StorageBackend;
    use std::sync::Arc;

    const TEST_SIGNING_SECRET: &str = "test-oauth-cookie-signing-secret";
    const NOW: i64 = 1_800_000_000;

    fn jar_with_pending(pending: &PendingProviderOAuth) -> CookieJar {
        let value = encode_pending_oauth(pending, TEST_SIGNING_SECRET).unwrap();
        CookieJar::new().add(Cookie::new(
            oauth_state_cookie_name(&pending.provider_id),
            value,
        ))
    }

    fn sample_pending() -> PendingProviderOAuth {
        PendingProviderOAuth {
            state: "csrf-token".to_string(),
            provider_id: "provider_abc".to_string(),
            org_id: 42,
            user_id: Some(uuid::Uuid::nil()),
            expires_at: NOW + 600,
            code_verifier: "verifier".to_string(),
        }
    }

    #[test]
    fn pkce_challenge_is_base64url_sha256() {
        // RFC 7636 worked example.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            pkce_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn openrouter_authorize_url_prefills_the_org_key_label() {
        let url = openrouter_authorize_url(
            "https://openrouter.ai/auth",
            "https://app.everruns.com/v1/providers/provider_abc/oauth/callback?state=csrf",
            "challenge",
            "Acme & Co",
        )
        .unwrap();
        let parsed = reqwest::Url::parse(&url).unwrap();
        let query: std::collections::HashMap<String, String> =
            parsed.query_pairs().into_owned().collect();
        assert_eq!(
            query.get("key_label").map(String::as_str),
            Some("Everruns Acme & Co")
        );
        assert_eq!(
            query.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(parsed.host_str(), Some("openrouter.ai"));
    }

    #[test]
    fn openrouter_key_label_uses_the_org_name() {
        assert_eq!(openrouter_key_label("Acme"), "Everruns Acme");
        assert_eq!(openrouter_key_label("  Acme  "), "Everruns Acme");
        assert_eq!(openrouter_key_label("   "), "Everruns");
    }

    #[test]
    fn openrouter_key_label_stays_within_openrouter_limit() {
        let org = "界".repeat(200);
        let label = openrouter_key_label(&org);
        assert_eq!(label.chars().count(), OPENROUTER_KEY_LABEL_LIMIT);
        assert!(label.starts_with("Everruns 界"));
        assert!(label.is_char_boundary(label.len()));
    }

    #[test]
    fn cookie_name_sanitizes_separators() {
        assert_eq!(
            oauth_state_cookie_name("mcp:oauth/id"),
            "__Host-provider_oauth_state_mcp_oauth_id"
        );
    }

    #[test]
    fn validate_pending_accepts_matching_state() {
        let pending = sample_pending();
        let jar = jar_with_pending(&pending);
        let result = validate_pending_oauth(
            &jar,
            "provider_abc",
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
            NOW,
        )
        .unwrap();
        assert_eq!(result.code_verifier, "verifier");
    }

    #[test]
    fn validate_pending_rejects_state_mismatch() {
        let jar = jar_with_pending(&sample_pending());
        let err = validate_pending_oauth(
            &jar,
            "provider_abc",
            Some("wrong"),
            TEST_SIGNING_SECRET,
            NOW,
        )
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_rejects_expired_state() {
        // The browser may keep the cookie past its max-age; the signed payload's
        // own expiry still refuses it.
        let jar = jar_with_pending(&sample_pending());
        let err = validate_pending_oauth(
            &jar,
            "provider_abc",
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
            NOW + 601,
        )
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_rejects_provider_mismatch() {
        let jar = jar_with_pending(&sample_pending());
        // Cookie name is per-provider, so a different provider id finds no cookie.
        let err = validate_pending_oauth(
            &jar,
            "provider_other",
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
            NOW,
        )
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_requires_query_state() {
        let jar = jar_with_pending(&sample_pending());
        let err = validate_pending_oauth(&jar, "provider_abc", None, TEST_SIGNING_SECRET, NOW)
            .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_rejects_missing_cookie() {
        let err = validate_pending_oauth(
            &CookieJar::new(),
            "provider_abc",
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
            NOW,
        )
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_rejects_unsigned_cookie() {
        let pending = sample_pending();
        let forged_value = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&pending).unwrap());
        let jar = CookieJar::new().add(Cookie::new(
            oauth_state_cookie_name("provider_abc"),
            forged_value,
        ));
        let err = validate_pending_oauth(
            &jar,
            "provider_abc",
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
            NOW,
        )
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_rejects_tampered_signature() {
        let pending = sample_pending();
        let mut value = encode_pending_oauth(&pending, TEST_SIGNING_SECRET).unwrap();
        value.push('A');
        let jar = CookieJar::new().add(Cookie::new(oauth_state_cookie_name("provider_abc"), value));
        let err = validate_pending_oauth(
            &jar,
            "provider_abc",
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
            NOW,
        )
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    fn callback_state(mode: AuthMode) -> AppState {
        let db = Arc::new(StorageBackend::test_database());
        let config = crate::auth::AuthConfig {
            mode,
            jwt: crate::auth::config::JwtConfig {
                secret: TEST_SIGNING_SECRET.to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        AppState::new(
            db.clone(),
            None,
            Arc::new(DriverRegistry::new()),
            AuthState::builtin(config, db),
            None,
        )
    }

    fn live_pending() -> PendingProviderOAuth {
        PendingProviderOAuth {
            expires_at: chrono::Utc::now().timestamp() + 600,
            ..sample_pending()
        }
    }

    async fn callback_location(
        state: AppState,
        jar: CookieJar,
        query: ProviderOAuthCallbackQuery,
    ) -> String {
        use axum::response::IntoResponse;
        let response = oauth_callback(
            State(state),
            jar,
            Path("provider_abc".to_string()),
            Query(query),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        response.headers()[axum::http::header::LOCATION]
            .to_str()
            .unwrap()
            .to_string()
    }

    // Regression: the provider's redirect back carries no session cookie once
    // the 15-minute access token lapses during consent. The callback must not
    // answer with a bare 401 JSON page; it returns the browser to the providers
    // page with a reason the UI can show.
    #[tokio::test]
    async fn callback_without_session_or_state_returns_to_the_ui() {
        let location = callback_location(
            callback_state(AuthMode::Admin),
            CookieJar::new(),
            ProviderOAuthCallbackQuery {
                code: Some("code".into()),
                state: Some("csrf-token".into()),
                error: None,
            },
        )
        .await;
        assert_eq!(
            location,
            "http://localhost:9300/models?tab=providers&connect_error=expired"
        );
    }

    #[tokio::test]
    async fn callback_reports_declined_consent() {
        let location = callback_location(
            callback_state(AuthMode::Admin),
            jar_with_pending(&live_pending()),
            ProviderOAuthCallbackQuery {
                code: None,
                state: Some("csrf-token".into()),
                error: Some("access_denied".into()),
            },
        )
        .await;
        assert!(
            location.ends_with("connect_error=provider_refused"),
            "{location}"
        );
    }

    // With a valid signed state and no session, the callback acts as the admin
    // the state names, re-resolved from the database: a user who is no longer a
    // member (here: unknown) cannot finish the flow.
    #[tokio::test]
    async fn callback_rechecks_the_admin_named_by_the_state() {
        let location = callback_location(
            callback_state(AuthMode::Admin),
            jar_with_pending(&live_pending()),
            ProviderOAuthCallbackQuery {
                code: Some("code".into()),
                state: Some("csrf-token".into()),
                error: None,
            },
        )
        .await;
        assert!(location.ends_with("connect_error=failed"), "{location}");
    }

    #[tokio::test]
    async fn callback_refuses_expired_state_even_if_the_browser_kept_it() {
        let expired = PendingProviderOAuth {
            expires_at: chrono::Utc::now().timestamp() - 1,
            ..sample_pending()
        };
        let location = callback_location(
            callback_state(AuthMode::Admin),
            jar_with_pending(&expired),
            ProviderOAuthCallbackQuery {
                code: Some("code".into()),
                state: Some("csrf-token".into()),
                error: None,
            },
        )
        .await;
        assert!(location.ends_with("connect_error=expired"), "{location}");
    }

    // Trivial derive-only serde round-trips removed; covered by the derive + handler tests.
}

// Trivial ErrorResponse::new() + serialize round-trips removed; covered by
// crates/server/src/api/common.rs::tests::test_error_response_into_response.

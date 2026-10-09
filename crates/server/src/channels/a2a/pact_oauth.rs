// PACT Delegated profile (PACT 1.0 §5): the OAuth 2.0 authorization server a
// PACT endpoint runs so a personal agent can act on a user's account with the
// company behind the endpoint. RFC 8628 device code, under
// `/v1/a2a/{channel_id}/oauth/`.
//
// Flow: the personal agent asks for scopes (`device_authorization`) and shows
// the user a link to the company's own login page. After signing the user in,
// the company's site POSTs a signed, single-use assertion to `consent`; the
// user ticks the scopes to grant and submits `consent/decision`; the personal
// agent then redeems its device code at `token` for a delegation token.
//
// Design Decisions:
// - Routing before authentication, as for the rest of PACT: an endpoint
//   without `pact.delegation` is a plain `404` on every route here.
// - The OAuth client is the personal agent, authenticated with its §3 JWT on
//   both client endpoints; `client_id` must equal that JWT's issuer, so one
//   personal agent can never poll or refresh another's codes.
// - The company never shares a credential with Everruns. Its login proves the
//   user with an assertion it signs (verified against its JWKS, bound to this
//   consent URL and to the request's `user_code`, five minutes, `jti` single
//   use), and the consent page carries that proof forward as a short-lived
//   session token signed with the endpoint's own key (`pact_keys.rs`).
// - Device codes and refresh tokens are stored only as hashes. Refresh tokens
//   rotate on every use; a used one is refused.
// - Consent is never skipped, even when an earlier grant already covers the
//   request (§5.3 permits skipping, it does not require it).
// See `knowledge/integrations/a2a-channel.md`.

use std::collections::HashSet;

use axum::{
    Router,
    body::Bytes,
    extract::{OriginalUri, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::pact::{self, Peer};
use super::pact_keys::ProviderKey;
use super::{ChannelA2aState, pact_identity};
use crate::api::channel_ingress::{IngressChannel, IngressContext};
use crate::api::mcp_endpoint::cards::escape_html;
use crate::records::pact_delegation::PactDelegationConfig;
use crate::storage::pact_delegation::{PactDeviceAuthorizationRow, PactGrantRow};

const BASE: &str = "/v1/a2a/{channel_id}/oauth";

pub(super) const DEVICE_CODE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const REFRESH_TOKEN_GRANT_TYPE: &str = "refresh_token";
/// `typ` of a delegation token (RFC 9068).
pub(super) const ACCESS_TOKEN_TYPE: &str = "at+jwt";
/// `typ` of the consent page's session token.
const CONSENT_SESSION_TYPE: &str = "pact-consent+jwt";

const DEVICE_CODE_TTL_SECS: i64 = 600;
const POLL_INTERVAL_SECS: i32 = 5;
/// §5.4: "Lifetime SHOULD be ≤ 1 h."
const ACCESS_TOKEN_TTL_SECS: i64 = 3600;
const GRANT_TTL_DAYS: i64 = 30;
const CONSENT_SESSION_TTL_SECS: i64 = 600;
/// Oldest company sign-in assertion accepted, by `iat`.
const ASSERTION_MAX_AGE_SECS: i64 = 300;
const CLOCK_SKEW_SECS: i64 = 30;
/// RFC 8628 §6.1: a short code from an alphabet without vowels or lookalikes.
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";

pub(super) fn routes(router: Router<ChannelA2aState>) -> Router<ChannelA2aState> {
    router
        .route(
            &format!("{BASE}/.well-known/oauth-authorization-server"),
            get(metadata),
        )
        .route(&format!("{BASE}/jwks.json"), get(jwks))
        .route(
            &format!("{BASE}/device_authorization"),
            post(device_authorization),
        )
        .route(&format!("{BASE}/token"), post(token))
        .route(&format!("{BASE}/consent"), post(consent))
        .route(&format!("{BASE}/consent/decision"), post(decision))
}

/// The endpoint's OAuth URLs, all derived from its interface URL.
pub(super) struct Urls {
    pub interface: String,
    pub issuer: String,
}

impl Urls {
    pub fn new(interface: &str) -> Self {
        Self {
            interface: interface.to_string(),
            issuer: format!("{interface}/oauth"),
        }
    }

    /// From a request under `/oauth/`.
    fn from_request(headers: &HeaderMap, path: &str) -> Self {
        let interface = path.rfind("/oauth/").map_or(path, |at| &path[..at]);
        Self::new(&super::agent_card::absolute_url(headers, interface))
    }

    fn at(&self, path: &str) -> String {
        format!("{}/{path}", self.issuer)
    }

    pub fn metadata(&self) -> String {
        self.at(".well-known/oauth-authorization-server")
    }
    pub fn jwks(&self) -> String {
        self.at("jwks.json")
    }
    pub fn device_authorization(&self) -> String {
        self.at("device_authorization")
    }
    pub fn token(&self) -> String {
        self.at("token")
    }
    pub fn consent(&self) -> String {
        self.at("consent")
    }
    pub fn decision(&self) -> String {
        self.at("consent/decision")
    }
}

/// The card's `userDelegation` scheme (§5.1).
pub(super) fn security_scheme(interface_url: &str, delegation: &PactDelegationConfig) -> Value {
    let urls = Urls::new(interface_url);
    let scopes: serde_json::Map<String, Value> = delegation
        .scopes
        .iter()
        .map(|scope| (scope.id.clone(), Value::String(scope.description.clone())))
        .collect();
    json!({
        "oauth2SecurityScheme": {
            "flows": {
                "deviceCode": {
                    "deviceAuthorizationUrl": urls.device_authorization(),
                    "tokenUrl": urls.token(),
                    "scopes": scopes,
                }
            },
            "oauth2MetadataUrl": urls.metadata(),
        }
    })
}

/// The card as JSON text with the `userDelegation` scopes in config order.
///
/// Design Decision: `serde_json::Value` objects sort their keys, but the
/// scopes are a list the company ordered (PACT's suite and consent page read
/// them in that order), so that one object is written by hand.
pub(super) fn card_json(mut card: Value, delegation: Option<&PactDelegationConfig>) -> String {
    const PLACEHOLDER: &str = "__pact_scopes__";
    let Some(delegation) = delegation else {
        return card.to_string();
    };
    let Some(scopes) = card.pointer_mut(
        "/securitySchemes/userDelegation/oauth2SecurityScheme/flows/deviceCode/scopes",
    ) else {
        return card.to_string();
    };
    *scopes = Value::String(PLACEHOLDER.into());
    let ordered = delegation
        .scopes
        .iter()
        .map(|scope| format!("{}:{}", json!(scope.id), json!(scope.description)))
        .collect::<Vec<_>>()
        .join(",");
    card.to_string()
        .replacen(&format!("\"{PLACEHOLDER}\""), &format!("{{{ordered}}}"), 1)
}

/// A delegation-enabled endpoint, before any authentication.
struct Endpoint {
    app: IngressContext,
    channel: IngressChannel,
    config: crate::records::A2aChannelConfig,
    pact: crate::records::agent_channel::PactProfileConfig,
    delegation: PactDelegationConfig,
}

impl Endpoint {
    async fn key(&self, state: &ChannelA2aState) -> anyhow::Result<ProviderKey> {
        ProviderKey::for_channel(
            &state.db,
            state.encryption.as_ref(),
            self.channel.internal_id,
        )
        .await
    }

    fn brand_name(&self) -> &str {
        self.delegation
            .brand_name
            .as_deref()
            .unwrap_or(&self.app.name)
    }
}

async fn endpoint(state: &ChannelA2aState, channel_id: &str) -> Result<Endpoint, Response> {
    match pact::pact_channel(state, channel_id).await? {
        Some((app, channel, config, pact)) => match pact.delegation.clone() {
            Some(delegation) => Ok(Endpoint {
                app,
                channel,
                config,
                pact,
                delegation,
            }),
            None => Err(super::not_found().into_response()),
        },
        None => Err(super::not_found().into_response()),
    }
}

/// Resolve the endpoint, then authenticate the personal agent (the OAuth
/// client) and check that `client_id` names it. `Err` is the finished
/// response.
async fn admit_client(
    state: &ChannelA2aState,
    channel_id: &str,
    headers: &HeaderMap,
    peer: Peer,
    form: &Form,
) -> Result<(Endpoint, pact_identity::PersonalAgentUser), Response> {
    let endpoint = endpoint(state, channel_id).await?;
    let user = pact_identity::verify(&state.auth_verifier, &endpoint.pact, headers)
        .await
        .map_err(|()| pact::unauthorized())?;
    pact::rate_limit(
        state,
        &endpoint.app,
        &endpoint.channel,
        &endpoint.config,
        headers,
        peer,
    )
    .await?;
    if form.one("client_id") != Some(user.issuer.as_str()) {
        return Err(oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "client_id must equal the personal agent's issuer",
        ));
    }
    Ok((endpoint, user))
}

/// A parsed `application/x-www-form-urlencoded` body.
struct Form(Vec<(String, String)>);

impl Form {
    fn parse(body: &[u8]) -> Self {
        Self(url::form_urlencoded::parse(body).into_owned().collect())
    }
    fn one(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
    fn all(&self, name: &str) -> impl Iterator<Item = &str> {
        self.0
            .iter()
            .filter(move |(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Space-separated scope ids, de-duplicated in order.
pub(super) fn parse_scope(scope: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    scope
        .split(' ')
        .filter(|id| !id.is_empty() && seen.insert(*id))
        .map(str::to_owned)
        .collect()
}

fn no_store(status: StatusCode, body: &Value) -> Response {
    (
        status,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (header::PRAGMA, HeaderValue::from_static("no-cache")),
        ],
        body.to_string(),
    )
        .into_response()
}

/// RFC 6749 §5.2 error.
fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    no_store(
        status,
        &json!({ "error": error, "error_description": description }),
    )
}

fn internal(err: anyhow::Error) -> Response {
    tracing::error!(error = %err, "PACT OAuth request failed");
    oauth_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "server_error",
        "Internal error",
    )
}

fn sha256(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}

fn random_token(prefix: &str) -> String {
    use base64::Engine as _;
    let bytes: [u8; 32] = rand::rng().random();
    format!(
        "{prefix}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

fn user_code() -> String {
    let mut rng = rand::rng();
    let chars: Vec<char> = (0..8)
        .map(|_| USER_CODE_ALPHABET[rng.random_range(0..USER_CODE_ALPHABET.len())] as char)
        .collect();
    format!(
        "{}-{}",
        chars[..4].iter().collect::<String>(),
        chars[4..].iter().collect::<String>()
    )
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

async fn metadata(
    State(state): State<ChannelA2aState>,
    Path(channel_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    let endpoint = match endpoint(&state, &channel_id).await {
        Ok(endpoint) => endpoint,
        Err(response) => return response,
    };
    let urls = Urls::from_request(&headers, uri.path());
    let scopes: Vec<&str> = endpoint
        .delegation
        .scopes
        .iter()
        .map(|scope| scope.id.as_str())
        .collect();
    (
        [(header::CONTENT_TYPE, "application/json")],
        json!({
            "issuer": urls.issuer,
            "device_authorization_endpoint": urls.device_authorization(),
            "token_endpoint": urls.token(),
            "jwks_uri": urls.jwks(),
            "scopes_supported": scopes,
            "grant_types_supported": [DEVICE_CODE_GRANT_TYPE, REFRESH_TOKEN_GRANT_TYPE],
            "token_endpoint_auth_methods_supported": ["private_key_jwt"],
        })
        .to_string(),
    )
        .into_response()
}

async fn jwks(State(state): State<ChannelA2aState>, Path(channel_id): Path<String>) -> Response {
    let endpoint = match endpoint(&state, &channel_id).await {
        Ok(endpoint) => endpoint,
        Err(response) => return response,
    };
    match endpoint.key(&state).await {
        Ok(key) => (
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CACHE_CONTROL, "public, max-age=300"),
            ],
            key.jwks().to_string(),
        )
            .into_response(),
        Err(err) => internal(err),
    }
}

// ---------------------------------------------------------------------------
// Client endpoints (the personal agent)
// ---------------------------------------------------------------------------

/// The company's login page, sent back to `return_to` afterwards.
fn login_link(login_url: &str, return_to: &str) -> String {
    match url::Url::parse(login_url) {
        Ok(mut url) => {
            url.query_pairs_mut().append_pair("return_to", return_to);
            url.to_string()
        }
        Err(_) => login_url.to_string(),
    }
}

/// Start a device-code request for `scopes`. Shared with step-up (§5.5),
/// which asks for the missing scopes on the caller's behalf.
pub(super) async fn start_device_authorization(
    state: &ChannelA2aState,
    channel_internal_id: uuid::Uuid,
    delegation: &PactDelegationConfig,
    urls: &Urls,
    client_id: &str,
    scopes: &[String],
) -> anyhow::Result<Value> {
    let device_code = random_token("dc_");
    let code = user_code();
    state
        .db
        .create_pact_device_authorization(
            channel_internal_id,
            &PactDeviceAuthorizationRow {
                device_code_hash: sha256(&device_code),
                client_id: client_id.to_string(),
                user_code: code.clone(),
                requested_scope: scopes.join(" "),
                status: "pending".into(),
                grant_id: None,
                interval_secs: POLL_INTERVAL_SECS,
                last_polled_at: None,
                expires_at: Utc::now() + chrono::Duration::seconds(DEVICE_CODE_TTL_SECS),
            },
        )
        .await?;
    let consent = urls.consent();
    let complete = login_link(
        &delegation.login_url,
        &format!("{consent}?user_code={code}"),
    );
    Ok(json!({
        "device_code": device_code,
        "user_code": code,
        "verification_uri": login_link(&delegation.login_url, &consent),
        "verification_uri_complete": complete,
        "expires_in": DEVICE_CODE_TTL_SECS,
        "interval": POLL_INTERVAL_SECS,
    }))
}

async fn device_authorization(
    State(state): State<ChannelA2aState>,
    Path(channel_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    peer: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let form = Form::parse(&body);
    let (endpoint, user) = match admit_client(&state, &channel_id, &headers, peer, &form).await {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };
    let scopes = parse_scope(form.one("scope").unwrap_or_default());
    if scopes.is_empty()
        || scopes
            .iter()
            .any(|scope| endpoint.delegation.scope(scope).is_none())
    {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            "Request scope ids listed on the Agent Card",
        );
    }
    let urls = Urls::from_request(&headers, uri.path());
    match start_device_authorization(
        &state,
        endpoint.channel.internal_id,
        &endpoint.delegation,
        &urls,
        &user.issuer,
        &scopes,
    )
    .await
    {
        Ok(body) => no_store(StatusCode::OK, &body),
        Err(err) => internal(err),
    }
}

/// Claims of a delegation token (§5.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct DelegationClaims {
    pub iss: String,
    pub aud: String,
    pub sub: String,
    pub client_id: String,
    pub scope: String,
    pub grant_id: String,
    pub iat: i64,
    pub exp: i64,
}

async fn issue_tokens(
    state: &ChannelA2aState,
    key: &ProviderKey,
    urls: &Urls,
    grant: &PactGrantRow,
) -> anyhow::Result<Response> {
    let now = Utc::now().timestamp();
    let access_token = key.sign(
        ACCESS_TOKEN_TYPE,
        &DelegationClaims {
            iss: urls.issuer.clone(),
            aud: urls.interface.clone(),
            sub: grant.brand_user_id.clone(),
            client_id: grant.client_id.clone(),
            scope: grant.scope.clone(),
            grant_id: grant.id.clone(),
            iat: now,
            exp: now + ACCESS_TOKEN_TTL_SECS,
        },
    )?;
    let refresh_token = random_token("rt_");
    state
        .db
        .insert_pact_refresh_token(&sha256(&refresh_token), &grant.id, grant.expires_at)
        .await?;
    Ok(no_store(
        StatusCode::OK,
        &json!({
            "token_type": "Bearer",
            "access_token": access_token,
            "refresh_token": refresh_token,
            "expires_in": ACCESS_TOKEN_TTL_SECS,
            "scope": grant.scope,
        }),
    ))
}

async fn token(
    State(state): State<ChannelA2aState>,
    Path(channel_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    peer: Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let form = Form::parse(&body);
    let (endpoint, user) = match admit_client(&state, &channel_id, &headers, peer, &form).await {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };
    let urls = Urls::from_request(&headers, uri.path());
    let channel = endpoint.channel.internal_id;
    let invalid_grant =
        |description| oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", description);
    let grant_id = match form.one("grant_type") {
        Some(DEVICE_CODE_GRANT_TYPE) => {
            let hash = sha256(form.one("device_code").unwrap_or_default());
            let row = match state
                .db
                .poll_pact_device_authorization(channel, &hash)
                .await
            {
                Ok(Some(row)) if row.client_id == user.issuer => row,
                Ok(_) => return invalid_grant("Unknown device_code"),
                Err(err) => return internal(err),
            };
            let now = Utc::now();
            if row.expires_at <= now {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "expired_token",
                    "The device code expired",
                );
            }
            match row.status.as_str() {
                "denied" => {
                    return oauth_error(
                        StatusCode::BAD_REQUEST,
                        "access_denied",
                        "The user denied access",
                    );
                }
                "pending" => {
                    let too_soon = row.last_polled_at.is_some_and(|last| {
                        now - last < chrono::Duration::seconds(i64::from(row.interval_secs))
                    });
                    return if too_soon {
                        oauth_error(StatusCode::BAD_REQUEST, "slow_down", "Poll less often")
                    } else {
                        oauth_error(
                            StatusCode::BAD_REQUEST,
                            "authorization_pending",
                            "Waiting for the user",
                        )
                    };
                }
                _ => {}
            }
            match state
                .db
                .consume_pact_device_authorization(channel, &hash)
                .await
            {
                Ok(Some(grant_id)) => grant_id,
                Ok(None) => return invalid_grant("device_code was used"),
                Err(err) => return internal(err),
            }
        }
        Some(REFRESH_TOKEN_GRANT_TYPE) => {
            let hash = sha256(form.one("refresh_token").unwrap_or_default());
            match state.db.use_pact_refresh_token(&hash).await {
                Ok(Some(grant_id)) => grant_id,
                Ok(None) => return invalid_grant("Unknown, used or expired refresh_token"),
                Err(err) => return internal(err),
            }
        }
        _ => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "unsupported_grant_type",
                "Unsupported grant_type",
            );
        }
    };
    // THREAT[TM-A2A-017]: a grant redeems only for the personal agent it was
    // made for, on the endpoint it was made on, while it is live.
    let grant = match state.db.pact_grant(channel, &grant_id).await {
        Ok(Some(grant)) if grant.client_id == user.issuer && grant.is_live(Utc::now()) => grant,
        Ok(_) => return invalid_grant("The grant is revoked or expired"),
        Err(err) => return internal(err),
    };
    let key = match endpoint.key(&state).await {
        Ok(key) => key,
        Err(err) => return internal(err),
    };
    issue_tokens(&state, &key, &urls, &grant)
        .await
        .unwrap_or_else(internal)
}

// ---------------------------------------------------------------------------
// User-facing pages (the company's login returns here)
// ---------------------------------------------------------------------------

/// The consent page's session: the signed-in user, carried from the company's
/// assertion to the decision form.
#[derive(Debug, Serialize, Deserialize)]
struct ConsentSession {
    aud: String,
    sub: String,
    user_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    iat: i64,
    exp: i64,
}

/// A company sign-in assertion, as the consent page needs it.
#[derive(Debug, Deserialize)]
struct Assertion {
    sub: String,
    jti: String,
    iat: i64,
    exp: i64,
    user_code: String,
    #[serde(default)]
    email: Option<String>,
}

/// Verify the company's sign-in assertion (§5.3): its key, `iss`, `aud`
/// (this consent page), age and lifetime. Every failure is `None`.
async fn verify_assertion(
    state: &ChannelA2aState,
    delegation: &PactDelegationConfig,
    consent_url: &str,
    token: &str,
) -> Option<Assertion> {
    let header = decode_header(token).ok()?;
    if !matches!(header.alg, Algorithm::ES256 | Algorithm::RS256) {
        return None;
    }
    let jwks = match (
        delegation.login_jwks.as_ref(),
        delegation.login_jwks_uri.as_deref(),
    ) {
        (Some(inline), _) => {
            std::sync::Arc::new(serde_json::from_value::<JwkSet>(inline.clone()).ok()?)
        }
        (None, Some(uri)) => state.auth_verifier.jwks(uri).await.ok()?,
        (None, None) => return None,
    };
    let jwk = match header.kid.as_deref() {
        Some(kid) => jwks
            .keys
            .iter()
            .find(|jwk| jwk.common.key_id.as_deref() == Some(kid)),
        None if jwks.keys.len() == 1 => jwks.keys.first(),
        None => None,
    }?;
    let mut validation = Validation::new(header.alg);
    validation.set_issuer(&[delegation.login_issuer.as_str()]);
    validation.set_audience(&[consent_url]);
    validation.set_required_spec_claims(&["exp", "iat", "iss", "aud", "sub", "jti"]);
    validation.leeway = CLOCK_SKEW_SECS as u64;
    let assertion = decode::<Assertion>(token, &DecodingKey::from_jwk(jwk).ok()?, &validation)
        .ok()?
        .claims;
    let now = Utc::now().timestamp();
    let fresh = assertion.iat <= now + CLOCK_SKEW_SECS
        && now - assertion.iat <= ASSERTION_MAX_AGE_SECS
        && assertion.exp - assertion.iat <= ASSERTION_MAX_AGE_SECS;
    (fresh && !assertion.sub.is_empty() && !assertion.jti.is_empty()).then_some(assertion)
}

async fn consent(
    State(state): State<ChannelA2aState>,
    Path(channel_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let endpoint = match endpoint(&state, &channel_id).await {
        Ok(endpoint) => endpoint,
        Err(response) => return response,
    };
    let brand = endpoint.brand_name().to_string();
    let urls = Urls::from_request(&headers, uri.path());
    let form = Form::parse(&body);
    // THREAT[TM-A2A-017]: only the company's signed, fresh, single-use
    // assertion for this consent page opens it.
    let Some(assertion) = verify_assertion(
        &state,
        &endpoint.delegation,
        &urls.consent(),
        form.one("assertion").unwrap_or_default(),
    )
    .await
    else {
        return page(
            StatusCode::UNAUTHORIZED,
            &brand,
            "The sign-in could not be verified. Try again from your agent.",
        );
    };
    let expires = chrono::DateTime::from_timestamp(assertion.exp + CLOCK_SKEW_SECS, 0)
        .unwrap_or_else(Utc::now);
    match state
        .db
        .record_pact_assertion(endpoint.channel.internal_id, &assertion.jti, expires)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return page(
                StatusCode::BAD_REQUEST,
                &brand,
                "This sign-in link was already used.",
            );
        }
        Err(err) => return internal(err),
    }
    let user_code = assertion.user_code.trim().to_ascii_uppercase();
    let pending = match state
        .db
        .pending_pact_device_authorization(endpoint.channel.internal_id, &user_code)
        .await
    {
        Ok(Some(pending)) => pending,
        Ok(None) => return expired(&brand),
        Err(err) => return internal(err),
    };
    let key = match endpoint.key(&state).await {
        Ok(key) => key,
        Err(err) => return internal(err),
    };
    let now = Utc::now().timestamp();
    let session = match key.sign(
        CONSENT_SESSION_TYPE,
        &ConsentSession {
            aud: urls.decision(),
            sub: assertion.sub.clone(),
            user_code,
            email: assertion.email.clone(),
            iat: now,
            exp: now + CONSENT_SESSION_TTL_SECS,
        },
    ) {
        Ok(session) => session,
        Err(err) => return internal(err),
    };
    let requested = parse_scope(&pending.requested_scope);
    html(
        StatusCode::OK,
        &consent_page(&ConsentView {
            brand: &brand,
            agent_origin: &origin(&pending.client_id),
            account: assertion.email.as_deref().unwrap_or(&assertion.sub),
            scopes: &requested
                .iter()
                .filter_map(|id| endpoint.delegation.scope(id))
                .map(|scope| (scope.id.as_str(), scope.description.as_str()))
                .collect::<Vec<_>>(),
            action: &urls.decision(),
            session: &session,
        }),
    )
}

async fn decision(
    State(state): State<ChannelA2aState>,
    Path(channel_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let endpoint = match endpoint(&state, &channel_id).await {
        Ok(endpoint) => endpoint,
        Err(response) => return response,
    };
    let brand = endpoint.brand_name().to_string();
    let urls = Urls::from_request(&headers, uri.path());
    let form = Form::parse(&body);
    let key = match endpoint.key(&state).await {
        Ok(key) => key,
        Err(err) => return internal(err),
    };
    let Some(session) = key.verify::<ConsentSession>(
        CONSENT_SESSION_TYPE,
        form.one("session").unwrap_or_default(),
        Some(&urls.decision()),
    ) else {
        return page(
            StatusCode::BAD_REQUEST,
            &brand,
            "This consent page expired. Start again from your agent.",
        );
    };
    let channel = endpoint.channel.internal_id;
    let pending = match state
        .db
        .pending_pact_device_authorization(channel, &session.user_code)
        .await
    {
        Ok(Some(pending)) => pending,
        Ok(None) => return expired(&brand),
        Err(err) => return internal(err),
    };
    let chosen: HashSet<&str> = form.all("scope").collect();
    let granted: Vec<String> = parse_scope(&pending.requested_scope)
        .into_iter()
        .filter(|scope| chosen.contains(scope.as_str()))
        .collect();
    let approved = form.one("decision") == Some("allow") && !granted.is_empty();
    let recorded = if approved {
        state
            .db
            .approve_pact_device_authorization(
                &pending.device_code_hash,
                &PactGrantRow {
                    id: format!("a2agrant_{}", uuid::Uuid::now_v7().simple()),
                    channel_id: channel,
                    client_id: pending.client_id.clone(),
                    brand_user_id: session.sub.clone(),
                    scope: granted.join(" "),
                    expires_at: Utc::now() + chrono::Duration::days(GRANT_TTL_DAYS),
                    revoked_at: None,
                },
            )
            .await
    } else {
        state
            .db
            .deny_pact_device_authorization(channel, &pending.device_code_hash, &session.sub)
            .await
    };
    match recorded {
        Ok(true) => {}
        Ok(false) => return expired(&brand),
        Err(err) => return internal(err),
    }
    let status = if approved { "approved" } else { "denied" };
    match endpoint.delegation.connected_url.as_deref() {
        Some(connected) => {
            let mut done = match url::Url::parse(connected) {
                Ok(done) => done,
                Err(_) => return finished(&brand, approved),
            };
            {
                let mut query = done.query_pairs_mut();
                query.append_pair("status", status);
                if approved {
                    query.append_pair("scope", &granted.join(" "));
                }
                query.append_pair("client", &origin(&pending.client_id));
            }
            (
                StatusCode::SEE_OTHER,
                [(header::LOCATION, done.to_string())],
            )
                .into_response()
        }
        None => finished(&brand, approved),
    }
}

/// The host a personal agent's issuer URL names, shown to the user.
fn origin(client_id: &str) -> String {
    url::Url::parse(client_id)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| client_id.to_string())
}

fn expired(brand: &str) -> Response {
    page(
        StatusCode::BAD_REQUEST,
        brand,
        "This request expired or was already answered. Start again from your agent.",
    )
}

fn finished(brand: &str, approved: bool) -> Response {
    page(
        StatusCode::OK,
        brand,
        if approved {
            "Done. You can return to your agent."
        } else {
            "Nothing was shared. You can return to your agent."
        },
    )
}

fn html(status: StatusCode, body: &str) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_FRAME_OPTIONS, "DENY"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; form-action *; base-uri 'none'; frame-ancestors 'none'",
            ),
        ],
        body.to_string(),
    )
        .into_response()
}

const STYLE: &str = "body{font-family:system-ui,sans-serif;max-width:32rem;margin:3rem auto;padding:0 1rem;color:#1d2433}\
h1{font-size:1.3rem}label{display:block;margin:.6rem 0}.muted{color:#5b6475;font-size:.9rem}\
button{margin:1rem .5rem 0 0;padding:.5rem 1.2rem;font-size:1rem}";

fn page(status: StatusCode, brand: &str, message: &str) -> Response {
    html(
        status,
        &format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>{brand}</title><style>{STYLE}</style></head>\
<body><h1>{brand}</h1><p>{message}</p></body></html>",
            brand = escape_html(brand),
            message = escape_html(message),
        ),
    )
}

struct ConsentView<'a> {
    brand: &'a str,
    agent_origin: &'a str,
    account: &'a str,
    scopes: &'a [(&'a str, &'a str)],
    action: &'a str,
    session: &'a str,
}

/// §5.3: the personal agent's origin, the company, and each requested scope
/// as a checkbox the user may uncheck. Descriptions are shown verbatim.
fn consent_page(view: &ConsentView<'_>) -> String {
    let scopes: String = view
        .scopes
        .iter()
        .map(|(id, description)| {
            format!(
                "<label><input type=\"checkbox\" name=\"scope\" value=\"{id}\" checked> {description}</label>",
                id = escape_html(id),
                description = escape_html(description),
            )
        })
        .collect();
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>{brand}: allow access</title><style>{STYLE}</style></head>\
<body><form method=\"post\" action=\"{action}\">\
<h1>Allow {agent} to act on your {brand} account?</h1>\
<p class=\"muted\">Signed in to {brand} as {account}.</p>\
{scopes}\
<input type=\"hidden\" name=\"session\" value=\"{session}\">\
<button type=\"submit\" name=\"decision\" value=\"allow\">Allow</button>\
<button type=\"submit\" name=\"decision\" value=\"deny\">Deny</button>\
<p class=\"muted\">You can come back and change this later by starting again from your agent.</p>\
</form></body></html>",
        brand = escape_html(view.brand),
        agent = escape_html(view.agent_origin),
        account = escape_html(view.account),
        action = escape_html(view.action),
        session = escape_html(view.session),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_derive_from_the_interface_url() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("x.example"));
        let urls = Urls::from_request(&headers, "/v1/a2a/ch/oauth/consent/decision");
        assert_eq!(urls.interface, "https://x.example/v1/a2a/ch");
        assert_eq!(urls.issuer, "https://x.example/v1/a2a/ch/oauth");
        assert_eq!(
            urls.decision(),
            "https://x.example/v1/a2a/ch/oauth/consent/decision"
        );
        assert_eq!(
            urls.metadata(),
            "https://x.example/v1/a2a/ch/oauth/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn scope_parsing_drops_blanks_and_duplicates() {
        assert_eq!(parse_scope(" a  b a c "), vec!["a", "b", "c"]);
        assert!(parse_scope("").is_empty());
    }

    #[test]
    fn user_codes_use_the_unambiguous_alphabet() {
        let code = user_code();
        assert_eq!(code.len(), 9);
        assert_eq!(&code[4..5], "-");
        assert!(
            code.replace('-', "")
                .bytes()
                .all(|b| USER_CODE_ALPHABET.contains(&b))
        );
    }

    #[test]
    fn login_link_keeps_the_login_query_and_adds_return_to() {
        assert_eq!(
            login_link(
                "https://brand.example/login?lang=en",
                "https://p.example/consent?user_code=AB"
            ),
            "https://brand.example/login?lang=en&return_to=https%3A%2F%2Fp.example%2Fconsent%3Fuser_code%3DAB"
        );
    }

    #[test]
    fn consent_page_escapes_company_text() {
        let html = consent_page(&ConsentView {
            brand: "<Brand>",
            agent_origin: "pa.example",
            account: "jane@example.com",
            scopes: &[("orders:read", "See \"your\" <orders>")],
            action: "https://p.example/decision",
            session: "a.b.c",
        });
        assert!(html.contains("&lt;Brand&gt;"));
        assert!(html.contains("See &quot;your&quot; &lt;orders&gt;"));
        assert!(html.contains("name=\"session\" value=\"a.b.c\""));
        assert!(!html.contains("<Brand>"));
    }

    #[test]
    fn card_scopes_keep_config_order() {
        let delegation: PactDelegationConfig = serde_json::from_value(json!({
            "login_url": "https://b.example/login",
            "login_issuer": "https://b.example",
            "login_jwks_uri": "https://b.example/jwks",
            "scopes": [
                { "id": "z:read", "description": "Zed" },
                { "id": "a:write", "description": "A \"quoted\" one" },
            ],
        }))
        .unwrap();
        let card = json!({ "securitySchemes": {
            "userDelegation": security_scheme("https://x.example/v1/a2a/c", &delegation),
        }});
        let text = card_json(card, Some(&delegation));
        let parsed: Value = serde_json::from_str(&text).unwrap();
        let scopes = &parsed["securitySchemes"]["userDelegation"]["oauth2SecurityScheme"]["flows"]
            ["deviceCode"]["scopes"];
        assert_eq!(scopes["a:write"], "A \"quoted\" one");
        assert!(text.find("z:read").unwrap() < text.find("a:write").unwrap());
    }
}

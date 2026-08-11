//! OAuth 2.1 authorization-code client: the protocol steps, and nothing else.
//!
//! This started life in `everruns-core` as a client every host could share.
//! EVE-879 moved it here: MCP login/refresh is its only consumer, and the
//! execution kernel should not carry token-exchange plumbing or form encoding
//! for a flow it never runs. The server's control-plane OAuth flows (provider
//! connect, user connections) keep their own egress-based exchange client.
//!
//! What lives here is deliberately the *stateless* half:
//!
//! - discovery — protected-resource metadata (RFC 9728) → authorization-server
//!   metadata (RFC 8414 / OpenID discovery)
//! - dynamic client registration (RFC 7591)
//! - PKCE (RFC 7636) and the authorize URL
//! - code exchange, refresh, and the resulting [`TokenSet`]
//! - issuer validation on the callback (RFC 9207)
//!
//! What does *not* live here is everything hosts disagree about: opening a
//! browser, listening on a loopback port or serving a redirect route, and
//! where tokens are persisted. A host owns those and calls in for the rest.
//!
//! Every request goes through [`EgressService`], so discovery and token calls
//! get the same egress policy and DNS-pinned SSRF protection as the traffic
//! they authenticate. This matters more than it looks: for MCP the
//! authorization-server URL is *discovered from the remote server*, which makes
//! it attacker-influenced input pointed at an internal-network probe unless it
//! goes through the boundary.

use std::collections::BTreeMap;

use base64::Engine as _;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use everruns_core::egress::{EgressRequest, EgressRequestKind, EgressService};

/// Refresh this long before a token actually expires, so a request issued at
/// the edge of validity does not race the clock.
pub const DEFAULT_REFRESH_SKEW_SECONDS: i64 = 60;

/// Errors from the OAuth client.
#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    /// The endpoint URL is not usable (not absolute, or plaintext HTTP to a
    /// non-loopback host).
    #[error("insecure or malformed {what} URL: {url}")]
    InsecureUrl {
        /// Which endpoint was rejected.
        what: &'static str,
        /// The offending URL.
        url: String,
    },
    /// The authorization server answered, but not with success.
    #[error("{what} failed with HTTP {status}: {body}")]
    Http {
        /// Which step failed.
        what: &'static str,
        /// HTTP status returned.
        status: u16,
        /// Response body, truncated for logging.
        body: String,
    },
    /// The response was not the JSON document this step expects.
    #[error("{what} returned an unusable response: {message}")]
    Malformed {
        /// Which step failed.
        what: &'static str,
        /// What was wrong.
        message: String,
    },
    /// The `iss` returned on the callback did not match the issuer we started
    /// the flow with (RFC 9207 mix-up defense).
    #[error("authorization response issuer mismatch: expected {expected}, got {actual}")]
    IssuerMismatch {
        /// Issuer the flow was started against.
        expected: String,
        /// Issuer the callback claimed.
        actual: String,
    },
    /// No refresh token is available, so the grant cannot be renewed.
    #[error("token set has no refresh token")]
    NotRefreshable,
    /// The egress boundary refused or failed the request.
    #[error("egress failure during {what}: {message}")]
    Egress {
        /// Which step failed.
        what: &'static str,
        /// Underlying error text.
        message: String,
    },
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, OAuthError>;

/// A PKCE verifier/challenge pair (RFC 7636, S256).
///
/// `Debug` redacts the verifier: it is the secret half of the pair and leaks
/// grant-hijacking capability if it reaches logs.
#[derive(Clone)]
pub struct PkcePair {
    /// High-entropy secret kept by the client until code exchange.
    pub verifier: String,
    /// S256 hash of the verifier, sent on the authorize request.
    pub challenge: String,
}

impl std::fmt::Debug for PkcePair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PkcePair")
            .field("verifier", &"[REDACTED]")
            .field("challenge", &self.challenge)
            .finish()
    }
}

impl PkcePair {
    /// Generate a fresh pair.
    pub fn generate() -> Self {
        let verifier = random_url_safe(32);
        let challenge = base64_url(Sha256::digest(verifier.as_bytes()).as_slice());
        Self {
            verifier,
            challenge,
        }
    }
}

/// Generate an opaque, URL-safe random string (CSRF `state`, nonces).
pub fn random_state() -> String {
    random_url_safe(24)
}

fn random_url_safe(bytes: usize) -> String {
    let mut rng = rand::rng();
    let buf: Vec<u8> = (0..bytes).map(|_| rng.random::<u8>()).collect();
    base64_url(&buf)
}

fn base64_url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Protected-resource metadata (RFC 9728): what a resource server says about
/// the authorization servers that can issue tokens for it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProtectedResourceMetadata {
    /// Canonical resource identifier, when advertised.
    #[serde(default)]
    pub resource: Option<String>,
    /// Issuers that can mint tokens for this resource.
    #[serde(default)]
    pub authorization_servers: Vec<String>,
}

/// Authorization-server metadata (RFC 8414 / OpenID discovery).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationServerMetadata {
    /// Issuer identifier; echoed back on the callback as `iss`.
    pub issuer: String,
    /// Where the user-agent is sent to authorize.
    pub authorization_endpoint: String,
    /// Where codes and refresh tokens are exchanged.
    pub token_endpoint: String,
    /// Dynamic client registration endpoint, when the server offers one.
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    /// Scopes the server advertises.
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    /// PKCE methods the server advertises.
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
}

/// A client registered dynamically (RFC 7591), or configured by hand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredClient {
    /// Issued client identifier.
    pub client_id: String,
    /// Issued secret, for confidential clients only.
    #[serde(default)]
    pub client_secret: Option<String>,
}

/// Request body for dynamic client registration.
#[derive(Debug, Clone)]
pub struct ClientRegistration {
    /// Human-readable client name shown on the consent screen.
    pub client_name: String,
    /// Redirect URIs this client will use.
    pub redirect_uris: Vec<String>,
    /// Scopes to request, space-delimited.
    pub scope: Option<String>,
}

/// Tokens held for a grant, plus the bookkeeping needed to renew them without
/// re-running discovery.
///
/// `Serialize` exists for encrypted persistence (control-plane rows, a CLI
/// token file) — never serialize a `TokenSet` into events, snapshots, or API
/// responses. `Debug` redacts every credential field so tracing a token set
/// cannot leak the grant.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenSet {
    /// The bearer (or other type) access token.
    pub access_token: String,
    /// Refresh token, when the server issued one.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Token type; `Bearer` unless the server says otherwise.
    #[serde(default = "default_token_type")]
    pub token_type: String,
    /// Absolute expiry, derived from `expires_in` at issue time.
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    /// Granted scope, when reported.
    #[serde(default)]
    pub scope: Option<String>,
    /// Token endpoint that issued this set, so a refresh is self-contained.
    #[serde(default)]
    pub token_endpoint: Option<String>,
    /// Client id used to obtain it.
    #[serde(default)]
    pub client_id: Option<String>,
    /// Client secret, for confidential clients.
    #[serde(default)]
    pub client_secret: Option<String>,
}

fn default_token_type() -> String {
    "Bearer".to_string()
}

impl std::fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenSet")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("token_type", &self.token_type)
            .field("expires_at", &self.expires_at)
            .field("scope", &self.scope)
            .field("token_endpoint", &self.token_endpoint)
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl TokenSet {
    /// Whether the token is expired at `now`. An expiry exactly at `now`
    /// counts as expired.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|expiry| expiry <= now)
    }

    /// Whether the token should be renewed before use, allowing `skew_seconds`
    /// of headroom. A token with no expiry never needs refreshing.
    pub fn needs_refresh(&self, now: DateTime<Utc>, skew_seconds: i64) -> bool {
        self.expires_at
            .is_some_and(|expiry| expiry <= now + ChronoDuration::seconds(skew_seconds))
    }

    /// The `Authorization` header value this token set contributes.
    pub fn authorization_value(&self) -> String {
        format!("{} {}", self.token_type, self.access_token)
    }
}

/// Parameters for building the authorize URL.
#[derive(Debug, Clone)]
pub struct AuthorizeParams<'a> {
    /// Client identifier.
    pub client_id: &'a str,
    /// Redirect URI registered for this client.
    pub redirect_uri: &'a str,
    /// PKCE challenge for this attempt.
    pub code_challenge: &'a str,
    /// CSRF state to be echoed back.
    pub state: &'a str,
    /// Requested scope, space-delimited.
    pub scope: Option<&'a str>,
    /// Resource indicator (RFC 8707) — which API the token is for. MCP relies
    /// on this to keep a token from being replayed against another resource.
    pub resource: Option<&'a str>,
}

/// Build the authorization URL the user-agent should visit.
pub fn authorize_url(
    metadata: &AuthorizationServerMetadata,
    params: &AuthorizeParams<'_>,
) -> Result<String> {
    let mut url = require_secure(&metadata.authorization_endpoint, "authorization endpoint")?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", params.client_id);
        query.append_pair("redirect_uri", params.redirect_uri);
        query.append_pair("code_challenge", params.code_challenge);
        query.append_pair("code_challenge_method", "S256");
        query.append_pair("state", params.state);
        if let Some(scope) = params.scope {
            query.append_pair("scope", scope);
        }
        if let Some(resource) = params.resource {
            query.append_pair("resource", resource);
        }
    }
    Ok(url.to_string())
}

/// Validate the `iss` returned on the callback against the issuer the flow
/// started with (RFC 9207). A server that returns no `iss` is accepted, since
/// the parameter is not universally implemented; a *mismatched* one is not.
pub fn validate_callback_issuer(expected: &str, returned: Option<&str>) -> Result<()> {
    match returned {
        None => Ok(()),
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(OAuthError::IssuerMismatch {
            expected: expected.to_string(),
            actual: actual.to_string(),
        }),
    }
}

/// Reject endpoints that are not absolute HTTPS. Plaintext loopback is allowed
/// because the redirect leg of a native-app flow (RFC 8252) is local.
fn require_secure(raw: &str, what: &'static str) -> Result<Url> {
    let url = Url::parse(raw).map_err(|_| OAuthError::InsecureUrl {
        what,
        url: raw.to_string(),
    })?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() == "https" || (url.scheme() == "http" && loopback) {
        Ok(url)
    } else {
        Err(OAuthError::InsecureUrl {
            what,
            url: raw.to_string(),
        })
    }
}

/// The OAuth client. Holds no state: every call is a request/response.
pub struct OAuthClient<'a> {
    egress: &'a dyn EgressService,
    kind: EgressRequestKind,
    timeout_ms: u64,
}

/// Default per-request timeout for discovery and token calls.
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

impl<'a> OAuthClient<'a> {
    /// Build a client that sends through `egress`, labeling requests `kind`.
    pub fn new(egress: &'a dyn EgressService, kind: EgressRequestKind) -> Self {
        Self {
            egress,
            kind,
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }

    /// Override the per-request timeout.
    pub fn timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        what: &'static str,
    ) -> Result<Option<T>> {
        let request = EgressRequest::new("GET", url, self.kind.clone())
            .header("accept", "application/json")
            .timeout_ms(self.timeout_ms)
            .require_dns_pinning();
        let response = self
            .egress
            .send(request)
            .await
            .map_err(|error| OAuthError::Egress {
                what,
                message: error.to_string(),
            })?;
        if response.status == 404 {
            return Ok(None);
        }
        if !(200..300).contains(&response.status) {
            return Err(OAuthError::Http {
                what,
                status: response.status,
                body: truncate(&String::from_utf8_lossy(&response.body)),
            });
        }
        serde_json::from_slice(&response.body)
            .map(Some)
            .map_err(|error| OAuthError::Malformed {
                what,
                message: error.to_string(),
            })
    }

    async fn post_form(
        &self,
        url: &str,
        form: &BTreeMap<&str, String>,
        what: &'static str,
    ) -> Result<TokenResponse> {
        require_secure(url, "token endpoint")?;
        let body = serde_urlencoded::to_string(form).map_err(|error| OAuthError::Malformed {
            what,
            message: error.to_string(),
        })?;
        let request = EgressRequest::new("POST", url, self.kind.clone())
            .header("content-type", "application/x-www-form-urlencoded")
            .header("accept", "application/json")
            .body(body.into_bytes())
            .timeout_ms(self.timeout_ms)
            .require_dns_pinning();
        let response = self
            .egress
            .send(request)
            .await
            .map_err(|error| OAuthError::Egress {
                what,
                message: error.to_string(),
            })?;
        if !(200..300).contains(&response.status) {
            return Err(OAuthError::Http {
                what,
                status: response.status,
                body: truncate(&String::from_utf8_lossy(&response.body)),
            });
        }
        serde_json::from_slice(&response.body).map_err(|error| OAuthError::Malformed {
            what,
            message: error.to_string(),
        })
    }

    /// Fetch protected-resource metadata for `resource_url` (RFC 9728).
    ///
    /// `Ok(None)` means the resource does not publish the document — a normal
    /// case, and the caller should fall back to treating the resource origin
    /// as the issuer.
    pub async fn discover_protected_resource(
        &self,
        resource_url: &str,
    ) -> Result<Option<ProtectedResourceMetadata>> {
        let url = require_secure(resource_url, "resource")?;
        let document = well_known(&url, "oauth-protected-resource");
        self.get_json(&document, "protected-resource discovery")
            .await
    }

    /// Fetch authorization-server metadata for `issuer`, trying the RFC 8414
    /// location first and the OpenID location second.
    pub async fn discover_authorization_server(
        &self,
        issuer: &str,
    ) -> Result<AuthorizationServerMetadata> {
        let url = require_secure(issuer, "issuer")?;
        for suffix in ["oauth-authorization-server", "openid-configuration"] {
            let document = well_known(&url, suffix);
            if let Some(metadata) = self
                .get_json::<AuthorizationServerMetadata>(
                    &document,
                    "authorization-server discovery",
                )
                .await?
            {
                require_secure(&metadata.authorization_endpoint, "authorization endpoint")?;
                require_secure(&metadata.token_endpoint, "token endpoint")?;
                return Ok(metadata);
            }
        }
        Err(OAuthError::Malformed {
            what: "authorization-server discovery",
            message: format!("no metadata document published by {issuer}"),
        })
    }

    /// Register a public client dynamically (RFC 7591).
    pub async fn register_client(
        &self,
        registration_endpoint: &str,
        registration: &ClientRegistration,
    ) -> Result<RegisteredClient> {
        require_secure(registration_endpoint, "registration endpoint")?;
        let mut body = serde_json::json!({
            "client_name": registration.client_name,
            "redirect_uris": registration.redirect_uris,
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        });
        if let Some(scope) = &registration.scope {
            body["scope"] = serde_json::Value::String(scope.clone());
        }
        let request = EgressRequest::new("POST", registration_endpoint, self.kind.clone())
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(serde_json::to_vec(&body).unwrap_or_default())
            .timeout_ms(self.timeout_ms)
            .require_dns_pinning();
        let response = self
            .egress
            .send(request)
            .await
            .map_err(|error| OAuthError::Egress {
                what: "client registration",
                message: error.to_string(),
            })?;
        if !(200..300).contains(&response.status) {
            return Err(OAuthError::Http {
                what: "client registration",
                status: response.status,
                body: truncate(&String::from_utf8_lossy(&response.body)),
            });
        }
        serde_json::from_slice(&response.body).map_err(|error| OAuthError::Malformed {
            what: "client registration",
            message: error.to_string(),
        })
    }

    /// Exchange an authorization code for tokens.
    #[allow(clippy::too_many_arguments)]
    pub async fn exchange_code(
        &self,
        token_endpoint: &str,
        client: &RegisteredClient,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
        resource: Option<&str>,
    ) -> Result<TokenSet> {
        let mut form = BTreeMap::new();
        form.insert("grant_type", "authorization_code".to_string());
        form.insert("code", code.to_string());
        form.insert("code_verifier", code_verifier.to_string());
        form.insert("redirect_uri", redirect_uri.to_string());
        form.insert("client_id", client.client_id.clone());
        if let Some(secret) = &client.client_secret {
            form.insert("client_secret", secret.clone());
        }
        if let Some(resource) = resource {
            form.insert("resource", resource.to_string());
        }
        let response = self
            .post_form(token_endpoint, &form, "code exchange")
            .await?;
        Ok(response.into_token_set(token_endpoint, client, None))
    }

    /// Renew an access token from its refresh token.
    ///
    /// The renewed set carries the previous refresh token forward when the
    /// server does not rotate it, so a non-rotating server does not silently
    /// lose the ability to refresh again.
    pub async fn refresh(&self, tokens: &TokenSet) -> Result<TokenSet> {
        let refresh_token = tokens
            .refresh_token
            .as_deref()
            .ok_or(OAuthError::NotRefreshable)?;
        let token_endpoint = tokens
            .token_endpoint
            .as_deref()
            .ok_or(OAuthError::Malformed {
                what: "token refresh",
                message: "token set has no token endpoint".to_string(),
            })?;
        let client = RegisteredClient {
            client_id: tokens.client_id.clone().unwrap_or_default(),
            client_secret: tokens.client_secret.clone(),
        };

        let mut form = BTreeMap::new();
        form.insert("grant_type", "refresh_token".to_string());
        form.insert("refresh_token", refresh_token.to_string());
        if !client.client_id.is_empty() {
            form.insert("client_id", client.client_id.clone());
        }
        if let Some(secret) = &client.client_secret {
            form.insert("client_secret", secret.clone());
        }
        let response = self
            .post_form(token_endpoint, &form, "token refresh")
            .await?;
        Ok(response.into_token_set(token_endpoint, &client, tokens.refresh_token.clone()))
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
}

impl TokenResponse {
    fn into_token_set(
        self,
        token_endpoint: &str,
        client: &RegisteredClient,
        carried_refresh_token: Option<String>,
    ) -> TokenSet {
        TokenSet {
            expires_at: self
                .expires_in
                .map(|seconds| Utc::now() + ChronoDuration::seconds(seconds)),
            refresh_token: self.refresh_token.or(carried_refresh_token),
            token_type: self.token_type.unwrap_or_else(default_token_type),
            scope: self.scope,
            access_token: self.access_token,
            token_endpoint: Some(token_endpoint.to_string()),
            client_id: (!client.client_id.is_empty()).then(|| client.client_id.clone()),
            client_secret: client.client_secret.clone(),
        }
    }
}

/// Well-known document URL for `url`'s origin.
///
/// RFC 8414 inserts the suffix *before* any issuer path
/// (`https://host/.well-known/<suffix>/tenant`), which is what lets an issuer
/// with a path component publish metadata at all.
fn well_known(url: &Url, suffix: &str) -> String {
    let origin = format!(
        "{}://{}{}",
        url.scheme(),
        url.host_str().unwrap_or_default(),
        url.port().map(|p| format!(":{p}")).unwrap_or_default()
    );
    let path = url.path().trim_end_matches('/');
    if path.is_empty() {
        format!("{origin}/.well-known/{suffix}")
    } else {
        format!("{origin}/.well-known/{suffix}{path}")
    }
}

fn truncate(body: &str) -> String {
    const LIMIT: usize = 512;
    if body.len() <= LIMIT {
        return body.to_string();
    }
    let mut end = LIMIT;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &body[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use everruns_core::egress::{EgressError, EgressResponse, EgressResult};
    use std::sync::{Arc, Mutex};

    /// Egress double: answers by URL, records what was sent. Tests exercise
    /// the whole client without a socket, which is the payoff of routing OAuth
    /// through the egress seam instead of a private HTTP client.
    #[derive(Default)]
    struct FakeEgress {
        responses: Mutex<Vec<(String, u16, String)>>,
        sent: Arc<Mutex<Vec<EgressRequest>>>,
    }

    impl FakeEgress {
        fn with(routes: Vec<(&str, u16, &str)>) -> Self {
            Self {
                responses: Mutex::new(
                    routes
                        .into_iter()
                        .map(|(url, status, body)| (url.to_string(), status, body.to_string()))
                        .collect(),
                ),
                sent: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl EgressService for FakeEgress {
        async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
            self.sent.lock().unwrap().push(request.clone());
            let responses = self.responses.lock().unwrap();
            let matched = responses
                .iter()
                .find(|(url, _, _)| *url == request.url)
                .map(|(_, status, body)| (*status, body.clone()));
            match matched {
                Some((status, body)) => Ok(EgressResponse {
                    status,
                    headers: BTreeMap::new(),
                    body: body.into_bytes(),
                }),
                None => Ok(EgressResponse {
                    status: 404,
                    headers: BTreeMap::new(),
                    body: Vec::new(),
                }),
            }
        }

        async fn send_stream(
            &self,
            _request: EgressRequest,
        ) -> EgressResult<everruns_core::egress::EgressStreamResponse> {
            Err(EgressError::Transport(
                "streaming not supported".to_string(),
            ))
        }
    }

    fn metadata_json() -> &'static str {
        r#"{
            "issuer": "https://auth.example.com",
            "authorization_endpoint": "https://auth.example.com/authorize",
            "token_endpoint": "https://auth.example.com/token",
            "registration_endpoint": "https://auth.example.com/register",
            "code_challenge_methods_supported": ["S256"]
        }"#
    }

    #[test]
    fn pkce_challenge_is_the_s256_of_the_verifier() {
        let pair = PkcePair::generate();
        let expected = base64_url(Sha256::digest(pair.verifier.as_bytes()).as_slice());
        assert_eq!(pair.challenge, expected);
        assert_ne!(pair.verifier, pair.challenge);
        // Two generations must not collide.
        assert_ne!(PkcePair::generate().verifier, pair.verifier);
    }

    #[test]
    fn plaintext_endpoints_are_rejected_except_on_loopback() {
        assert!(require_secure("https://auth.example.com/token", "token endpoint").is_ok());
        assert!(require_secure("http://127.0.0.1:1455/callback", "redirect").is_ok());
        assert!(require_secure("http://auth.example.com/token", "token endpoint").is_err());
        assert!(require_secure("not-a-url", "token endpoint").is_err());
    }

    #[test]
    fn authorize_url_carries_pkce_state_and_resource() {
        let metadata: AuthorizationServerMetadata = serde_json::from_str(metadata_json()).unwrap();
        let url = authorize_url(
            &metadata,
            &AuthorizeParams {
                client_id: "client-1",
                redirect_uri: "http://127.0.0.1:1455/callback",
                code_challenge: "challenge",
                state: "state-1",
                scope: Some("mcp:read"),
                resource: Some("https://mcp.example.com/"),
            },
        )
        .unwrap();

        assert!(url.starts_with("https://auth.example.com/authorize?"));
        for expected in [
            "response_type=code",
            "client_id=client-1",
            "code_challenge=challenge",
            "code_challenge_method=S256",
            "state=state-1",
            "scope=mcp%3Aread",
            "resource=https%3A%2F%2Fmcp.example.com%2F",
        ] {
            assert!(url.contains(expected), "missing {expected} in {url}");
        }
    }

    #[test]
    fn callback_issuer_must_match_when_present() {
        assert!(validate_callback_issuer("https://auth.example.com", None).is_ok());
        assert!(
            validate_callback_issuer("https://auth.example.com", Some("https://auth.example.com"))
                .is_ok()
        );
        assert!(matches!(
            validate_callback_issuer("https://auth.example.com", Some("https://evil.example.com")),
            Err(OAuthError::IssuerMismatch { .. })
        ));
    }

    #[test]
    fn well_known_inserts_the_suffix_before_an_issuer_path() {
        let plain = Url::parse("https://auth.example.com").unwrap();
        assert_eq!(
            well_known(&plain, "oauth-authorization-server"),
            "https://auth.example.com/.well-known/oauth-authorization-server"
        );
        let tenanted = Url::parse("https://auth.example.com/tenant-a").unwrap();
        assert_eq!(
            well_known(&tenanted, "oauth-authorization-server"),
            "https://auth.example.com/.well-known/oauth-authorization-server/tenant-a"
        );
    }

    #[tokio::test]
    async fn discovery_falls_back_to_the_openid_document() {
        let egress = FakeEgress::with(vec![(
            "https://auth.example.com/.well-known/openid-configuration",
            200,
            metadata_json(),
        )]);
        let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);

        let metadata = client
            .discover_authorization_server("https://auth.example.com")
            .await
            .expect("discovery should fall back");

        assert_eq!(metadata.token_endpoint, "https://auth.example.com/token");
        // The RFC 8414 location is tried first, then OpenID.
        let sent = egress.sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert!(sent[0].url.contains("oauth-authorization-server"));
        assert!(sent[1].url.contains("openid-configuration"));
    }

    #[tokio::test]
    async fn discovery_requires_secure_endpoints_in_the_metadata() {
        // A compromised or sloppy server advertising a plaintext token endpoint
        // must not be used, even though the metadata document itself was HTTPS.
        let egress = FakeEgress::with(vec![(
            "https://auth.example.com/.well-known/oauth-authorization-server",
            200,
            r#"{
                "issuer": "https://auth.example.com",
                "authorization_endpoint": "https://auth.example.com/authorize",
                "token_endpoint": "http://auth.example.com/token"
            }"#,
        )]);
        let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);

        assert!(matches!(
            client
                .discover_authorization_server("https://auth.example.com")
                .await,
            Err(OAuthError::InsecureUrl { .. })
        ));
    }

    #[tokio::test]
    async fn missing_protected_resource_document_is_not_an_error() {
        let egress = FakeEgress::with(vec![]);
        let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);

        let found = client
            .discover_protected_resource("https://mcp.example.com/sse")
            .await
            .expect("a 404 means 'not published', not a failure");

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn code_exchange_posts_pkce_and_returns_a_dated_token_set() {
        let egress = FakeEgress::with(vec![(
            "https://auth.example.com/token",
            200,
            r#"{"access_token":"at-1","refresh_token":"rt-1","token_type":"Bearer","expires_in":3600}"#,
        )]);
        let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);

        let tokens = client
            .exchange_code(
                "https://auth.example.com/token",
                &RegisteredClient {
                    client_id: "client-1".to_string(),
                    client_secret: None,
                },
                "code-1",
                "verifier-1",
                "http://127.0.0.1:1455/callback",
                Some("https://mcp.example.com/"),
            )
            .await
            .expect("exchange should succeed");

        assert_eq!(tokens.access_token, "at-1");
        assert_eq!(tokens.authorization_value(), "Bearer at-1");
        assert!(tokens.expires_at.is_some());
        assert_eq!(
            tokens.token_endpoint.as_deref(),
            Some("https://auth.example.com/token")
        );

        let sent = egress.sent.lock().unwrap();
        let body = String::from_utf8(sent[0].body.clone()).unwrap();
        assert!(body.contains("grant_type=authorization_code"));
        assert!(body.contains("code_verifier=verifier-1"));
        assert!(body.contains("resource=https"));
        assert!(sent[0].dns_pinning_required, "token calls must be pinned");
    }

    #[tokio::test]
    async fn refresh_carries_the_old_refresh_token_when_the_server_does_not_rotate() {
        let egress = FakeEgress::with(vec![(
            "https://auth.example.com/token",
            200,
            r#"{"access_token":"at-2","token_type":"Bearer","expires_in":3600}"#,
        )]);
        let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);
        let existing = TokenSet {
            access_token: "at-1".to_string(),
            refresh_token: Some("rt-1".to_string()),
            token_type: "Bearer".to_string(),
            expires_at: Some(Utc::now() - ChronoDuration::seconds(1)),
            scope: None,
            token_endpoint: Some("https://auth.example.com/token".to_string()),
            client_id: Some("client-1".to_string()),
            client_secret: None,
        };

        let renewed = client.refresh(&existing).await.expect("refresh");

        assert_eq!(renewed.access_token, "at-2");
        assert_eq!(
            renewed.refresh_token.as_deref(),
            Some("rt-1"),
            "a non-rotating server must not cost us the refresh token"
        );
    }

    #[tokio::test]
    async fn refresh_without_a_refresh_token_is_a_typed_error() {
        let egress = FakeEgress::with(vec![]);
        let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);
        let tokens = TokenSet {
            access_token: "at-1".to_string(),
            refresh_token: None,
            token_type: "Bearer".to_string(),
            expires_at: None,
            scope: None,
            token_endpoint: Some("https://auth.example.com/token".to_string()),
            client_id: None,
            client_secret: None,
        };

        assert!(matches!(
            client.refresh(&tokens).await,
            Err(OAuthError::NotRefreshable)
        ));
    }

    #[tokio::test]
    async fn token_endpoint_errors_surface_status_and_body() {
        let egress = FakeEgress::with(vec![(
            "https://auth.example.com/token",
            400,
            r#"{"error":"invalid_grant"}"#,
        )]);
        let client = OAuthClient::new(&egress, EgressRequestKind::Mcp);

        let error = client
            .exchange_code(
                "https://auth.example.com/token",
                &RegisteredClient {
                    client_id: "client-1".to_string(),
                    client_secret: None,
                },
                "code-1",
                "verifier-1",
                "http://127.0.0.1:1455/callback",
                None,
            )
            .await
            .expect_err("a 400 must not be read as success");

        match error {
            OAuthError::Http { status, body, .. } => {
                assert_eq!(status, 400);
                assert!(body.contains("invalid_grant"));
            }
            other => panic!("expected an HTTP error, got {other}"),
        }
    }

    #[test]
    fn token_set_debug_redacts_every_credential_field() {
        let tokens = TokenSet {
            access_token: "at-secret".to_string(),
            refresh_token: Some("rt-secret".to_string()),
            token_type: "Bearer".to_string(),
            expires_at: None,
            scope: Some("mcp:read".to_string()),
            token_endpoint: Some("https://auth.example.com/token".to_string()),
            client_id: Some("client-1".to_string()),
            client_secret: Some("cs-secret".to_string()),
        };
        let rendered = format!("{tokens:?}");
        for secret in ["at-secret", "rt-secret", "cs-secret"] {
            assert!(!rendered.contains(secret), "{secret} leaked: {rendered}");
        }
        assert!(rendered.contains("[REDACTED]"));
        // Non-secret bookkeeping stays visible for debugging.
        assert!(rendered.contains("client-1"));
        assert!(rendered.contains("https://auth.example.com/token"));
    }

    #[test]
    fn pkce_debug_redacts_the_verifier() {
        let pair = PkcePair::generate();
        let rendered = format!("{pair:?}");
        assert!(!rendered.contains(&pair.verifier), "{rendered}");
        assert!(rendered.contains(&pair.challenge));
    }

    #[test]
    fn expiry_and_refresh_windows_are_distinct() {
        let now = Utc::now();
        let tokens = TokenSet {
            access_token: "at".to_string(),
            refresh_token: None,
            token_type: "Bearer".to_string(),
            expires_at: Some(now + ChronoDuration::seconds(30)),
            scope: None,
            token_endpoint: None,
            client_id: None,
            client_secret: None,
        };

        assert!(!tokens.is_expired(now), "still valid for another 30s");
        assert!(
            tokens.needs_refresh(now, DEFAULT_REFRESH_SKEW_SECONDS),
            "but inside the refresh window, so renew before using it"
        );

        let no_expiry = TokenSet {
            expires_at: None,
            ..tokens
        };
        assert!(!no_expiry.is_expired(now));
        assert!(!no_expiry.needs_refresh(now, DEFAULT_REFRESH_SKEW_SECONDS));
    }
}

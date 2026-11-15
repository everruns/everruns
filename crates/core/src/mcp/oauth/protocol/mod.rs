//! OAuth 2.1 authorization-code client: the protocol steps, and nothing else.
//!
//! The Framework and Server call this implementation for outbound MCP OAuth.
//! Hosts own callback delivery, credential persistence, tenant attribution and
//! refresh locking. This module owns the protocol requests and validation.
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

use crate::egress::{EgressRequest, EgressRequestKind, EgressService};

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
        /// Redacted URL marker; endpoint URLs can contain credentials.
        url: String,
    },
    /// The authorization server answered, but not with success.
    #[error("{what} failed with HTTP {status}: {body}")]
    Http {
        /// Which step failed.
        what: &'static str,
        /// HTTP status returned.
        status: u16,
        /// Safe, recognized OAuth error code; arbitrary provider text is redacted.
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
    /// Legacy grants have no trustworthy resource; only a new login can bind them.
    #[error("MCP OAuth resource binding is missing; sign in again")]
    ReauthorizationRequired,
    /// The egress boundary refused or failed the request.
    #[error("egress failure during {what}: {message}")]
    Egress {
        /// Which step failed.
        what: &'static str,
        /// Safe transport classification; upstream text and URLs are omitted.
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
    /// Scopes advertised by the protected resource.
    #[serde(default)]
    pub scopes_supported: Vec<String>,
}

/// Authorization-server metadata (RFC 8414 / OpenID discovery).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationServerMetadata {
    /// Issuer identifier; echoed back on the callback as `iss`.
    #[serde(default, deserialize_with = "optional_issuer")]
    pub issuer: String,
    /// Where the user-agent is sent to authorize.
    pub authorization_endpoint: String,
    /// Where codes and refresh tokens are exchanged.
    pub token_endpoint: String,
    /// Dynamic client registration endpoint, when the server offers one.
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    /// Whether the issuer accepts an HTTPS Client ID Metadata Document URL.
    /// Absence is not support; never infer support from an endpoint URL.
    #[serde(default)]
    pub client_id_metadata_document_supported: bool,
    /// Scopes the server advertises.
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    /// PKCE methods the server advertises.
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
}

// Legacy Server metadata accepted both absent and null issuers. Discovery fills
// either from the selected issuer before validating the rest of the flow.
fn optional_issuer<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<String, D::Error> {
    Option::<String>::deserialize(deserializer).map(Option::unwrap_or_default)
}

/// A configured, dynamically registered, or public metadata-document client.
#[derive(Clone, Serialize, Deserialize)]
pub struct RegisteredClient {
    /// Issued client identifier, or the exact metadata-document URL for CIMD.
    pub client_id: String,
    /// Issued secret, for confidential clients only.
    #[serde(default)]
    pub client_secret: Option<String>,
}

impl std::fmt::Debug for RegisteredClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // THREAT[TM-FW-001]: confidential-client credentials must stay out of diagnostic output.
        f.debug_struct("RegisteredClient")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Validate a host-owned Client ID Metadata Document URL without fetching it.
///
/// The authorization server resolves the document. The host must publish matching
/// `client_id`, `client_name`, and `redirect_uris` metadata, with token endpoint
/// authentication `none` for this public-client flow. Keep the original string:
/// the document's identifier is compared exactly, without URL normalization.
pub fn validate_client_metadata_url(client_id: &str) -> Result<()> {
    let invalid = || OAuthError::Malformed {
        what: "client metadata URL",
        message:
            "expected an HTTPS URL with a non-root path, no userinfo, fragment, or dot segments"
                .to_string(),
    };
    let parsed = Url::parse(client_id).map_err(|_| invalid())?;
    // Url normalizes dot segments, backslashes, and some whitespace. Check the
    // original spelling too, so validation cannot change the client identity.
    let (_, authority_and_path) = client_id.split_once("://").ok_or_else(invalid)?;
    let (authority, path_and_query) = authority_and_path.split_once('/').ok_or_else(invalid)?;
    let raw_path = path_and_query.split(['?', '#']).next().unwrap_or_default();
    let has_dot_segment = raw_path.split('/').any(|segment| {
        let dots = segment.to_ascii_lowercase().replace("%2e", ".");
        dots == "." || dots == ".."
    });
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || parsed.path() == "/"
        || authority.contains('@')
        || parsed.fragment().is_some()
        || has_dot_segment
        || client_id
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return Err(invalid());
    }
    Ok(())
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
    /// Exact resource selected and validated at authorization time. Missing on
    /// legacy records, which must reauthorize before refreshing.
    #[serde(default)]
    pub resource: Option<String>,
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

/// Validate host-supplied authorization extensions without allowing protocol fields to be replaced.
pub fn validate_authorization_params(params: &BTreeMap<String, String>) -> Result<()> {
    for key in params.keys() {
        if [
            "client_id",
            "code_challenge",
            "code_challenge_method",
            "redirect_uri",
            "resource",
            "response_type",
            "scope",
            "state",
        ]
        .contains(&key.as_str())
        {
            return Err(OAuthError::Malformed {
                what: "authorization parameters",
                message: format!("OAuth authorization parameter '{key}' is reserved"),
            });
        }
    }
    Ok(())
}

/// Build a PKCE authorization URL with validated host-specific consent parameters.
pub fn authorize_url_with_params(
    metadata: &AuthorizationServerMetadata,
    params: &AuthorizeParams<'_>,
    extensions: &BTreeMap<String, String>,
) -> Result<String> {
    validate_authorization_params(extensions)?;
    let mut url =
        Url::parse(&authorize_url(metadata, params)?).map_err(|_| OAuthError::Malformed {
            what: "authorization URL",
            message: "invalid URL".into(),
        })?;
    url.query_pairs_mut().extend_pairs(extensions);
    Ok(url.into())
}

/// Reject a callback whose state differs from the captured login, before any exchange.
pub fn validate_callback_state(expected: &str, returned: &str) -> Result<()> {
    if expected.is_empty() || returned != expected {
        return Err(OAuthError::Malformed {
            what: "callback state",
            message: "authorization response state does not match".into(),
        });
    }
    Ok(())
}

/// Validate the `iss` returned on the callback against the issuer the flow
/// started with (RFC 9207). A server that returns no `iss` is accepted, since
/// the parameter is not universally implemented; a *mismatched* one is not.
pub fn validate_callback_issuer(expected: &str, returned: Option<&str>) -> Result<()> {
    match returned {
        None => Ok(()),
        Some(actual) if actual == expected => Ok(()),
        Some(_actual) => Err(OAuthError::IssuerMismatch {
            expected: "configured issuer".into(),
            actual: "different callback issuer".into(),
        }),
    }
}

/// Reject endpoints that are not absolute HTTPS. Plaintext loopback is allowed
/// because the redirect leg of a native-app flow (RFC 8252) is local.
fn require_secure(raw: &str, what: &'static str) -> Result<Url> {
    let url = Url::parse(raw).map_err(|_| OAuthError::InsecureUrl {
        what,
        url: "[REDACTED]".to_string(),
    })?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() == "https" || (url.scheme() == "http" && loopback) {
        Ok(url)
    } else {
        Err(OAuthError::InsecureUrl {
            what,
            url: "[REDACTED]".to_string(),
        })
    }
}

/// First advertised authorization server, or the MCP endpoint's origin when metadata is absent.
pub fn authorization_server_issuer(
    metadata: Option<&ProtectedResourceMetadata>,
    server_url: &str,
) -> Result<String> {
    let endpoint = require_secure(server_url, "resource")?;
    Ok(metadata
        .and_then(|m| m.authorization_servers.first())
        .cloned()
        .unwrap_or_else(|| endpoint.origin().ascii_serialization()))
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
                message: safe_egress_error(&error).to_string(),
            })?;
        if response.status == 404 {
            return Ok(None);
        }
        if !(200..300).contains(&response.status) {
            return Err(OAuthError::Http {
                what,
                status: response.status,
                body: safe_error_code(&response.body).to_string(),
            });
        }
        serde_json::from_slice(&response.body)
            .map(Some)
            .map_err(|_| OAuthError::Malformed {
                what,
                message: "invalid JSON response".into(),
            })
    }

    async fn post_form(
        &self,
        url: &str,
        form: &BTreeMap<&str, String>,
        what: &'static str,
    ) -> Result<TokenResponse> {
        require_secure(url, "token endpoint")?;
        let body = serde_urlencoded::to_string(form).map_err(|_| OAuthError::Malformed {
            what,
            message: "invalid OAuth request or response".into(),
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
                message: safe_egress_error(&error).to_string(),
            })?;
        if !(200..300).contains(&response.status) {
            return Err(OAuthError::Http {
                what,
                status: response.status,
                body: safe_error_code(&response.body).to_string(),
            });
        }
        let token: TokenResponse =
            serde_json::from_slice(&response.body).map_err(|_| OAuthError::Malformed {
                what,
                message: "invalid JSON token response".into(),
            })?;
        token.expires_at()?;
        Ok(token)
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
        if let Some(metadata) = self
            .get_json(&document, "protected-resource discovery")
            .await?
        {
            return Ok(Some(metadata));
        }
        let root = format!(
            "{}/.well-known/oauth-protected-resource",
            url.origin().ascii_serialization()
        );
        if root == document {
            return Ok(None);
        }
        self.get_json(&root, "protected-resource discovery").await
    }

    /// Fetch authorization-server metadata for `issuer`, trying the RFC 8414
    /// location first and the OpenID location second.
    pub async fn discover_authorization_server(
        &self,
        issuer: &str,
    ) -> Result<AuthorizationServerMetadata> {
        let url = require_secure(issuer, "issuer")?;
        let mut candidates = vec![
            well_known(&url, "oauth-authorization-server"),
            well_known(&url, "openid-configuration"),
        ];
        if !url.path().trim_end_matches('/').is_empty() {
            candidates.push(format!(
                "{}/.well-known/openid-configuration",
                url.as_str().trim_end_matches('/')
            ));
            // Keep the historical Server fallback for existing presets.
            candidates.push(format!(
                "{}/.well-known/oauth-authorization-server",
                url.as_str().trim_end_matches('/')
            ));
        }
        for document in candidates {
            if let Some(mut metadata) = self
                .get_json::<AuthorizationServerMetadata>(
                    &document,
                    "authorization-server discovery",
                )
                .await?
            {
                if metadata.issuer.is_empty() {
                    metadata.issuer = issuer.to_string();
                }
                let advertised = require_secure(&metadata.issuer, "issuer")?;
                if advertised.as_str().trim_end_matches('/') != url.as_str().trim_end_matches('/') {
                    return Err(OAuthError::IssuerMismatch {
                        expected: "configured issuer".into(),
                        actual: "different metadata issuer".into(),
                    });
                }
                require_secure(&metadata.authorization_endpoint, "authorization endpoint")?;
                require_secure(&metadata.token_endpoint, "token endpoint")?;
                if let Some(endpoint) = &metadata.registration_endpoint {
                    require_secure(endpoint, "registration endpoint")?;
                }
                return Ok(metadata);
            }
        }
        Err(OAuthError::Malformed {
            what: "authorization-server discovery",
            message: "authorization server publishes no metadata".into(),
        })
    }

    /// Register a public client dynamically (RFC 7591).
    pub async fn register_client(
        &self,
        registration_endpoint: &str,
        registration: &ClientRegistration,
    ) -> Result<RegisteredClient> {
        self.register_client_with_auth_method(
            registration_endpoint,
            registration,
            ClientAuthMethod::None,
        )
        .await
    }

    /// Register a public or confidential client using the host's authentication policy.
    pub async fn register_client_with_auth_method(
        &self,
        registration_endpoint: &str,
        registration: &ClientRegistration,
        auth_method: ClientAuthMethod,
    ) -> Result<RegisteredClient> {
        require_secure(registration_endpoint, "registration endpoint")?;
        let mut body = serde_json::json!({
            "client_name": registration.client_name,
            "redirect_uris": registration.redirect_uris,
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": match auth_method { ClientAuthMethod::None => "none", ClientAuthMethod::ClientSecretPost => "client_secret_post" },
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
                message: safe_egress_error(&error).to_string(),
            })?;
        if !(200..300).contains(&response.status) {
            return Err(OAuthError::Http {
                what: "client registration",
                status: response.status,
                body: safe_error_code(&response.body).to_string(),
            });
        }
        serde_json::from_slice(&response.body).map_err(|_| OAuthError::Malformed {
            what: "client registration",
            message: "invalid JSON response".into(),
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
        let response = self
            .exchange_code_response(
                token_endpoint,
                client,
                code,
                code_verifier,
                redirect_uri,
                resource,
            )
            .await?;
        response.into_token_set(token_endpoint, client, None, resource)
    }

    /// Exchange a code while leaving token persistence and expiry representation to the host.
    #[allow(clippy::too_many_arguments)]
    pub async fn exchange_code_response(
        &self,
        token_endpoint: &str,
        client: &RegisteredClient,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
        resource: Option<&str>,
    ) -> Result<TokenResponse> {
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
        self.post_form(token_endpoint, &form, "code exchange").await
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
        let resource = tokens
            .resource
            .as_deref()
            .filter(|resource| !resource.trim().is_empty())
            .ok_or(OAuthError::ReauthorizationRequired)?;
        let client = RegisteredClient {
            client_id: tokens.client_id.clone().unwrap_or_default(),
            client_secret: tokens.client_secret.clone(),
        };

        let response = self
            .refresh_response(token_endpoint, &client, refresh_token, resource)
            .await?;
        response.into_token_set(
            token_endpoint,
            &client,
            tokens.refresh_token.clone(),
            Some(resource),
        )
    }

    /// Renew a bound grant; hosts coordinate locking and persist rotation before releasing it.
    pub async fn refresh_response(
        &self,
        token_endpoint: &str,
        client: &RegisteredClient,
        refresh_token: &str,
        resource: &str,
    ) -> Result<TokenResponse> {
        if resource.trim().is_empty() {
            return Err(OAuthError::ReauthorizationRequired);
        }
        let mut form = BTreeMap::new();
        form.insert("grant_type", "refresh_token".to_string());
        form.insert("refresh_token", refresh_token.to_string());
        // THREAT[TM-MCP-008]: preserve the stored audience exactly, without discovery.
        form.insert("resource", resource.to_string());
        if !client.client_id.is_empty() {
            form.insert("client_id", client.client_id.clone());
        }
        if let Some(secret) = &client.client_secret {
            form.insert("client_secret", secret.clone());
        }
        self.post_form(token_endpoint, &form, "token refresh").await
    }
}

/// Wire token response, intended for host-owned secret persistence.
#[derive(Clone, Deserialize)]
pub struct TokenResponse {
    /// Access credential.
    pub access_token: String,
    /// Rotated refresh credential, when supplied.
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Authorization scheme; absence means Bearer.
    #[serde(default)]
    pub token_type: Option<String>,
    /// Token lifetime in seconds.
    #[serde(default)]
    pub expires_in: Option<i64>,
    /// Granted scopes.
    #[serde(default)]
    pub scope: Option<String>,
}
impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenResponse")
            .field("credentials", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

/// Token-endpoint authentication requested during dynamic registration.
#[derive(Debug, Clone, Copy)]
pub enum ClientAuthMethod {
    /// Public client, protected by PKCE.
    None,
    /// Confidential client, with its secret in the token form.
    ClientSecretPost,
}

impl TokenResponse {
    /// Convert the untrusted lifetime with checked arithmetic; invalid lifetimes never panic.
    pub fn expires_at(&self) -> Result<Option<DateTime<Utc>>> {
        self.expires_in
            .map(|seconds| {
                ChronoDuration::try_seconds(seconds)
                    .and_then(|delta| Utc::now().checked_add_signed(delta))
                    .ok_or(OAuthError::Malformed {
                        what: "token lifetime",
                        message: "invalid token lifetime".into(),
                    })
            })
            .transpose()
    }

    fn into_token_set(
        self,
        token_endpoint: &str,
        client: &RegisteredClient,
        carried_refresh_token: Option<String>,
        resource: Option<&str>,
    ) -> Result<TokenSet> {
        Ok(TokenSet {
            expires_at: self.expires_at()?,
            refresh_token: self.refresh_token.or(carried_refresh_token),
            token_type: self.token_type.unwrap_or_else(default_token_type),
            scope: self.scope,
            access_token: self.access_token,
            token_endpoint: Some(token_endpoint.to_string()),
            resource: resource.map(str::to_owned),
            client_id: (!client.client_id.is_empty()).then(|| client.client_id.clone()),
            client_secret: client.client_secret.clone(),
        })
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

fn safe_error_code(body: &[u8]) -> &'static str {
    // THREAT[TM-MCP-016]: providers can echo credentials; retain only fixed classifications.
    let value = serde_json::from_slice::<serde_json::Value>(body).ok();
    match value
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|v| v.as_str())
    {
        Some("invalid_grant") => "invalid_grant",
        Some("invalid_client") => "invalid_client",
        Some("invalid_scope") => "invalid_scope",
        Some("invalid_request") => "invalid_request",
        Some("unauthorized_client") => "unauthorized_client",
        Some("unsupported_grant_type") => "unsupported_grant_type",
        Some("access_denied") => "access_denied",
        _ => "[REDACTED]",
    }
}

fn safe_egress_error(error: &crate::egress::EgressError) -> &'static str {
    match error {
        crate::egress::EgressError::NetworkAccessDenied { .. } => "blocked_by_network_policy",
        crate::egress::EgressError::InvalidRequest(_) => "blocked_url",
        _ => "transport_failed",
    }
}

#[cfg(test)]
mod tests;

//! [`ChannelAuthVerifier`]: verifies one request against one
//! [`ChannelAuthConfig`], fetching identity-provider documents through an
//! SSRF-safe client.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use everruns_contracts::url_validation::{is_blocked_ip, validate_safe_url};
use jsonwebtoken::jwk::JwkSet;
use moka::future::Cache;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use serde_json::Value;
use tokio::net::lookup_host;
use tokio::time::timeout;
use url::Host;

use super::claims::{
    constant_time_eq, extract_basic_credentials, extract_bearer, normalize_issuer,
    principal_from_claims, validate_claim_requirements, verify_jwt_with_jwks, verify_shared_secret,
};
use super::{
    AGENTID_ISSUER, ChannelAuthConfig, ChannelAuthError, ChannelAuthMode, ChannelAuthPrincipal,
    ChannelAuthProviderConfig, GOOGLE_ISSUER, LegacyChannelAuth,
};

/// Checks an HTTP Basic password against a stored hash: `Ok(true)` on a
/// match, `Err(())` when the hash cannot be read.
///
/// Decision: the hash format belongs to the host's password store, so the
/// verifier takes the check as a function. A verifier without one answers
/// `Misconfigured` for `http_basic`, failing closed.
pub type PasswordCheck = Arc<dyn Fn(&str, &str) -> Result<bool, ()> + Send + Sync>;

#[derive(Clone)]
pub struct ChannelAuthVerifier {
    // Outbound HTTP clients are built per-request via `build_pinned_client` so
    // each request can pin reqwest's DNS resolution to addresses we just
    // validated, eliminating the rebinding TOCTOU window. The struct only
    // carries the discovery/JWKS response caches across calls.
    discovery_cache: Cache<String, OidcDiscovery>,
    jwks_cache: Cache<String, Arc<JwkSet>>,
    /// Public JSON documents other parties publish about themselves, such as
    /// a personal agent's client metadata and keys (Poppy).
    document_cache: Cache<String, Arc<Value>>,
    password_check: Option<PasswordCheck>,
}

impl std::fmt::Debug for ChannelAuthVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelAuthVerifier")
            .field("password_check", &self.password_check.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for ChannelAuthVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelAuthVerifier {
    pub fn new() -> Self {
        Self {
            discovery_cache: Cache::builder()
                .time_to_live(Duration::from_secs(15 * 60))
                .max_capacity(256)
                .build(),
            jwks_cache: Cache::builder()
                .time_to_live(Duration::from_secs(15 * 60))
                .max_capacity(256)
                .build(),
            document_cache: Cache::builder()
                .time_to_live(Duration::from_secs(5 * 60))
                .max_capacity(1024)
                .build(),
            password_check: None,
        }
    }

    /// Verify `http_basic` passwords with `check`. See [`PasswordCheck`].
    pub fn with_password_check(mut self, check: PasswordCheck) -> Self {
        self.password_check = Some(check);
        self
    }

    pub async fn verify(
        &self,
        auth: &ChannelAuthConfig,
        headers: &HeaderMap,
        legacy: LegacyChannelAuth<'_>,
    ) -> Result<(), ChannelAuthError> {
        self.verify_principal(auth, headers, legacy)
            .await
            .map(|_| ())
    }

    pub async fn verify_principal(
        &self,
        auth: &ChannelAuthConfig,
        headers: &HeaderMap,
        legacy: LegacyChannelAuth<'_>,
    ) -> Result<Option<ChannelAuthPrincipal>, ChannelAuthError> {
        match auth.mode {
            ChannelAuthMode::Anonymous => Ok(None),
            ChannelAuthMode::SharedSecret => {
                let expected = legacy
                    .shared_secret
                    .ok_or(ChannelAuthError::Misconfigured)?;
                verify_shared_secret(headers, expected)?;
                Ok(None)
            }
            ChannelAuthMode::ApiKey => {
                let api_key = legacy.api_key.ok_or(ChannelAuthError::Misconfigured)?;
                verify_shared_secret(headers, api_key)?;
                Ok(None)
            }
            ChannelAuthMode::HttpBasic => {
                self.verify_basic(auth, headers)?;
                Ok(None)
            }
            ChannelAuthMode::GoogleOidc | ChannelAuthMode::Oidc => {
                self.verify_oidc(auth, headers).await.map(Some)
            }
            ChannelAuthMode::OAuth2Introspection => self
                .verify_oauth2_introspection(auth, headers)
                .await
                .map(Some),
            ChannelAuthMode::Mtls => {
                self.verify_mtls(auth, headers)?;
                Ok(None)
            }
        }
    }

    pub(super) fn verify_basic(
        &self,
        auth: &ChannelAuthConfig,
        headers: &HeaderMap,
    ) -> Result<(), ChannelAuthError> {
        let Some(ChannelAuthProviderConfig::HttpBasic {
            username,
            password_hash,
            ..
        }) = auth.provider.as_ref()
        else {
            return Err(ChannelAuthError::Misconfigured);
        };
        let expected_hash = password_hash
            .as_deref()
            .filter(|hash| !hash.trim().is_empty())
            .ok_or(ChannelAuthError::Misconfigured)?;
        let check = self
            .password_check
            .as_ref()
            .ok_or(ChannelAuthError::Misconfigured)?;
        let (provided_user, provided_password) =
            extract_basic_credentials(headers).ok_or(ChannelAuthError::Unauthorized)?;
        if provided_user != *username {
            return Err(ChannelAuthError::Unauthorized);
        }
        let valid = check(&provided_password, expected_hash)
            .map_err(|_| ChannelAuthError::Misconfigured)?;
        if valid {
            Ok(())
        } else {
            Err(ChannelAuthError::Unauthorized)
        }
    }

    async fn verify_oidc(
        &self,
        auth: &ChannelAuthConfig,
        headers: &HeaderMap,
    ) -> Result<ChannelAuthPrincipal, ChannelAuthError> {
        let token = extract_bearer(headers).ok_or(ChannelAuthError::Unauthorized)?;
        // `verifier_authority` names the key source that proves the claims. A
        // configured JWKS URL is endpoint-owner input, so it is the authority;
        // discovered keys are vouched for by the issuer itself, so the issuer is,
        // which keeps identities stable if the IdP rotates its jwks_uri.
        let (issuer, jwks_url, verifier_authority, requirements) = match auth.provider.as_ref() {
            Some(ChannelAuthProviderConfig::GoogleOidc {
                client_id,
                allowed_domains,
            }) => {
                let mut requirements = auth.requirements.clone();
                if requirements.audiences.is_empty() {
                    requirements.audiences.push(client_id.clone());
                }
                if requirements.domains.is_empty() {
                    requirements.domains = allowed_domains.clone();
                }
                (
                    GOOGLE_ISSUER.to_string(),
                    "https://www.googleapis.com/oauth2/v3/certs".to_string(),
                    format!("oidc-discovery:{GOOGLE_ISSUER}"),
                    requirements,
                )
            }
            Some(ChannelAuthProviderConfig::Oidc { issuer, jwks_url }) => {
                let discovery = self.discovery(issuer).await?;
                let issuer = normalize_issuer(issuer);
                let (jwks_url, verifier_authority) = match jwks_url {
                    Some(url) => (url.clone(), format!("oidc-jwks:{url}")),
                    None => (
                        discovery.jwks_uri.to_string(),
                        format!("oidc-discovery:{issuer}"),
                    ),
                };
                (
                    issuer,
                    jwks_url,
                    verifier_authority,
                    auth.requirements.clone(),
                )
            }
            _ => return Err(ChannelAuthError::Misconfigured),
        };

        if requirements.audiences.is_empty() {
            return Err(ChannelAuthError::Misconfigured);
        }
        let agentid = auth.is_agentid();
        let jwks = self.jwks(&jwks_url).await?;
        verify_jwt_with_jwks(
            token,
            &issuer,
            &jwks,
            &requirements,
            &verifier_authority,
            agentid,
        )
    }

    async fn verify_oauth2_introspection(
        &self,
        auth: &ChannelAuthConfig,
        headers: &HeaderMap,
    ) -> Result<ChannelAuthPrincipal, ChannelAuthError> {
        let token = extract_bearer(headers).ok_or(ChannelAuthError::Unauthorized)?;
        let Some(ChannelAuthProviderConfig::OAuth2Introspection {
            introspection_url,
            client_id,
            client_secret,
            ..
        }) = auth.provider.as_ref()
        else {
            return Err(ChannelAuthError::Misconfigured);
        };
        let client = build_pinned_client(introspection_url)
            .await
            .map_err(|_| ChannelAuthError::Misconfigured)?;
        let mut req = client.post(introspection_url).form(&[("token", token)]);
        if let Some(client_id) = client_id.as_deref().filter(|id| !id.is_empty()) {
            req = if let Some(secret) = client_secret.as_deref() {
                req.basic_auth(client_id, Some(secret))
            } else {
                req.basic_auth(client_id, Option::<&str>::None)
            };
        }
        let response = req
            .send()
            .await
            .map_err(|_| ChannelAuthError::ProviderUnavailable)?;
        if response.status().is_server_error() {
            return Err(ChannelAuthError::ProviderUnavailable);
        }
        if !response.status().is_success() {
            return Err(ChannelAuthError::Unauthorized);
        }
        let claims: Value = response
            .json()
            .await
            .map_err(|_| ChannelAuthError::ProviderUnavailable)?;
        if claims.get("active").and_then(Value::as_bool) != Some(true) {
            return Err(ChannelAuthError::Unauthorized);
        }
        let requirements = auth.requirements.clone();
        validate_claim_requirements(&claims, &requirements)?;
        principal_from_claims(
            &claims,
            &format!("oauth2_introspection:{introspection_url}"),
        )
    }

    pub(super) fn verify_mtls(
        &self,
        auth: &ChannelAuthConfig,
        headers: &HeaderMap,
    ) -> Result<(), ChannelAuthError> {
        let Some(ChannelAuthProviderConfig::Mtls {
            header_name,
            allowed_values,
            proxy_secret_header,
            proxy_secret,
            ..
        }) = auth.provider.as_ref()
        else {
            return Err(ChannelAuthError::Misconfigured);
        };
        // THREAT[TM-AUTH-021]: mTLS identity requires BOTH the cert identity
        // header (injected by the reverse proxy after client-cert verification)
        // AND a shared proxy secret (proxy_secret_header / proxy_secret). The
        // proxy secret proves the request came through the trusted TLS terminator.
        // Without it, a caller who knows or guesses an allowed cert value can
        // spoof the identity header directly (EVE-545).
        let (proxy_hdr, expected_secret) =
            match (proxy_secret_header.as_deref(), proxy_secret.as_deref()) {
                (Some(h), Some(s)) if !h.trim().is_empty() && !s.trim().is_empty() => (h, s),
                _ => return Err(ChannelAuthError::Misconfigured),
            };
        let cert_value = headers
            .get(header_name)
            .and_then(|v| v.to_str().ok())
            .ok_or(ChannelAuthError::Unauthorized)?;
        if !allowed_values.iter().any(|a| a == cert_value) {
            return Err(ChannelAuthError::Unauthorized);
        }
        let provided_secret = headers
            .get(proxy_hdr)
            .and_then(|v| v.to_str().ok())
            .ok_or(ChannelAuthError::Unauthorized)?;
        if constant_time_eq(provided_secret.as_bytes(), expected_secret.as_bytes()) {
            Ok(())
        } else {
            Err(ChannelAuthError::Unauthorized)
        }
    }

    async fn discovery(&self, issuer: &str) -> Result<OidcDiscovery, ChannelAuthError> {
        let issuer = normalize_issuer(issuer);
        if let Some(discovery) = self.discovery_cache.get(&issuer).await {
            return Ok(discovery);
        }
        // Validate the issuer URL itself even though we don't hit it directly:
        // misconfigured private/loopback issuers should fail fast before we
        // construct any derived URL.
        resolve_and_validate(&issuer)
            .await
            .map_err(|_| ChannelAuthError::Misconfigured)?;
        let url = format!("{issuer}/.well-known/openid-configuration");
        let client = build_pinned_client(&url)
            .await
            .map_err(|_| ChannelAuthError::Misconfigured)?;
        let discovery = client
            .get(&url)
            .send()
            .await
            .map_err(|_| ChannelAuthError::ProviderUnavailable)?
            .error_for_status()
            .map_err(|_| ChannelAuthError::ProviderUnavailable)?
            .json::<OidcDiscovery>()
            .await
            .map_err(|_| ChannelAuthError::ProviderUnavailable)?;
        // Early-fail on a misconfigured jwks_uri before caching the discovery.
        resolve_and_validate(&discovery.jwks_uri)
            .await
            .map_err(|_| ChannelAuthError::Misconfigured)?;
        self.discovery_cache.insert(issuer, discovery.clone()).await;
        Ok(discovery)
    }

    /// AgentID's published keys, found through AgentID's own discovery
    /// document (never an operator-supplied URL).
    pub async fn agentid_jwks(&self) -> Result<Arc<JwkSet>, ChannelAuthError> {
        let discovery = self.discovery(AGENTID_ISSUER).await?;
        self.jwks(discovery.jwks_uri.as_str()).await
    }

    /// The key set at `jwks_url`, cached for 15 minutes.
    pub async fn jwks(&self, jwks_url: &str) -> Result<Arc<JwkSet>, ChannelAuthError> {
        if let Some(jwks) = self.jwks_cache.get(jwks_url).await {
            return Ok(jwks);
        }
        let client = build_pinned_client(jwks_url)
            .await
            .map_err(|_| ChannelAuthError::Misconfigured)?;
        let jwks = client
            .get(jwks_url)
            .send()
            .await
            .map_err(|_| ChannelAuthError::ProviderUnavailable)?
            .error_for_status()
            .map_err(|_| ChannelAuthError::ProviderUnavailable)?
            .json::<JwkSet>()
            .await
            .map_err(|_| ChannelAuthError::ProviderUnavailable)?;
        let jwks = Arc::new(jwks);
        self.jwks_cache
            .insert(jwks_url.to_string(), jwks.clone())
            .await;
        Ok(jwks)
    }

    /// A public JSON document at an HTTPS URL, through the same SSRF-safe
    /// pinned client, without redirects, at most `MAX_PUBLIC_DOCUMENT_BYTES`.
    /// THREAT[TM-POPPY-002]: the URL comes from an unauthenticated caller.
    pub async fn public_document(&self, url: &str) -> Result<Arc<Value>, ChannelAuthError> {
        if let Some(document) = self.document_cache.get(url).await {
            return Ok(document);
        }
        if !url.starts_with("https://") {
            return Err(ChannelAuthError::Misconfigured);
        }
        let client = build_pinned_client(url)
            .await
            .map_err(|_| ChannelAuthError::Misconfigured)?;
        let mut response = client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| ChannelAuthError::ProviderUnavailable)?
            .error_for_status()
            .map_err(|_| ChannelAuthError::ProviderUnavailable)?;
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ChannelAuthError::ProviderUnavailable)?
        {
            if body.len() + chunk.len() > MAX_PUBLIC_DOCUMENT_BYTES {
                return Err(ChannelAuthError::ProviderUnavailable);
            }
            body.extend_from_slice(&chunk);
        }
        let document = Arc::new(
            serde_json::from_slice::<Value>(&body)
                .map_err(|_| ChannelAuthError::ProviderUnavailable)?,
        );
        self.document_cache
            .insert(url.to_string(), document.clone())
            .await;
        Ok(document)
    }

    /// Serve `document` for `url` without fetching it, for tests and local
    /// development where the publisher is not reachable.
    pub async fn prime_public_document(&self, url: &str, document: Value) {
        self.document_cache
            .insert(url.to_string(), Arc::new(document))
            .await;
    }

    /// Answer `issuer`'s OIDC discovery with `jwks_uri` and that URL with
    /// `jwks`, without fetching either, for tests and local development
    /// where the identity provider is not reachable (or is on loopback, which
    /// the SSRF guard refuses).
    pub async fn prime_oidc(&self, issuer: &str, jwks_uri: &str, jwks: JwkSet) {
        self.discovery_cache
            .insert(
                normalize_issuer(issuer),
                OidcDiscovery {
                    jwks_uri: jwks_uri.to_string(),
                },
            )
            .await;
        self.jwks_cache
            .insert(jwks_uri.to_string(), Arc::new(jwks))
            .await;
    }
}

/// Largest public document [`ChannelAuthVerifier::public_document`] reads.
const MAX_PUBLIC_DOCUMENT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Deserialize)]
struct OidcDiscovery {
    jwks_uri: String,
}

// Cap DNS resolution to the same budget as the outbound HTTP request so a slow
// or stuck resolver cannot stall auth verification past the overall timeout.
const DNS_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Result of resolving and validating an outbound auth-provider URL.
///
/// For hostnames we hold the pre-validated socket addresses so the actual HTTP
/// request can pin DNS to those exact IPs (closing the TOCTOU window where a
/// rebinding attacker could flip the DNS answer between validation and
/// connect). For URLs that already use a literal IP, `addrs` is empty because
/// [`validate_safe_url`] already classified the address.
pub(super) struct ResolvedTarget {
    host: Option<String>,
    addrs: Vec<SocketAddr>,
}

pub(super) async fn resolve_and_validate(raw_url: &str) -> Result<ResolvedTarget, ()> {
    let url = validate_safe_url(raw_url).map_err(|_| ())?;
    let host = url.host().ok_or(())?;
    if matches!(host, Host::Ipv4(_) | Host::Ipv6(_)) {
        return Ok(ResolvedTarget {
            host: None,
            addrs: Vec::new(),
        });
    }
    let port = url.port_or_known_default().ok_or(())?;
    let host = host.to_string();
    let resolved: Vec<SocketAddr> = timeout(DNS_LOOKUP_TIMEOUT, lookup_host((host.as_str(), port)))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?
        .collect();
    if resolved.is_empty() {
        return Err(());
    }
    for addr in &resolved {
        if is_blocked_ip(addr.ip()) {
            return Err(());
        }
    }
    Ok(ResolvedTarget {
        host: Some(host),
        addrs: resolved,
    })
}

/// Build a one-shot HTTP client whose DNS resolution is pinned to addresses we
/// just validated. This eliminates the TOCTOU window where reqwest's own
/// resolver could otherwise reach a different (and now-private) address than
/// the one we checked.
fn pinned_http_client(target: &ResolvedTarget) -> Result<reqwest::Client, ()> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(HTTP_REQUEST_TIMEOUT);
    if let Some(host) = target.host.as_deref() {
        builder = builder.resolve_to_addrs(host, &target.addrs);
    }
    builder.build().map_err(|_| ())
}

async fn build_pinned_client(raw_url: &str) -> Result<reqwest::Client, ()> {
    let target = resolve_and_validate(raw_url).await?;
    pinned_http_client(&target)
}

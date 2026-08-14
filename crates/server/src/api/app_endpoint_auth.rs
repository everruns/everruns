// Shared auth verifier for public App endpoint channels.
//
// Decision: endpoint auth config is inline on `app_channels.channel_config.auth`
// for the first product iteration. The verifier remains provider-shaped so the
// same runtime can later read org-level reusable providers without changing
// AG-UI/A2A ingress behavior.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::kernel_imports::{
    everruns_provider::url_validation::is_blocked_ip,
    everruns_provider::url_validation::validate_safe_url,
};
use axum::http::{HeaderMap, header::AUTHORIZATION};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use everruns_platform::{
    AppEndpointAuthConfig, AppEndpointAuthMode, AppEndpointAuthProviderConfig,
    AppEndpointAuthRequirements,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use moka::future::Cache;
use serde::Deserialize;
use serde_json::Value;
use tokio::net::lookup_host;
use tokio::time::timeout;
use url::Host;

use crate::security::constant_time_eq;
use crate::storage::password::verify_password;

#[derive(Clone)]
pub struct AppEndpointAuthVerifier {
    // Outbound HTTP clients are built per-request via `build_pinned_client` so
    // each request can pin reqwest's DNS resolution to addresses we just
    // validated, eliminating the rebinding TOCTOU window. The struct only
    // carries the discovery/JWKS response caches across calls.
    discovery_cache: Cache<String, OidcDiscovery>,
    jwks_cache: Cache<String, Arc<JwkSet>>,
}

impl Default for AppEndpointAuthVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl AppEndpointAuthVerifier {
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
        }
    }

    pub async fn verify(
        &self,
        auth: &AppEndpointAuthConfig,
        headers: &HeaderMap,
        legacy: LegacyEndpointAuth<'_>,
    ) -> Result<(), AppEndpointAuthError> {
        self.verify_principal(auth, headers, legacy)
            .await
            .map(|_| ())
    }

    pub async fn verify_principal(
        &self,
        auth: &AppEndpointAuthConfig,
        headers: &HeaderMap,
        legacy: LegacyEndpointAuth<'_>,
    ) -> Result<Option<AppEndpointAuthPrincipal>, AppEndpointAuthError> {
        match auth.mode {
            AppEndpointAuthMode::Anonymous => Ok(None),
            AppEndpointAuthMode::SharedSecret => {
                let expected = legacy
                    .shared_secret
                    .ok_or(AppEndpointAuthError::Misconfigured)?;
                verify_shared_secret(headers, expected)?;
                Ok(None)
            }
            AppEndpointAuthMode::ApiKey => {
                let api_key = legacy.api_key.ok_or(AppEndpointAuthError::Misconfigured)?;
                verify_shared_secret(headers, api_key)?;
                Ok(None)
            }
            AppEndpointAuthMode::HttpBasic => {
                self.verify_basic(auth, headers)?;
                Ok(None)
            }
            AppEndpointAuthMode::GoogleOidc | AppEndpointAuthMode::Oidc => {
                self.verify_oidc(auth, headers).await.map(Some)
            }
            AppEndpointAuthMode::OAuth2Introspection => self
                .verify_oauth2_introspection(auth, headers)
                .await
                .map(Some),
            AppEndpointAuthMode::Mtls => {
                self.verify_mtls(auth, headers)?;
                Ok(None)
            }
        }
    }

    fn verify_basic(
        &self,
        auth: &AppEndpointAuthConfig,
        headers: &HeaderMap,
    ) -> Result<(), AppEndpointAuthError> {
        let Some(AppEndpointAuthProviderConfig::HttpBasic {
            username,
            password_hash,
            ..
        }) = auth.provider.as_ref()
        else {
            return Err(AppEndpointAuthError::Misconfigured);
        };
        let expected_hash = password_hash
            .as_deref()
            .filter(|hash| !hash.trim().is_empty())
            .ok_or(AppEndpointAuthError::Misconfigured)?;
        let (provided_user, provided_password) =
            extract_basic_credentials(headers).ok_or(AppEndpointAuthError::Unauthorized)?;
        if provided_user != *username {
            return Err(AppEndpointAuthError::Unauthorized);
        }
        let valid = verify_password(&provided_password, expected_hash)
            .map_err(|_| AppEndpointAuthError::Misconfigured)?;
        if valid {
            Ok(())
        } else {
            Err(AppEndpointAuthError::Unauthorized)
        }
    }

    async fn verify_oidc(
        &self,
        auth: &AppEndpointAuthConfig,
        headers: &HeaderMap,
    ) -> Result<AppEndpointAuthPrincipal, AppEndpointAuthError> {
        let token = extract_bearer(headers).ok_or(AppEndpointAuthError::Unauthorized)?;
        let (issuer, jwks_url, requirements) = match auth.provider.as_ref() {
            Some(AppEndpointAuthProviderConfig::GoogleOidc {
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
                    "https://accounts.google.com".to_string(),
                    "https://www.googleapis.com/oauth2/v3/certs".to_string(),
                    requirements,
                )
            }
            Some(AppEndpointAuthProviderConfig::Oidc { issuer, jwks_url }) => {
                let discovery = self.discovery(issuer).await?;
                (
                    normalize_issuer(issuer),
                    jwks_url
                        .clone()
                        .unwrap_or_else(|| discovery.jwks_uri.to_string()),
                    auth.requirements.clone(),
                )
            }
            _ => return Err(AppEndpointAuthError::Misconfigured),
        };

        if requirements.audiences.is_empty() {
            return Err(AppEndpointAuthError::Misconfigured);
        }
        let jwks = self.jwks(&jwks_url).await?;
        let header = decode_header(token).map_err(|_| AppEndpointAuthError::Unauthorized)?;
        let kid = header.kid.ok_or(AppEndpointAuthError::Unauthorized)?;
        let jwk = jwks
            .keys
            .iter()
            .find(|jwk| jwk.common.key_id.as_deref() == Some(kid.as_str()))
            .ok_or(AppEndpointAuthError::Unauthorized)?;
        let alg = header.alg;
        if !is_public_key_algorithm(alg) {
            return Err(AppEndpointAuthError::Unauthorized);
        }
        let key = DecodingKey::from_jwk(jwk).map_err(|_| AppEndpointAuthError::Unauthorized)?;
        let mut validation = Validation::new(alg);
        validation.set_issuer(&[issuer]);
        validation.set_audience(&requirements.audiences);
        validation.validate_nbf = true;
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        let claims = decode::<Value>(token, &key, &validation)
            .map_err(|_| AppEndpointAuthError::Unauthorized)?
            .claims;
        validate_claim_requirements(&claims, &requirements)?;
        principal_from_claims(&claims)
    }

    async fn verify_oauth2_introspection(
        &self,
        auth: &AppEndpointAuthConfig,
        headers: &HeaderMap,
    ) -> Result<AppEndpointAuthPrincipal, AppEndpointAuthError> {
        let token = extract_bearer(headers).ok_or(AppEndpointAuthError::Unauthorized)?;
        let Some(AppEndpointAuthProviderConfig::OAuth2Introspection {
            introspection_url,
            client_id,
            client_secret,
        }) = auth.provider.as_ref()
        else {
            return Err(AppEndpointAuthError::Misconfigured);
        };
        let client = build_pinned_client(introspection_url)
            .await
            .map_err(|_| AppEndpointAuthError::Misconfigured)?;
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
            .map_err(|_| AppEndpointAuthError::ProviderUnavailable)?;
        if response.status().is_server_error() {
            return Err(AppEndpointAuthError::ProviderUnavailable);
        }
        if !response.status().is_success() {
            return Err(AppEndpointAuthError::Unauthorized);
        }
        let claims: Value = response
            .json()
            .await
            .map_err(|_| AppEndpointAuthError::ProviderUnavailable)?;
        if claims.get("active").and_then(Value::as_bool) != Some(true) {
            return Err(AppEndpointAuthError::Unauthorized);
        }
        let requirements = auth.requirements.clone();
        validate_claim_requirements(&claims, &requirements)?;
        principal_from_claims(&claims)
    }

    fn verify_mtls(
        &self,
        auth: &AppEndpointAuthConfig,
        headers: &HeaderMap,
    ) -> Result<(), AppEndpointAuthError> {
        let Some(AppEndpointAuthProviderConfig::Mtls {
            header_name,
            allowed_values,
            proxy_secret_header,
            proxy_secret,
        }) = auth.provider.as_ref()
        else {
            return Err(AppEndpointAuthError::Misconfigured);
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
                _ => return Err(AppEndpointAuthError::Misconfigured),
            };
        let cert_value = headers
            .get(header_name)
            .and_then(|v| v.to_str().ok())
            .ok_or(AppEndpointAuthError::Unauthorized)?;
        if !allowed_values.iter().any(|a| a == cert_value) {
            return Err(AppEndpointAuthError::Unauthorized);
        }
        let provided_secret = headers
            .get(proxy_hdr)
            .and_then(|v| v.to_str().ok())
            .ok_or(AppEndpointAuthError::Unauthorized)?;
        if constant_time_eq(provided_secret.as_bytes(), expected_secret.as_bytes()) {
            Ok(())
        } else {
            Err(AppEndpointAuthError::Unauthorized)
        }
    }

    async fn discovery(&self, issuer: &str) -> Result<OidcDiscovery, AppEndpointAuthError> {
        let issuer = normalize_issuer(issuer);
        if let Some(discovery) = self.discovery_cache.get(&issuer).await {
            return Ok(discovery);
        }
        // Validate the issuer URL itself even though we don't hit it directly:
        // misconfigured private/loopback issuers should fail fast before we
        // construct any derived URL.
        resolve_and_validate(&issuer)
            .await
            .map_err(|_| AppEndpointAuthError::Misconfigured)?;
        let url = format!("{issuer}/.well-known/openid-configuration");
        let client = build_pinned_client(&url)
            .await
            .map_err(|_| AppEndpointAuthError::Misconfigured)?;
        let discovery = client
            .get(&url)
            .send()
            .await
            .map_err(|_| AppEndpointAuthError::ProviderUnavailable)?
            .error_for_status()
            .map_err(|_| AppEndpointAuthError::ProviderUnavailable)?
            .json::<OidcDiscovery>()
            .await
            .map_err(|_| AppEndpointAuthError::ProviderUnavailable)?;
        // Early-fail on a misconfigured jwks_uri before caching the discovery.
        resolve_and_validate(&discovery.jwks_uri)
            .await
            .map_err(|_| AppEndpointAuthError::Misconfigured)?;
        self.discovery_cache.insert(issuer, discovery.clone()).await;
        Ok(discovery)
    }

    async fn jwks(&self, jwks_url: &str) -> Result<Arc<JwkSet>, AppEndpointAuthError> {
        if let Some(jwks) = self.jwks_cache.get(jwks_url).await {
            return Ok(jwks);
        }
        let client = build_pinned_client(jwks_url)
            .await
            .map_err(|_| AppEndpointAuthError::Misconfigured)?;
        let jwks = client
            .get(jwks_url)
            .send()
            .await
            .map_err(|_| AppEndpointAuthError::ProviderUnavailable)?
            .error_for_status()
            .map_err(|_| AppEndpointAuthError::ProviderUnavailable)?
            .json::<JwkSet>()
            .await
            .map_err(|_| AppEndpointAuthError::ProviderUnavailable)?;
        let jwks = Arc::new(jwks);
        self.jwks_cache
            .insert(jwks_url.to_string(), jwks.clone())
            .await;
        Ok(jwks)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEndpointAuthPrincipal {
    pub issuer: String,
    pub subject: String,
}

#[derive(Debug, Clone, Copy)]
pub struct LegacyEndpointAuth<'a> {
    pub shared_secret: Option<&'a str>,
    pub api_key: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEndpointAuthError {
    Unauthorized,
    Misconfigured,
    ProviderUnavailable,
}

#[derive(Debug, Clone, Deserialize)]
struct OidcDiscovery {
    jwks_uri: String,
}

fn principal_from_claims(claims: &Value) -> Result<AppEndpointAuthPrincipal, AppEndpointAuthError> {
    let issuer = claims
        .get("iss")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(AppEndpointAuthError::Unauthorized)?;
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(AppEndpointAuthError::Unauthorized)?;
    Ok(AppEndpointAuthPrincipal {
        issuer: normalize_issuer(issuer),
        subject: subject.to_string(),
    })
}

fn normalize_issuer(issuer: &str) -> String {
    issuer.trim().trim_end_matches('/').to_string()
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
struct ResolvedTarget {
    host: Option<String>,
    addrs: Vec<SocketAddr>,
}

async fn resolve_and_validate(raw_url: &str) -> Result<ResolvedTarget, ()> {
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

fn is_public_key_algorithm(alg: Algorithm) -> bool {
    matches!(
        alg,
        Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512
            | Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::EdDSA
    )
}

pub fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let auth = headers.get(AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = auth.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(token)
}

fn extract_basic_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let auth = headers.get(AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, encoded) = auth.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = BASE64_STANDARD.decode(encoded).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (username, password) = decoded.split_once(':')?;
    Some((username.to_string(), password.to_string()))
}

fn verify_shared_secret(headers: &HeaderMap, expected: &str) -> Result<(), AppEndpointAuthError> {
    let provided = extract_bearer(headers).ok_or(AppEndpointAuthError::Unauthorized)?;
    if constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(AppEndpointAuthError::Unauthorized)
    }
}

fn validate_claim_requirements(
    claims: &Value,
    requirements: &AppEndpointAuthRequirements,
) -> Result<(), AppEndpointAuthError> {
    if !requirements.audiences.is_empty() {
        let audiences = claim_audience_set(claims);
        if !requirements
            .audiences
            .iter()
            .any(|audience| audiences.contains(audience))
        {
            return Err(AppEndpointAuthError::Unauthorized);
        }
    }
    if !requirements.scopes.is_empty() {
        let token_scopes = claim_string_set(claims, "scope")
            .into_iter()
            .chain(claim_array_strings(claims, "scp"))
            .collect::<HashSet<_>>();
        if !requirements
            .scopes
            .iter()
            .all(|scope| token_scopes.contains(scope))
        {
            return Err(AppEndpointAuthError::Unauthorized);
        }
    }
    if !requirements.subjects.is_empty() {
        let subject = claims
            .get("sub")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !requirements
            .subjects
            .iter()
            .any(|allowed| allowed == subject)
        {
            return Err(AppEndpointAuthError::Unauthorized);
        }
    }
    if !requirements.domains.is_empty() {
        let domain = claims
            .get("hd")
            .and_then(Value::as_str)
            .or_else(|| email_domain(claims.get("email").and_then(Value::as_str)))
            .unwrap_or_default();
        if !requirements.domains.iter().any(|allowed| allowed == domain) {
            return Err(AppEndpointAuthError::Unauthorized);
        }
    }
    if !requirements.groups.is_empty() {
        let groups = claim_array_strings(claims, "groups");
        if !requirements
            .groups
            .iter()
            .any(|group| groups.contains(group))
        {
            return Err(AppEndpointAuthError::Unauthorized);
        }
    }
    for (name, expected) in &requirements.claims {
        if claims.get(name) != Some(expected) {
            return Err(AppEndpointAuthError::Unauthorized);
        }
    }
    Ok(())
}

fn claim_string_set(claims: &Value, name: &str) -> HashSet<String> {
    claims
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split_whitespace()
        .filter(|scope| !scope.is_empty())
        .map(str::to_string)
        .collect()
}

fn claim_array_strings(claims: &Value, name: &str) -> HashSet<String> {
    claims
        .get(name)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn claim_audience_set(claims: &Value) -> HashSet<String> {
    let mut audiences = claim_array_strings(claims, "aud");
    if let Some(audience) = claims.get("aud").and_then(Value::as_str) {
        audiences.insert(audience.to_string());
    }
    audiences
}

fn email_domain(email: Option<&str>) -> Option<&str> {
    email?.split_once('@').map(|(_, domain)| domain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::password::hash_password;
    use axum::http::HeaderValue;

    const EXAMPLE_USERNAME: &str = "example";
    const EXAMPLE_PASSWORD: &str = "YExample0";

    fn example_basic_auth_header() -> HeaderValue {
        let credentials = BASE64_STANDARD.encode(format!("{EXAMPLE_USERNAME}:{EXAMPLE_PASSWORD}"));
        HeaderValue::from_bytes(format!("basic {credentials}").as_bytes()).unwrap()
    }

    #[test]
    fn extracts_basic_credentials() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, example_basic_auth_header());
        assert_eq!(
            extract_basic_credentials(&headers),
            Some((EXAMPLE_USERNAME.to_string(), EXAMPLE_PASSWORD.to_string()))
        );
    }

    #[test]
    fn shared_secret_uses_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("bearer YExample0"));
        assert!(verify_shared_secret(&headers, EXAMPLE_PASSWORD).is_ok());
        assert_eq!(
            verify_shared_secret(&headers, "other").unwrap_err(),
            AppEndpointAuthError::Unauthorized
        );
    }

    #[test]
    fn basic_auth_verifies_argon2_password_hash() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, example_basic_auth_header());
        let auth = AppEndpointAuthConfig {
            mode: AppEndpointAuthMode::HttpBasic,
            provider: Some(AppEndpointAuthProviderConfig::HttpBasic {
                username: EXAMPLE_USERNAME.to_string(),
                password: None,
                password_hash: Some(hash_password(EXAMPLE_PASSWORD).unwrap()),
            }),
            requirements: AppEndpointAuthRequirements::default(),
        };
        assert!(
            AppEndpointAuthVerifier::new()
                .verify_basic(&auth, &headers)
                .is_ok()
        );
    }

    #[test]
    fn claim_requirements_check_scope_domain_and_claims() {
        let claims = serde_json::json!({
            "sub": "user-1",
            "email": "a@example.com",
            "scope": "read write",
            "tier": "prod"
        });
        let requirements = AppEndpointAuthRequirements {
            audiences: vec!["api://test".to_string()],
            scopes: vec!["read".to_string()],
            domains: vec!["example.com".to_string()],
            subjects: vec!["user-1".to_string()],
            claims: [("tier".to_string(), Value::String("prod".to_string()))]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        assert!(validate_claim_requirements(&claims, &requirements).is_err());

        let claims = serde_json::json!({
            "sub": "user-1",
            "email": "a@example.com",
            "aud": ["api://test"],
            "scope": "read write",
            "tier": "prod"
        });
        assert!(validate_claim_requirements(&claims, &requirements).is_ok());
    }

    fn mtls_auth() -> AppEndpointAuthConfig {
        AppEndpointAuthConfig {
            mode: AppEndpointAuthMode::Mtls,
            provider: Some(AppEndpointAuthProviderConfig::Mtls {
                header_name: "x-client-cert".to_string(),
                allowed_values: vec!["CN=trusted".to_string()],
                proxy_secret_header: Some("x-proxy-secret".to_string()),
                proxy_secret: Some("supersecret".to_string()),
            }),
            requirements: AppEndpointAuthRequirements::default(),
        }
    }

    #[test]
    fn mtls_requires_both_cert_and_proxy_secret() {
        let auth = mtls_auth();
        let verifier = AppEndpointAuthVerifier::new();

        // No headers — Unauthorized (cert missing).
        assert_eq!(
            verifier.verify_mtls(&auth, &HeaderMap::new()).unwrap_err(),
            AppEndpointAuthError::Unauthorized
        );

        // Cert header only — Unauthorized (proxy secret missing). This is the
        // spoofing case EVE-545 guards against.
        let mut headers = HeaderMap::new();
        headers.insert("x-client-cert", HeaderValue::from_static("CN=trusted"));
        assert_eq!(
            verifier.verify_mtls(&auth, &headers).unwrap_err(),
            AppEndpointAuthError::Unauthorized,
            "spoofed cert header alone must not authenticate"
        );

        // Cert + wrong proxy secret — Unauthorized.
        let mut headers = HeaderMap::new();
        headers.insert("x-client-cert", HeaderValue::from_static("CN=trusted"));
        headers.insert("x-proxy-secret", HeaderValue::from_static("wrongsecret"));
        assert_eq!(
            verifier.verify_mtls(&auth, &headers).unwrap_err(),
            AppEndpointAuthError::Unauthorized
        );

        // Both correct — Ok.
        let mut headers = HeaderMap::new();
        headers.insert("x-client-cert", HeaderValue::from_static("CN=trusted"));
        headers.insert("x-proxy-secret", HeaderValue::from_static("supersecret"));
        assert!(verifier.verify_mtls(&auth, &headers).is_ok());
    }

    #[test]
    fn mtls_without_proxy_secret_config_is_misconfigured() {
        // Legacy configs that predate EVE-545 (no proxy_secret fields) must fail
        // closed so they cannot be exploited after an upgrade.
        let auth = AppEndpointAuthConfig {
            mode: AppEndpointAuthMode::Mtls,
            provider: Some(AppEndpointAuthProviderConfig::Mtls {
                header_name: "x-client-cert".to_string(),
                allowed_values: vec!["CN=trusted".to_string()],
                proxy_secret_header: None,
                proxy_secret: None,
            }),
            requirements: AppEndpointAuthRequirements::default(),
        };
        let verifier = AppEndpointAuthVerifier::new();
        let mut headers = HeaderMap::new();
        headers.insert("x-client-cert", HeaderValue::from_static("CN=trusted"));
        assert_eq!(
            verifier.verify_mtls(&auth, &headers).unwrap_err(),
            AppEndpointAuthError::Misconfigured
        );
    }

    #[tokio::test]
    async fn resolve_and_validate_rejects_literal_loopback() {
        assert!(resolve_and_validate("http://127.0.0.1/").await.is_err());
        assert!(resolve_and_validate("http://[::1]/").await.is_err());
    }

    #[tokio::test]
    async fn resolve_and_validate_rejects_literal_private_ranges() {
        assert!(resolve_and_validate("http://10.0.0.1/").await.is_err());
        assert!(resolve_and_validate("http://192.168.1.1/").await.is_err());
        assert!(
            resolve_and_validate("http://169.254.169.254/")
                .await
                .is_err()
        );
    }
}

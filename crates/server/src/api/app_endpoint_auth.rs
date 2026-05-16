// Shared auth verifier for public App endpoint channels.
//
// Decision: endpoint auth config is inline on `app_channels.channel_config.auth`
// for the first product iteration. The verifier remains provider-shaped so the
// same runtime can later read org-level reusable providers without changing
// AG-UI/A2A ingress behavior.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderMap, header::AUTHORIZATION};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use everruns_core::{
    AppEndpointAuthConfig, AppEndpointAuthMode, AppEndpointAuthProviderConfig,
    AppEndpointAuthRequirements, is_blocked_ip, validate_safe_url,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use moka::future::Cache;
use serde::Deserialize;
use serde_json::Value;
use tokio::net::lookup_host;
use tokio::time::timeout;
use url::Host;

use crate::storage::password::verify_password;

#[derive(Clone)]
pub struct AppEndpointAuthVerifier {
    http: reqwest::Client,
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
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(5))
                .build()
                .expect("app endpoint auth HTTP client builds"),
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
        match auth.mode {
            AppEndpointAuthMode::Anonymous => Ok(()),
            AppEndpointAuthMode::SharedSecret => {
                let expected = legacy
                    .shared_secret
                    .ok_or(AppEndpointAuthError::Misconfigured)?;
                verify_shared_secret(headers, expected)
            }
            AppEndpointAuthMode::ApiKey => {
                let api_key = legacy.api_key.ok_or(AppEndpointAuthError::Misconfigured)?;
                verify_shared_secret(headers, api_key)
            }
            AppEndpointAuthMode::HttpBasic => self.verify_basic(auth, headers),
            AppEndpointAuthMode::GoogleOidc | AppEndpointAuthMode::Oidc => {
                self.verify_oidc(auth, headers).await
            }
            AppEndpointAuthMode::OAuth2Introspection => {
                self.verify_oauth2_introspection(auth, headers).await
            }
            AppEndpointAuthMode::Mtls => self.verify_mtls(auth, headers),
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
    ) -> Result<(), AppEndpointAuthError> {
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
        Ok(())
    }

    async fn verify_oauth2_introspection(
        &self,
        auth: &AppEndpointAuthConfig,
        headers: &HeaderMap,
    ) -> Result<(), AppEndpointAuthError> {
        let token = extract_bearer(headers).ok_or(AppEndpointAuthError::Unauthorized)?;
        let Some(AppEndpointAuthProviderConfig::OAuth2Introspection {
            introspection_url,
            client_id,
            client_secret,
        }) = auth.provider.as_ref()
        else {
            return Err(AppEndpointAuthError::Misconfigured);
        };
        validate_resolved_safe_url(introspection_url)
            .await
            .map_err(|_| AppEndpointAuthError::Misconfigured)?;
        let mut req = self.http.post(introspection_url).form(&[("token", token)]);
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
        Ok(())
    }

    fn verify_mtls(
        &self,
        auth: &AppEndpointAuthConfig,
        headers: &HeaderMap,
    ) -> Result<(), AppEndpointAuthError> {
        let Some(AppEndpointAuthProviderConfig::Mtls {
            header_name,
            allowed_values,
        }) = auth.provider.as_ref()
        else {
            return Err(AppEndpointAuthError::Misconfigured);
        };
        // THREAT[TM-AUTH-021]: mTLS identity is accepted only from a configured
        // reverse-proxy header. Deployments must strip this header at the public
        // edge and set it only after client-cert verification.
        let value = headers
            .get(header_name)
            .and_then(|value| value.to_str().ok())
            .ok_or(AppEndpointAuthError::Unauthorized)?;
        if allowed_values.iter().any(|allowed| allowed == value) {
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
        validate_resolved_safe_url(&issuer)
            .await
            .map_err(|_| AppEndpointAuthError::Misconfigured)?;
        let url = format!("{issuer}/.well-known/openid-configuration");
        validate_resolved_safe_url(&url)
            .await
            .map_err(|_| AppEndpointAuthError::Misconfigured)?;
        let discovery = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|_| AppEndpointAuthError::ProviderUnavailable)?
            .error_for_status()
            .map_err(|_| AppEndpointAuthError::ProviderUnavailable)?
            .json::<OidcDiscovery>()
            .await
            .map_err(|_| AppEndpointAuthError::ProviderUnavailable)?;
        validate_resolved_safe_url(&discovery.jwks_uri)
            .await
            .map_err(|_| AppEndpointAuthError::Misconfigured)?;
        self.discovery_cache.insert(issuer, discovery.clone()).await;
        Ok(discovery)
    }

    async fn jwks(&self, jwks_url: &str) -> Result<Arc<JwkSet>, AppEndpointAuthError> {
        if let Some(jwks) = self.jwks_cache.get(jwks_url).await {
            return Ok(jwks);
        }
        validate_resolved_safe_url(jwks_url)
            .await
            .map_err(|_| AppEndpointAuthError::Misconfigured)?;
        let jwks = self
            .http
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

fn normalize_issuer(issuer: &str) -> String {
    issuer.trim().trim_end_matches('/').to_string()
}

// Cap DNS resolution to the same budget as the outbound HTTP request so a slow
// or stuck resolver cannot stall auth verification past the overall timeout.
const DNS_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

async fn validate_resolved_safe_url(raw_url: &str) -> Result<(), ()> {
    let url = validate_safe_url(raw_url).map_err(|_| ())?;
    let host = url.host().ok_or(())?;
    if matches!(host, Host::Ipv4(_) | Host::Ipv6(_)) {
        return Ok(());
    }
    let port = url.port_or_known_default().ok_or(())?;
    let host = host.to_string();
    let resolved = timeout(DNS_LOOKUP_TIMEOUT, lookup_host((host.as_str(), port)))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
    let mut found = false;
    for addr in resolved {
        found = true;
        if is_blocked_ip(addr.ip()) {
            return Err(());
        }
    }
    if !found {
        return Err(());
    }
    Ok(())
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

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::password::hash_password;
    use axum::http::HeaderValue;

    #[test]
    fn extracts_basic_credentials() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("basic YWxpY2U6c2VjcmV0"),
        );
        assert_eq!(
            extract_basic_credentials(&headers),
            Some(("alice".to_string(), "secret".to_string()))
        );
    }

    #[test]
    fn shared_secret_uses_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("bearer secret"));
        assert!(verify_shared_secret(&headers, "secret").is_ok());
        assert_eq!(
            verify_shared_secret(&headers, "other").unwrap_err(),
            AppEndpointAuthError::Unauthorized
        );
    }

    #[test]
    fn basic_auth_verifies_argon2_password_hash() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("basic YWxpY2U6c2VjcmV0"),
        );
        let auth = AppEndpointAuthConfig {
            mode: AppEndpointAuthMode::HttpBasic,
            provider: Some(AppEndpointAuthProviderConfig::HttpBasic {
                username: "alice".to_string(),
                password: None,
                password_hash: Some(hash_password("secret").unwrap()),
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

    #[tokio::test]
    async fn validate_resolved_safe_url_rejects_literal_loopback() {
        assert!(
            validate_resolved_safe_url("http://127.0.0.1/")
                .await
                .is_err()
        );
        assert!(validate_resolved_safe_url("http://[::1]/").await.is_err());
    }

    #[tokio::test]
    async fn validate_resolved_safe_url_rejects_literal_private_ranges() {
        assert!(
            validate_resolved_safe_url("http://10.0.0.1/")
                .await
                .is_err()
        );
        assert!(
            validate_resolved_safe_url("http://192.168.1.1/")
                .await
                .is_err()
        );
        assert!(
            validate_resolved_safe_url("http://169.254.169.254/")
                .await
                .is_err()
        );
    }
}

//! Shared OAuth token-exchange client for MCP connection flows.

use crate::kernel_imports::{
    EgressRequest, EgressRequestKind, EgressResponse, EgressService, ScopedEgressService,
    contracts::url_validation::validate_url_dns_pinned,
};
use async_trait::async_trait;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// The refresh secret and its original audience travel in one encrypted value.
/// Plain legacy refresh tokens cannot prove a binding and require reauthorization.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct McpRefreshGrant {
    pub refresh_token: String,
    pub resource: String,
    pub config_hash: String,
}

/// Binds pending authorization and persisted refresh grants to the same server
/// and OAuth configuration, without putting client secrets in browser cookies.
pub(crate) fn mcp_oauth_config_hash(
    server_url: &str,
    oauth: &crate::domains::mcp_servers::McpServerOAuthSettings,
) -> String {
    use sha2::{Digest, Sha256};
    // Hash only binding inputs in a fixed order. Adding unrelated settings or
    // changing requested scopes must not invalidate every existing grant.
    // Secret/key rotation does not change the client identity or audience.
    // These concrete types cannot fail JSON serialization.
    hex::encode(Sha256::digest(
        serde_json::to_vec(&(
            server_url,
            &oauth.issuer,
            &oauth.authorization_endpoint,
            &oauth.token_endpoint,
            &oauth.client_id,
            &oauth.resource,
        ))
        .expect("OAuth settings JSON"),
    ))
}

impl McpRefreshGrant {
    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("MCP refresh grant JSON")
    }

    pub fn decode(value: &str, config_hash: &str) -> Option<Self> {
        let grant: Self = serde_json::from_str(value).ok()?;
        if grant.config_hash != config_hash || grant.resource.is_empty() {
            return None;
        }
        Some(grant)
    }
}

pub(crate) use everruns_core::mcp::oauth::protocol::TokenResponse as OAuthTokenResponse;

#[derive(Clone)]
pub(crate) struct OAuthRefreshRequest {
    pub token_endpoint: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub refresh_token: String,
    pub resource: Option<String>,
    /// Internal id of the org the connection belongs to; the refresh is
    /// attributed to it at the egress boundary.
    pub org_id: i64,
}

pub(crate) struct OAuthCodeExchangeRequest<'a> {
    pub token_endpoint: &'a str,
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
    pub redirect_uri: &'a str,
    pub code: &'a str,
    pub code_verifier: &'a str,
    pub resource: Option<&'a str>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OAuthRefreshError {
    InvalidGrant,
    Failed(StatusCode),
}

#[async_trait]
pub(crate) trait OAuthRefreshExchange: Send + Sync {
    async fn exchange(
        &self,
        request: OAuthRefreshRequest,
    ) -> Result<OAuthTokenResponse, OAuthRefreshError>;
}

pub(crate) struct EgressOAuthRefreshExchange {
    egress: Arc<dyn EgressService>,
}

impl EgressOAuthRefreshExchange {
    pub fn new(egress: Arc<dyn EgressService>) -> Self {
        Self { egress }
    }
}

#[async_trait]
impl OAuthRefreshExchange for EgressOAuthRefreshExchange {
    async fn exchange(
        &self,
        request: OAuthRefreshRequest,
    ) -> Result<OAuthTokenResponse, OAuthRefreshError> {
        let egress = org_scoped_egress(self.egress.clone(), request.org_id);
        exchange_oauth_refresh_token(
            egress.as_ref(),
            &request.token_endpoint,
            &request.client_id,
            request.client_secret.as_deref(),
            &request.refresh_token,
            request.resource.as_deref(),
        )
        .await
    }
}

/// `egress` with every request attributed to `org_id`.
///
/// OAuth sign-in runs outside a turn, so nothing upstream stamps a tenant on
/// its requests. Without one, the tenant egress proxy and the audit log see
/// discovery, client registration and token traffic as nobody's: no
/// per-org limits, allowlist or kill switch apply to it.
pub(crate) fn org_scoped_egress(
    egress: Arc<dyn EgressService>,
    org_id: i64,
) -> Arc<dyn EgressService> {
    Arc::new(ScopedEgressService::new(
        egress,
        crate::domains::mcp_servers::presentation::org_scope(org_id),
    ))
}

/// Server-only network policy: reject private destinations and pin the validated DNS result.
/// Protocol request construction and parsing live in core.
pub(crate) struct ServerOAuthEgress<'a>(pub &'a dyn EgressService);

#[async_trait]
impl EgressService for ServerOAuthEgress<'_> {
    async fn send(&self, request: EgressRequest) -> everruns_core::EgressResult<EgressResponse> {
        // THREAT[TM-TOOL-018]: every discovery, registration and token request is pinned.
        let (parsed, addresses) = validate_url_dns_pinned(&request.url)
            .await
            .map_err(|_| everruns_core::EgressError::InvalidRequest("blocked OAuth URL".into()))?;
        let request = if addresses.is_empty() {
            request.require_dns_pinning()
        } else {
            request.pinned_addrs(parsed.host_str().unwrap_or_default(), addresses)
        };
        self.0.send(request).await
    }
    async fn send_stream(
        &self,
        _: EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        Err(everruns_core::EgressError::InvalidRequest(
            "OAuth does not stream".into(),
        ))
    }
}

/// Translate safe protocol classifications into the Server's browser error contract.
pub(crate) fn oauth_error(error: everruns_core::mcp::oauth::OAuthError) -> (StatusCode, String) {
    use everruns_core::mcp::oauth::OAuthError;
    let status = match &error {
        OAuthError::Egress { message, .. } if message == "blocked_by_network_policy" => {
            return (
                StatusCode::BAD_GATEWAY,
                OAUTH_BLOCKED_BY_NETWORK_POLICY.into(),
            );
        }
        OAuthError::InsecureUrl { .. } => StatusCode::BAD_REQUEST,
        OAuthError::Egress { message, .. } if message == "blocked_url" => StatusCode::BAD_REQUEST,
        OAuthError::Http {
            what: "code exchange",
            ..
        } => StatusCode::BAD_REQUEST,
        _ => StatusCode::BAD_GATEWAY,
    };
    (status, error.to_string())
}

pub(crate) async fn exchange_oauth_code(
    egress: &dyn EgressService,
    request: OAuthCodeExchangeRequest<'_>,
) -> Result<OAuthTokenResponse, (StatusCode, String)> {
    let guarded = ServerOAuthEgress(egress);
    everruns_core::mcp::oauth::OAuthClient::new(&guarded, EgressRequestKind::Mcp)
        .exchange_code_response(
            request.token_endpoint,
            &everruns_core::mcp::oauth::RegisteredClient {
                client_id: request.client_id.into(),
                client_secret: request.client_secret.map(str::to_owned),
            },
            request.code,
            request.code_verifier,
            request.redirect_uri,
            request.resource,
        )
        .await
        .map_err(oauth_error)
}

async fn exchange_oauth_refresh_token(
    egress: &dyn EgressService,
    token_endpoint: &str,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
    resource: Option<&str>,
) -> Result<OAuthTokenResponse, OAuthRefreshError> {
    use everruns_core::mcp::oauth::{OAuthClient, OAuthError, RegisteredClient};
    let resource = resource
        .filter(|r| !r.is_empty())
        .ok_or(OAuthRefreshError::Failed(StatusCode::BAD_REQUEST))?;
    let guarded = ServerOAuthEgress(egress);
    OAuthClient::new(&guarded, EgressRequestKind::Mcp)
        .refresh_response(
            token_endpoint,
            &RegisteredClient {
                client_id: client_id.into(),
                client_secret: client_secret.map(str::to_owned),
            },
            refresh_token,
            resource,
        )
        .await
        .map_err(|error| match &error {
            OAuthError::Http { body, .. } if body == "invalid_grant" => {
                OAuthRefreshError::InvalidGrant
            }
            _ => OAuthRefreshError::Failed(oauth_error(error).0),
        })
}

/// Error message for an OAuth request the egress network policy refused.
/// Browser OAuth handlers match on it to choose `blocked_by_network_policy`.
pub(crate) const OAUTH_BLOCKED_BY_NETWORK_POLICY: &str =
    "The sign-in service's host is not on the organization's allowed network list";

impl std::fmt::Debug for OAuthRefreshRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthRefreshRequest")
            .field("credentials", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_binding_survives_secret_rotation_but_rejects_retargeting() {
        let mut oauth = crate::domains::mcp_servers::McpServerOAuthSettings {
            token_endpoint: Some("https://auth.example/token".into()),
            client_id: Some("client".into()),
            resource: Some("https://mcp.example/resource".into()),
            ..Default::default()
        };
        let binding = mcp_oauth_config_hash("https://mcp.example/mcp", &oauth);
        oauth.client_secret_encrypted = Some("rotated-ciphertext".into());
        oauth.scope = Some("new-requested-scope".into());
        assert_eq!(
            binding,
            mcp_oauth_config_hash("https://mcp.example/mcp", &oauth)
        );
        oauth.resource = Some("https://mcp.example/another-resource".into());
        assert_ne!(
            binding,
            mcp_oauth_config_hash("https://mcp.example/mcp", &oauth)
        );
    }

    use std::collections::BTreeMap;

    struct InvalidGrantEgress;

    #[async_trait]
    impl EgressService for InvalidGrantEgress {
        async fn send(
            &self,
            _request: EgressRequest,
        ) -> everruns_core::EgressResult<EgressResponse> {
            Ok(EgressResponse {
                status: 400,
                headers: BTreeMap::new(),
                body: br#"{"error":"invalid_grant","error_description":"expired"}"#.to_vec(),
            })
        }

        async fn send_stream(
            &self,
            _request: EgressRequest,
        ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
            panic!("streaming egress is not used by OAuth refresh")
        }
    }

    #[tokio::test]
    async fn refresh_classifies_invalid_grant_without_exposing_provider_body() {
        let error = exchange_oauth_refresh_token(
            &InvalidGrantEgress,
            "https://8.8.8.8/token",
            "client",
            None,
            "refresh",
            Some("https://8.8.8.8/mcp"),
        )
        .await
        .unwrap_err();

        assert_eq!(error, OAuthRefreshError::InvalidGrant);
    }

    struct RecordingEgress(std::sync::Mutex<Vec<EgressRequest>>);

    #[async_trait]
    impl EgressService for RecordingEgress {
        async fn send(
            &self,
            request: EgressRequest,
        ) -> everruns_core::EgressResult<EgressResponse> {
            self.0.lock().unwrap().push(request);
            Ok(EgressResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: br#"{"access_token":"fresh"}"#.to_vec(),
            })
        }

        async fn send_stream(
            &self,
            _request: EgressRequest,
        ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
            panic!("streaming egress is not used by OAuth refresh")
        }
    }

    #[tokio::test]
    async fn refresh_is_attributed_to_the_connections_org() {
        let egress = Arc::new(RecordingEgress(Default::default()));
        let exchange = EgressOAuthRefreshExchange::new(egress.clone());

        let token = exchange
            .exchange(OAuthRefreshRequest {
                token_endpoint: "https://8.8.8.8/token".to_string(),
                client_id: "client".to_string(),
                client_secret: None,
                refresh_token: "refresh".to_string(),
                resource: Some("https://8.8.8.8/mcp".into()),
                org_id: 7,
            })
            .await
            .unwrap();

        assert_eq!(token.access_token, "fresh");
        let sent = egress.0.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            sent[0].scope,
            Some(crate::domains::mcp_servers::presentation::org_scope(7))
        );
        assert!(sent[0].scope.as_ref().unwrap().org_id.is_some());
    }
}

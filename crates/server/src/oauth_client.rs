//! Shared OAuth token-exchange client for MCP connection flows.

use crate::kernel_imports::{
    EgressRequest, EgressRequestKind, EgressService,
    everruns_provider::url_validation::validate_safe_url,
    everruns_provider::url_validation::validate_url_dns_pinned,
};
use async_trait::async_trait;
use axum::http::StatusCode;
use serde::Deserialize;
use std::sync::Arc;

use crate::api::common::sanitized_bad_gateway;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct OAuthTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct OAuthRefreshRequest {
    pub token_endpoint: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub refresh_token: String,
}

#[async_trait]
pub(crate) trait OAuthRefreshExchange: Send + Sync {
    async fn exchange(
        &self,
        request: OAuthRefreshRequest,
    ) -> Result<OAuthTokenResponse, (StatusCode, String)>;
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
    ) -> Result<OAuthTokenResponse, (StatusCode, String)> {
        exchange_oauth_refresh_token(
            self.egress.as_ref(),
            &request.token_endpoint,
            &request.client_id,
            request.client_secret.as_deref(),
            &request.refresh_token,
        )
        .await
    }
}

pub(crate) async fn exchange_oauth_code(
    egress: &dyn EgressService,
    token_endpoint: &str,
    client_id: &str,
    client_secret: Option<&str>,
    redirect_uri: &str,
    code: &str,
    code_verifier: &str,
) -> Result<OAuthTokenResponse, (StatusCode, String)> {
    let mut params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("client_id", client_id.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("code", code.to_string()),
        ("code_verifier", code_verifier.to_string()),
    ];
    if let Some(secret) = client_secret {
        params.push(("client_secret", secret.to_string()));
    }
    exchange_oauth_token(egress, token_endpoint, params).await
}

async fn exchange_oauth_refresh_token(
    egress: &dyn EgressService,
    token_endpoint: &str,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
) -> Result<OAuthTokenResponse, (StatusCode, String)> {
    let mut params = vec![
        ("grant_type", "refresh_token".to_string()),
        ("client_id", client_id.to_string()),
        ("refresh_token", refresh_token.to_string()),
    ];
    if let Some(secret) = client_secret {
        params.push(("client_secret", secret.to_string()));
    }
    exchange_oauth_token(egress, token_endpoint, params).await
}

async fn exchange_oauth_token(
    egress: &dyn EgressService,
    token_endpoint: &str,
    params: Vec<(&str, String)>,
) -> Result<OAuthTokenResponse, (StatusCode, String)> {
    validate_safe_url(token_endpoint).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Token endpoint blocked: {e}"),
        )
    })?;
    let body = serde_urlencoded::to_string(&params)
        .map_err(|e| sanitized_bad_gateway("OAuth token body", &e))?
        .into_bytes();
    egress_oauth_json(
        egress,
        "POST",
        token_endpoint,
        &[(
            "Content-Type",
            "application/x-www-form-urlencoded".to_string(),
        )],
        body,
    )
    .await
}

/// Send an OAuth request through the host egress boundary with DNS pinning.
pub(crate) async fn egress_oauth_json<T: serde::de::DeserializeOwned>(
    egress: &dyn EgressService,
    method: &str,
    url: &str,
    headers: &[(&str, String)],
    body: Vec<u8>,
) -> Result<T, (StatusCode, String)> {
    // THREAT[TM-TOOL-018]: bind the outbound connection to the IPs validated
    // for this request, including token refreshes of long-lived credentials.
    let (parsed, pinned_addrs) = validate_url_dns_pinned(url)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Blocked URL: {e}")))?;
    let host = parsed.host_str().unwrap_or("").to_string();

    let mut request = EgressRequest::new(method, url, EgressRequestKind::Mcp);
    for (name, value) in headers {
        request = request.header(*name, value.clone());
    }
    if !body.is_empty() {
        request = request.body(body);
    }
    request = if pinned_addrs.is_empty() {
        request.require_dns_pinning()
    } else {
        request.pinned_addrs(host, pinned_addrs)
    };

    let response = egress
        .send(request)
        .await
        .map_err(|e| sanitized_bad_gateway("OAuth external service", &e))?;

    if !(200..300).contains(&response.status) {
        tracing::warn!(
            status = response.status,
            body_len = response.body.len(),
            "OAuth external service rejected request"
        );
        return Err((StatusCode::BAD_GATEWAY, "Bad gateway".to_string()));
    }

    serde_json::from_slice(&response.body)
        .map_err(|e| sanitized_bad_gateway("External service response parse", &e))
}

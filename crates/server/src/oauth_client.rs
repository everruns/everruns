//! Shared OAuth token-exchange client for MCP connection flows.

use crate::kernel_imports::{
    EgressRequest, EgressRequestKind, EgressResponse, EgressService,
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
    pub resource: Option<String>,
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
        exchange_oauth_refresh_token(
            self.egress.as_ref(),
            &request.token_endpoint,
            &request.client_id,
            request.client_secret.as_deref(),
            &request.refresh_token,
            request.resource.as_deref(),
        )
        .await
    }
}

pub(crate) async fn exchange_oauth_code(
    egress: &dyn EgressService,
    request: OAuthCodeExchangeRequest<'_>,
) -> Result<OAuthTokenResponse, (StatusCode, String)> {
    let mut params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("client_id", request.client_id.to_string()),
        ("redirect_uri", request.redirect_uri.to_string()),
        ("code", request.code.to_string()),
        ("code_verifier", request.code_verifier.to_string()),
    ];
    if let Some(secret) = request.client_secret {
        params.push(("client_secret", secret.to_string()));
    }
    if let Some(resource) = request.resource {
        params.push(("resource", resource.to_string()));
    }
    exchange_oauth_token(egress, request.token_endpoint, params).await
}

async fn exchange_oauth_refresh_token(
    egress: &dyn EgressService,
    token_endpoint: &str,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
    resource: Option<&str>,
) -> Result<OAuthTokenResponse, OAuthRefreshError> {
    let mut params = vec![
        ("grant_type", "refresh_token".to_string()),
        ("client_id", client_id.to_string()),
        ("refresh_token", refresh_token.to_string()),
    ];
    if let Some(secret) = client_secret {
        params.push(("client_secret", secret.to_string()));
    }
    if let Some(resource) = resource {
        params.push(("resource", resource.to_string()));
    }
    validate_safe_url(token_endpoint)
        .map_err(|_| OAuthRefreshError::Failed(StatusCode::BAD_REQUEST))?;
    let body = serde_urlencoded::to_string(&params)
        .map_err(|_| OAuthRefreshError::Failed(StatusCode::BAD_GATEWAY))?
        .into_bytes();
    let response = send_oauth_request(
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
    .map_err(|(status, _)| OAuthRefreshError::Failed(status))?;
    if !(200..300).contains(&response.status) {
        tracing::warn!(
            status = response.status,
            body_len = response.body.len(),
            "OAuth external service rejected refresh request"
        );
        let invalid_grant = serde_json::from_slice::<serde_json::Value>(&response.body)
            .ok()
            .and_then(|body| {
                body.get("error")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned)
            })
            .is_some_and(|error| error == "invalid_grant");
        return Err(if invalid_grant {
            OAuthRefreshError::InvalidGrant
        } else {
            OAuthRefreshError::Failed(StatusCode::BAD_GATEWAY)
        });
    }
    serde_json::from_slice(&response.body)
        .map_err(|_| OAuthRefreshError::Failed(StatusCode::BAD_GATEWAY))
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
    let response = send_oauth_request(
        egress,
        "POST",
        token_endpoint,
        &[(
            "Content-Type",
            "application/x-www-form-urlencoded".to_string(),
        )],
        body,
    )
    .await?;
    if !(200..300).contains(&response.status) {
        #[derive(Deserialize)]
        struct OAuthErrorResponse {
            error: String,
            #[serde(default)]
            error_description: Option<String>,
        }

        tracing::warn!(
            status = response.status,
            body_len = response.body.len(),
            "OAuth external service rejected token request"
        );
        let message = serde_json::from_slice::<OAuthErrorResponse>(&response.body)
            .ok()
            .map(|response| {
                let error = sanitized_oauth_error_text(&response.error);
                match response
                    .error_description
                    .as_deref()
                    .map(sanitized_oauth_error_text)
                    .filter(|description| !description.is_empty())
                {
                    Some(description) => {
                        format!("OAuth token request was refused: {error} ({description})")
                    }
                    None => format!("OAuth token request was refused: {error}"),
                }
            })
            .unwrap_or_else(|| "OAuth token request was refused".to_string());
        return Err((StatusCode::BAD_REQUEST, message));
    }
    serde_json::from_slice(&response.body)
        .map_err(|e| sanitized_bad_gateway("External service response parse", &e))
}

fn sanitized_oauth_error_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

/// Send an OAuth request through the host egress boundary with DNS pinning.
pub(crate) async fn egress_oauth_json<T: serde::de::DeserializeOwned>(
    egress: &dyn EgressService,
    method: &str,
    url: &str,
    headers: &[(&str, String)],
    body: Vec<u8>,
) -> Result<T, (StatusCode, String)> {
    let response = send_oauth_request(egress, method, url, headers, body).await?;
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

async fn send_oauth_request(
    egress: &dyn EgressService,
    method: &str,
    url: &str,
    headers: &[(&str, String)],
    body: Vec<u8>,
) -> Result<EgressResponse, (StatusCode, String)> {
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

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
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
            None,
        )
        .await
        .unwrap_err();

        assert_eq!(error, OAuthRefreshError::InvalidGrant);
    }
}

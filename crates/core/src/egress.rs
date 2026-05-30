//! Host-owned outbound network boundary.
//!
//! `EgressService` is the runtime contract for traffic that leaves an
//! Everruns deployment. Provider drivers, capabilities, toolkit integrations,
//! and system services should depend on this trait instead of constructing
//! transport clients directly.

use crate::network_access::NetworkAccessList;
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::pin::Pin;
use std::time::Duration;
use thiserror::Error;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

pub type EgressResult<T> = std::result::Result<T, EgressError>;
pub type EgressByteStream = Pin<Box<dyn Stream<Item = EgressResult<Vec<u8>>> + Send>>;

#[derive(Debug, Error)]
pub enum EgressError {
    #[error("Invalid egress request: {0}")]
    InvalidRequest(String),

    #[error("Outbound request blocked by network access policy: {url}")]
    NetworkAccessDenied { url: String },

    #[error("Outbound request signing is not configured")]
    SigningUnavailable,

    #[error("Outbound transport error: {0}")]
    Transport(String),
}

impl EgressError {
    fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidRequest(message.into())
    }
}

/// Logical owner of an outbound request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EgressRequestKind {
    LlmProvider,
    Capability,
    Integration,
    SystemEmail,
    UtilityLlm,
    Mcp,
    Other(String),
}

/// Request signing behavior requested by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EgressSigning {
    Disabled,
    PlatformDefault,
    Required,
}

/// Provider-neutral HTTP request carried over the egress boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressRequest {
    pub method: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body: Vec<u8>,
    pub kind: EgressRequestKind,
    #[serde(default = "default_signing")]
    pub signing: EgressSigning,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_access: Option<NetworkAccessList>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

impl EgressRequest {
    pub fn new(method: impl Into<String>, url: impl Into<String>, kind: EgressRequestKind) -> Self {
        Self {
            method: method.into(),
            url: url.into(),
            headers: BTreeMap::new(),
            body: Vec::new(),
            kind,
            signing: EgressSigning::Disabled,
            network_access: None,
            timeout_ms: None,
        }
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn signing(mut self, signing: EgressSigning) -> Self {
        self.signing = signing;
        self
    }

    pub fn network_access(mut self, network_access: Option<NetworkAccessList>) -> Self {
        self.network_access = network_access;
        self
    }

    pub fn timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }
}

/// Provider-neutral HTTP response returned by the egress boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub struct EgressStreamResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: EgressByteStream,
}

#[async_trait]
pub trait EgressService: Send + Sync {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse>;

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse>;

    fn name(&self) -> &'static str {
        "EgressService"
    }
}

#[derive(Debug, Clone, Default)]
pub struct DisabledEgressService;

#[async_trait]
impl EgressService for DisabledEgressService {
    async fn send(&self, _request: EgressRequest) -> EgressResult<EgressResponse> {
        Err(EgressError::Transport(
            "outbound egress service is disabled".to_string(),
        ))
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        Err(EgressError::Transport(
            "outbound egress service is disabled".to_string(),
        ))
    }

    fn name(&self) -> &'static str {
        "DisabledEgressService"
    }
}

#[derive(Clone)]
pub struct DirectEgressService {
    client: reqwest::Client,
}

impl std::fmt::Debug for DirectEgressService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectEgressService")
            .finish_non_exhaustive()
    }
}

impl Default for DirectEgressService {
    fn default() -> Self {
        Self::new()
    }
}

impl DirectEgressService {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
                .timeout(DEFAULT_REQUEST_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("build direct egress HTTP client"),
        }
    }

    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }

    fn validate_request(request: &EgressRequest) -> EgressResult<()> {
        if request.method.trim().is_empty() {
            return Err(EgressError::invalid("method is required"));
        }
        let parsed = reqwest::Url::parse(&request.url)
            .map_err(|error| EgressError::invalid(format!("invalid URL: {error}")))?;
        match parsed.scheme() {
            "http" | "https" => {}
            scheme => {
                return Err(EgressError::invalid(format!(
                    "URL must use http or https, got '{scheme}'"
                )));
            }
        }
        if let Some(acl) = &request.network_access
            && !acl.is_url_allowed(&request.url)
        {
            return Err(EgressError::NetworkAccessDenied {
                url: request.url.clone(),
            });
        }
        Ok(())
    }

    fn build_request(&self, request: EgressRequest) -> EgressResult<reqwest::RequestBuilder> {
        Self::validate_request(&request)?;
        if request.signing == EgressSigning::Required {
            return Err(EgressError::SigningUnavailable);
        }

        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| EgressError::invalid(format!("invalid HTTP method: {error}")))?;
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        if let Some(timeout_ms) = request.timeout_ms {
            builder = builder.timeout(Duration::from_millis(timeout_ms));
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body);
        }
        Ok(builder)
    }
}

#[async_trait]
impl EgressService for DirectEgressService {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let response = self
            .build_request(request)?
            .send()
            .await
            .map_err(|error| EgressError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_string(), value.to_string()))
            })
            .collect();
        let body = response
            .bytes()
            .await
            .map_err(|error| EgressError::Transport(error.to_string()))?
            .to_vec();

        Ok(EgressResponse {
            status,
            headers,
            body,
        })
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        let response = self
            .build_request(request)?
            .send()
            .await
            .map_err(|error| EgressError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_string(), value.to_string()))
            })
            .collect();
        let body = response.bytes_stream().map(|chunk| {
            chunk
                .map(|bytes| bytes.to_vec())
                .map_err(|error| EgressError::Transport(error.to_string()))
        });

        Ok(EgressStreamResponse {
            status,
            headers,
            body: Box::pin(body),
        })
    }

    fn name(&self) -> &'static str {
        "DirectEgressService"
    }
}

fn default_signing() -> EgressSigning {
    EgressSigning::Disabled
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn direct_service_sends_json_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/test"))
            .and(header("Authorization", "Bearer test"))
            .and(body_json(json!({"ok": true})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id": "response_123"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let response = DirectEgressService::new()
            .send(
                EgressRequest::new(
                    "POST",
                    format!("{}/v1/test", server.uri()),
                    EgressRequestKind::Capability,
                )
                .header("Authorization", "Bearer test")
                .header("Content-Type", "application/json")
                .body(serde_json::to_vec(&json!({"ok": true})).unwrap()),
            )
            .await
            .unwrap();

        assert_eq!(response.status, 201);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response.body).unwrap()["id"],
            "response_123"
        );
    }

    #[tokio::test]
    async fn direct_service_enforces_network_access() {
        let error = DirectEgressService::new()
            .send(
                EgressRequest::new(
                    "GET",
                    "https://blocked.example.com/path",
                    EgressRequestKind::Capability,
                )
                .network_access(Some(NetworkAccessList::allow_only(["allowed.example.com"]))),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
    }

    #[tokio::test]
    async fn required_signing_fails_when_no_signer_is_configured() {
        let error = DirectEgressService::new()
            .send(
                EgressRequest::new("GET", "https://example.com", EgressRequestKind::Capability)
                    .signing(EgressSigning::Required),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, EgressError::SigningUnavailable));
    }

    #[tokio::test]
    async fn direct_service_does_not_follow_redirects() {
        let redirect_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/secret"))
            .respond_with(ResponseTemplate::new(200).set_body_string("secret"))
            .expect(0)
            .mount(&redirect_server)
            .await;

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/secret", redirect_server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;

        let response = DirectEgressService::new()
            .send(EgressRequest::new(
                "GET",
                format!("{}/start", server.uri()),
                EgressRequestKind::Capability,
            ))
            .await
            .unwrap();

        assert_eq!(response.status, 302);
    }

    #[tokio::test]
    async fn direct_service_streams_response_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/stream"))
            .respond_with(ResponseTemplate::new(200).set_body_string("data: one\n\ndata: two\n\n"))
            .expect(1)
            .mount(&server)
            .await;

        let mut response = DirectEgressService::new()
            .send_stream(EgressRequest::new(
                "GET",
                format!("{}/stream", server.uri()),
                EgressRequestKind::LlmProvider,
            ))
            .await
            .unwrap();

        assert_eq!(response.status, 200);
        let mut body = Vec::new();
        while let Some(chunk) = response.body.next().await {
            body.extend(chunk.unwrap());
        }
        assert_eq!(
            String::from_utf8(body).unwrap(),
            "data: one\n\ndata: two\n\n"
        );
    }
}

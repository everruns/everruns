//! Host-owned outbound network boundary.
//!
//! `EgressService` is the runtime contract for traffic that leaves an
//! Everruns deployment. Provider drivers, capabilities, toolkit integrations,
//! and system services should depend on this trait instead of constructing
//! transport clients directly.

use crate::network_access::NetworkAccessList;
use crate::system_allowlist::SystemAllowlist;
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;
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
    /// Pre-resolved socket addresses for DNS pinning (TM-TOOL-018).
    ///
    /// When set, `DirectEgressService` builds a per-request client pinned to
    /// these addresses via `resolve_to_addrs`, closing the TOCTOU window
    /// between URL validation and the actual TCP connect.  Obtained from
    /// `validate_url_dns_pinned`.  Not serialized — runtime-only hint.
    #[serde(skip)]
    pub pinned_addrs: Option<(String, Vec<std::net::SocketAddr>)>,
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
            pinned_addrs: None,
        }
    }

    /// Pin the outbound connection to pre-resolved addresses (TM-TOOL-018).
    ///
    /// No-op when `addrs` is empty (e.g. IP-literal URLs where the static
    /// check already validated the address).
    pub fn pinned_addrs(
        mut self,
        host: impl Into<String>,
        addrs: Vec<std::net::SocketAddr>,
    ) -> Self {
        if !addrs.is_empty() {
            self.pinned_addrs = Some((host.into(), addrs));
        }
        self
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
    /// Optional host-wide allowlist applied to every outbound request,
    /// independently of the per-request `network_access`. `None` means no
    /// global enforcement. See `crate::system_allowlist`.
    system_allowlist: Option<Arc<SystemAllowlist>>,
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
            system_allowlist: None,
        }
    }

    pub fn with_client(client: reqwest::Client) -> Self {
        Self {
            client,
            system_allowlist: None,
        }
    }

    /// Construct with the global system allowlist resolved from the environment.
    ///
    /// Enforcement is active only when `EVERRUNS_SYSTEM_ALLOWLIST_ENABLED` is
    /// set; otherwise this behaves exactly like [`DirectEgressService::new`].
    pub fn from_env() -> Self {
        Self::new().with_system_allowlist(SystemAllowlist::from_env())
    }

    /// Attach (or clear) the host-wide system allowlist.
    pub fn with_system_allowlist(mut self, system_allowlist: Option<Arc<SystemAllowlist>>) -> Self {
        self.system_allowlist = system_allowlist;
        self
    }

    fn validate_request(&self, request: &EgressRequest) -> EgressResult<()> {
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
        // Host-wide allowlist: when enabled, every outbound request must match
        // one of the curated public resources, regardless of request kind or the
        // per-request network access list. This is a separate, AND-ed gate — it
        // is never merged into `request.network_access`, so harness/agent/session
        // allowlists can only narrow within it and can never widen past it. The
        // system allowlist is a hard ceiling with maximum priority.
        if let Some(allowlist) = &self.system_allowlist
            && !allowlist.is_url_allowed(&request.url)
        {
            return Err(EgressError::NetworkAccessDenied {
                url: request.url.clone(),
            });
        }
        Ok(())
    }

    fn build_request(&self, request: EgressRequest) -> EgressResult<reqwest::RequestBuilder> {
        self.validate_request(&request)?;
        if request.signing == EgressSigning::Required {
            return Err(EgressError::SigningUnavailable);
        }

        let EgressRequest {
            method,
            url,
            headers,
            body,
            timeout_ms,
            pinned_addrs,
            ..
        } = request;

        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|error| EgressError::invalid(format!("invalid HTTP method: {error}")))?;

        // When pre-resolved addresses are provided, build a per-request client
        // pinned to those IPs.  This closes the TOCTOU window between
        // validate_url_dns_pinned and the actual TCP connect (TM-TOOL-018).
        let mut builder = if let Some((ref host, ref addrs)) = pinned_addrs {
            reqwest::Client::builder()
                .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .resolve_to_addrs(host, addrs)
                .build()
                .map_err(|e| EgressError::invalid(format!("pinned client build failed: {e}")))?
                .request(method, &url)
        } else {
            self.client.request(method, &url)
        };

        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        if let Some(timeout_ms) = timeout_ms {
            builder = builder.timeout(Duration::from_millis(timeout_ms));
        }
        if !body.is_empty() {
            builder = builder.body(body);
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
    async fn system_allowlist_blocks_unlisted_hosts() {
        use crate::system_allowlist::SystemAllowlist;
        let allowlist = SystemAllowlist::from_toml(
            r#"
            [groups.test]
            allowed = ["allowed.example.com"]
            "#,
        )
        .unwrap();

        let service = DirectEgressService::new().with_system_allowlist(Some(Arc::new(allowlist)));
        let error = service
            .send(EgressRequest::new(
                "GET",
                "https://blocked.example.com/path",
                // Even system-owned traffic (no per-request network_access) is
                // subject to the host-wide allowlist.
                EgressRequestKind::LlmProvider,
            ))
            .await
            .unwrap_err();

        assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
    }

    #[tokio::test]
    async fn system_allowlist_cannot_be_overridden_by_request_acl() {
        use crate::system_allowlist::SystemAllowlist;
        // The system allowlist is a hard ceiling: even a request whose own
        // network_access explicitly permits the host must still be denied when
        // the system allowlist does not list it. Sessions can narrow, never widen.
        let allowlist = SystemAllowlist::from_toml(
            r#"
            [groups.test]
            allowed = ["allowed.example.com"]
            "#,
        )
        .unwrap();

        let service = DirectEgressService::new().with_system_allowlist(Some(Arc::new(allowlist)));
        let error = service
            .send(
                EgressRequest::new(
                    "GET",
                    "https://blocked.example.com/path",
                    EgressRequestKind::Capability,
                )
                // A maximally-permissive per-request ACL that allows the host.
                .network_access(Some(NetworkAccessList::allow_only(["blocked.example.com"]))),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
    }

    #[tokio::test]
    async fn system_allowlist_permits_listed_hosts() {
        use crate::system_allowlist::SystemAllowlist;
        let server = MockServer::start().await;
        let host = reqwest::Url::parse(&server.uri())
            .unwrap()
            .host_str()
            .unwrap()
            .to_string();
        Mock::given(method("GET"))
            .and(path("/ok"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let allowlist =
            SystemAllowlist::from_toml(&format!("[groups.test]\nallowed = [\"{host}\"]\n"))
                .unwrap();
        let service = DirectEgressService::new().with_system_allowlist(Some(Arc::new(allowlist)));
        let response = service
            .send(EgressRequest::new(
                "GET",
                format!("{}/ok", server.uri()),
                EgressRequestKind::Capability,
            ))
            .await
            .unwrap();

        assert_eq!(response.status, 200);
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

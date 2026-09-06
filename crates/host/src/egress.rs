//! Reqwest-backed implementation of the neutral Everruns egress contract.

use async_trait::async_trait;
use everruns_core::{
    EgressError, EgressRequest, EgressResponse, EgressResult, EgressService, EgressSigning,
    EgressStreamResponse, SystemAllowlist,
};
use everruns_provider::url_validation::validate_url_dns_pinned;
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct DirectEgressService {
    client: reqwest::Client,
    /// Optional host-wide allowlist applied to every outbound request,
    /// independently of the per-request `network_access`. `None` means no
    /// global enforcement. See `everruns_core::system_allowlist`.
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

    /// Construct the default direct transport for tenant/agent runtime egress.
    ///
    /// This honors `EVERRUNS_SYSTEM_ALLOWLIST_ENABLED` and is the right default
    /// for capabilities, MCP, integrations, and runtime HTTP surfaces.
    pub fn for_runtime_traffic_from_env() -> Self {
        Self::from_env()
    }

    /// Construct with the global system allowlist resolved from the environment.
    ///
    /// Enforcement is active only when `EVERRUNS_SYSTEM_ALLOWLIST_ENABLED` is
    /// set; otherwise this behaves exactly like [`DirectEgressService::new`].
    /// Prefer [`DirectEgressService::for_runtime_traffic_from_env`] for new
    /// runtime/agent call sites. Host-owned services should use direct provider
    /// clients instead of `EgressService`.
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
        // Host-wide allowlist: when enabled, every request routed through this
        // runtime egress boundary must match one of the curated public
        // resources. Host-owned services should not use `EgressService`.
        if let Some(allowlist) = &self.system_allowlist
            && !allowlist.is_url_allowed(&request.url)
        {
            return Err(EgressError::NetworkAccessDenied {
                url: request.url.clone(),
            });
        }
        Ok(())
    }

    async fn prepare_request(&self, mut request: EgressRequest) -> EgressResult<EgressRequest> {
        self.validate_request(&request)?;
        if request.signing == EgressSigning::Required {
            return Err(EgressError::SigningUnavailable);
        }
        if request.dns_pinning_required {
            let (validated_url, resolved_addrs) = validate_url_dns_pinned(&request.url)
                .await
                .map_err(|error| EgressError::NetworkAccessDenied {
                    url: format!("{} ({error})", request.url),
                })?;
            let pin_host = validated_url.host_str().unwrap_or("").to_string();
            request = request.pinned_addrs(pin_host, resolved_addrs);
        }
        Ok(request)
    }

    fn build_request(&self, request: EgressRequest) -> EgressResult<reqwest::RequestBuilder> {
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
        let request = self.prepare_request(request).await?;
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
        let request = self.prepare_request(request).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::EgressRequestKind;
    use everruns_core::network_access::NetworkAccessList;
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
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({
                        "id": "response_123"
                    }))
                    .insert_header("X-Request-Id", "request-42"),
            )
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
            response.headers.get("x-request-id").map(String::as_str),
            Some("request-42")
        );
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
        use everruns_core::SystemAllowlist;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/blocked"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
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
                format!("{}/blocked", server.uri()),
                EgressRequestKind::Capability,
            ))
            .await
            .unwrap_err();

        assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
    }

    #[tokio::test]
    async fn system_allowlist_cannot_be_overridden_by_request_acl() {
        use everruns_core::SystemAllowlist;
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
        use everruns_core::SystemAllowlist;
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
    async fn dns_pinning_blocks_loopback_after_request_policy_passes() {
        let error = DirectEgressService::new()
            .send(
                EgressRequest::new(
                    "GET",
                    "http://127.0.0.1/latest/meta-data",
                    EgressRequestKind::Capability,
                )
                .network_access(Some(NetworkAccessList::allow_only(["127.0.0.1"])))
                .require_dns_pinning(),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, EgressError::NetworkAccessDenied { .. }));
        assert!(error.to_string().contains("private/internal address"));
    }

    #[tokio::test]
    async fn dns_pinning_does_not_run_before_system_allowlist_denial() {
        use everruns_core::SystemAllowlist;
        let allowlist = SystemAllowlist::from_toml(
            r#"
            [groups.test]
            allowed = ["allowed.example.com"]
            "#,
        )
        .unwrap();

        let blocked_url = "https://blocked.invalid/path";
        let error = DirectEgressService::new()
            .with_system_allowlist(Some(Arc::new(allowlist)))
            .send(
                EgressRequest::new("GET", blocked_url, EgressRequestKind::Capability)
                    .network_access(Some(NetworkAccessList::allow_only(["blocked.invalid"])))
                    .require_dns_pinning(),
            )
            .await
            .unwrap_err();

        assert!(
            matches!(error, EgressError::NetworkAccessDenied { ref url } if url == blocked_url)
        );
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
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw("data: one\n\ndata: two\n\n", "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let mut response = DirectEgressService::new()
            .send_stream(EgressRequest::new(
                "GET",
                format!("{}/stream", server.uri()),
                EgressRequestKind::Capability,
            ))
            .await
            .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(
            response.headers.get("content-type").map(String::as_str),
            Some("text/event-stream")
        );
        let mut body = Vec::new();
        while let Some(chunk) = response.body.next().await {
            body.extend(chunk.unwrap());
        }
        assert_eq!(
            String::from_utf8(body).unwrap(),
            "data: one\n\ndata: two\n\n"
        );
    }
    #[tokio::test]
    async fn merged_network_policy_denies_escaped_urls_before_transport() {
        use everruns_core::network_access::merge_network_access;
        let service = DirectEgressService::new();
        for (parent, child, target) in [
            (
                "*.example.com",
                "https://outside.invalid/path.example.com",
                "https://outside.invalid/path.example.com",
            ),
            (
                "https://api.example.com/v1/",
                "https://api.example.com/v1/../admin",
                "https://api.example.com/admin",
            ),
            (
                "https://",
                "https://outside.invalid/data",
                "https://outside.invalid/data",
            ),
        ] {
            let parent = NetworkAccessList::allow_only([parent]);
            let child = NetworkAccessList::allow_only([child]);
            let policy = merge_network_access(Some(&parent), Some(&child));
            for streaming in [false, true] {
                let request = EgressRequest::new("GET", target, EgressRequestKind::Capability)
                    .network_access(policy.clone());
                let error = if streaming {
                    match service.send_stream(request).await {
                        Err(error) => error,
                        Ok(_) => panic!("streaming request escaped policy: {target}"),
                    }
                } else {
                    service.send(request).await.unwrap_err()
                };
                assert!(matches!(error, EgressError::NetworkAccessDenied { url } if url == target));
            }
        }
    }
}

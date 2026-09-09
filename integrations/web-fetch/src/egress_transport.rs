//! Egress-backed HTTP transport for the `web_fetch` capability.
//!
//! Implements `knowledge/operations/egress.md` migration step 3 via fetchkit's transport
//! injection (fetchkit >= 0.4): [`EgressHttpTransport`] implements
//! `fetchkit::HttpTransport` over the host `EgressService`, so fetchkit keeps
//! its entire pipeline — specialized fetchers (GitHub, Wikipedia, arXiv, ...),
//! DNS policy (resolve-then-check producing pinned addresses, TM-API-008),
//! manual per-hop redirect validation, body caps, bot-auth signing, HTML
//! conversion, and file saving — while every HTTP hop crosses the egress
//! boundary, where the per-request `NetworkAccessList` and the deployment-wide
//! system allowlist are enforced (`knowledge/operations/system-allowlist.md`).
//!
//! fetchkit issues one transport call per redirect hop, so egress policy
//! applies to redirect targets too (TM-SSRF-010 stays in fetchkit; the egress
//! boundary independently re-denies hops its policy forbids). `pinned_addrs`
//! carries fetchkit's resolve-then-check result into
//! `EgressRequest.pinned_addrs`, closing the TOCTOU window at the egress
//! client (TM-TOOL-018).

use crate::egress::{EgressError, EgressRequest, EgressRequestKind, EgressService, EgressSigning};
use crate::network_access::NetworkAccessList;
use async_trait::async_trait;
use bytes::Bytes;
use fetchkit::{
    HttpTransport, TransportError, TransportMethod, TransportRequest, TransportResponse,
};
use futures::StreamExt;
use std::sync::Arc;

/// fetchkit transport that sends every HTTP hop through the host egress boundary.
pub(crate) struct EgressHttpTransport {
    egress: Arc<dyn EgressService>,
    /// Merged harness/agent/session access list, enforced at the egress
    /// boundary for every hop — the final enforcement point per
    /// `knowledge/operations/egress.md`. `web_fetch` pre-checks the initial URL itself for
    /// clearer user-facing errors.
    network_access: Option<NetworkAccessList>,
}

impl EgressHttpTransport {
    pub(crate) fn new(
        egress: Arc<dyn EgressService>,
        network_access: Option<NetworkAccessList>,
    ) -> Self {
        Self {
            egress,
            network_access,
        }
    }
}

#[async_trait]
impl HttpTransport for EgressHttpTransport {
    async fn execute(&self, req: TransportRequest) -> Result<TransportResponse, TransportError> {
        let method = match req.method {
            TransportMethod::Get => "GET",
            TransportMethod::Head => "HEAD",
        };
        let host = req.url.host_str().unwrap_or_default().to_string();
        let mut egress_request =
            EgressRequest::new(method, req.url.as_str(), EgressRequestKind::Capability)
                // fetchkit signs bot-auth headers before handing the hop to
                // the transport (re-signed per hop); platform-default signing
                // remains available for a future platform signer
                // (knowledge/operations/egress.md).
                .signing(EgressSigning::PlatformDefault)
                .network_access(self.network_access.clone())
                .pinned_addrs(host, req.pinned_addrs);
        for (name, value) in req.headers {
            egress_request = egress_request.header(name, value);
        }
        if let Some(timeout) = req.timeout {
            // Saturate instead of wrapping on absurdly large durations.
            let timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
            egress_request = egress_request.timeout_ms(timeout_ms);
        }
        // `respect_proxy_env` is intentionally not forwarded: outbound proxy
        // policy is host-owned at the egress boundary, not per-fetch config.

        let url = req.url;
        let response = self
            .egress
            .send_stream(egress_request)
            .await
            .map_err(map_egress_error)?;
        Ok(TransportResponse {
            status: response.status,
            url,
            headers: response.headers.into_iter().collect(),
            body: Box::pin(
                response
                    .body
                    .map(|chunk| chunk.map(Bytes::from).map_err(map_egress_error)),
            ),
        })
    }
}

/// Map egress failures onto transport errors.
///
/// `web_fetch` pre-checks the session access list and the system allowlist on
/// the initial URL with distinct messages, so a `NetworkAccessDenied` here
/// usually means a redirect hop was denied (by either list); keep the message
/// policy-explicit but list-neutral.
fn map_egress_error(error: EgressError) -> TransportError {
    match error {
        EgressError::NetworkAccessDenied { url } => {
            TransportError::Request(format!("Outbound request blocked by network policy: {url}"))
        }
        EgressError::InvalidRequest(message) => TransportError::Request(message),
        EgressError::SigningUnavailable => {
            TransportError::Other("outbound request signing unavailable".to_string())
        }
        EgressError::Transport(message) => TransportError::Request(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::{EgressResponse, EgressResult, EgressStreamResponse};
    use fetchkit::FetchRequest;
    use std::sync::Mutex;

    /// Programmable egress mock: pops queued responses in order and records
    /// every request it receives. Test URLs use public IP literals so
    /// fetchkit's DNS policy passes without resolution (pinned_addrs empty).
    struct MockEgress {
        responses: Mutex<Vec<EgressResult<EgressResponse>>>,
        requests: Mutex<Vec<EgressRequest>>,
    }

    impl MockEgress {
        fn with_responses(responses: Vec<EgressResult<EgressResponse>>) -> Self {
            Self {
                responses: Mutex::new(responses),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn ok(status: u16, headers: &[(&str, &str)], body: &str) -> EgressResult<EgressResponse> {
            Ok(EgressResponse {
                status,
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                body: body.as_bytes().to_vec(),
            })
        }

        fn requested_urls(&self) -> Vec<String> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.url.clone())
                .collect()
        }
    }

    #[async_trait]
    impl EgressService for MockEgress {
        async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
            self.requests.lock().unwrap().push(request);
            let mut responses = self.responses.lock().unwrap();
            assert!(!responses.is_empty(), "MockEgress ran out of responses");
            responses.remove(0)
        }

        async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
            let response = self.send(request).await?;
            Ok(EgressStreamResponse {
                status: response.status,
                headers: response.headers,
                body: Box::pin(futures::stream::once(async move { Ok(response.body) })),
            })
        }
    }

    fn tool_with_egress(egress: Arc<MockEgress>) -> fetchkit::Tool {
        fetchkit::Tool::builder()
            .transport(Arc::new(EgressHttpTransport::new(egress, None)))
            .build()
    }

    #[tokio::test]
    async fn fetches_html_and_converts_to_markdown() {
        let egress = Arc::new(MockEgress::with_responses(vec![MockEgress::ok(
            200,
            &[("content-type", "text/html; charset=utf-8")],
            "<html><head><title>T</title></head><body><h1>Title</h1><p>Body</p></body></html>",
        )]));
        let tool = tool_with_egress(egress.clone());

        let response = tool
            .execute(FetchRequest::new("http://93.184.216.34/page").as_markdown())
            .await
            .unwrap();

        assert_eq!(response.status_code, 200);
        assert_eq!(response.format.as_deref(), Some("markdown"));
        assert_eq!(response.content.as_deref(), Some("T\n# Title\n\nBody"));
        assert_eq!(egress.requested_urls(), vec!["http://93.184.216.34/page"]);
    }

    #[tokio::test]
    async fn follows_redirects_re_sending_each_hop_through_egress() {
        let egress = Arc::new(MockEgress::with_responses(vec![
            MockEgress::ok(302, &[("location", "http://93.184.216.35/final")], ""),
            MockEgress::ok(200, &[("content-type", "text/plain")], "done"),
        ]));
        let tool = tool_with_egress(egress.clone());

        let response = tool
            .execute(FetchRequest::new("http://93.184.216.34/start"))
            .await
            .unwrap();

        assert_eq!(response.status_code, 200);
        assert_eq!(response.url, "http://93.184.216.35/final");
        assert_eq!(response.content.as_deref(), Some("done"));
        assert_eq!(
            egress.requested_urls(),
            vec!["http://93.184.216.34/start", "http://93.184.216.35/final"],
            "every redirect hop must cross the egress boundary"
        );
    }

    fn transport_request(method: TransportMethod) -> TransportRequest {
        TransportRequest {
            method,
            url: "https://docs.example.test:8443/path?q=one".parse().unwrap(),
            headers: vec![
                ("user-agent".to_string(), "contract-agent".to_string()),
                ("if-none-match".to_string(), "\"v1\"".to_string()),
                ("signature".to_string(), "fixture-signature".to_string()),
            ],
            timeout: None,
            pinned_addrs: vec![],
            respect_proxy_env: false,
        }
    }

    #[tokio::test]
    async fn forwards_complete_requests_and_streamed_responses() {
        let acl = NetworkAccessList::allow_only(["docs.example.test"]);
        for (method, verb) in [
            (TransportMethod::Get, "GET"),
            (TransportMethod::Head, "HEAD"),
        ] {
            for (timeout, timeout_ms) in [
                (None, None),
                (Some(std::time::Duration::from_micros(1234)), Some(1)),
                (Some(std::time::Duration::MAX), Some(u64::MAX)),
            ] {
                for pinned in [
                    vec![],
                    vec![
                        "93.184.216.34:8443".parse().unwrap(),
                        "93.184.216.35:8443".parse().unwrap(),
                    ],
                ] {
                    let egress = Arc::new(MockEgress::with_responses(vec![MockEgress::ok(
                        206,
                        &[("content-type", "text/plain"), ("etag", "\"v2\"")],
                        "complete body",
                    )]));
                    let transport = EgressHttpTransport::new(egress.clone(), Some(acl.clone()));
                    let mut request = transport_request(method);
                    request.timeout = timeout;
                    request.pinned_addrs = pinned.clone();
                    request.respect_proxy_env = true;
                    let response = transport.execute(request).await.unwrap();
                    assert_eq!(response.status, 206);
                    assert_eq!(
                        response.url.as_str(),
                        "https://docs.example.test:8443/path?q=one"
                    );
                    assert_eq!(
                        response.headers,
                        vec![
                            ("content-type".to_string(), "text/plain".to_string()),
                            ("etag".to_string(), "\"v2\"".to_string())
                        ]
                    );
                    let chunks = response.body.collect::<Vec<_>>().await;
                    assert_eq!(chunks.len(), 1);
                    assert_eq!(chunks[0].as_ref().unwrap().as_ref(), b"complete body");
                    let requests = egress.requests.lock().unwrap();
                    assert_eq!(requests.len(), 1);
                    assert_eq!(
                        requests[0],
                        EgressRequest {
                            method: verb.to_string(),
                            url: "https://docs.example.test:8443/path?q=one".to_string(),
                            headers: [
                                ("user-agent".to_string(), "contract-agent".to_string()),
                                ("if-none-match".to_string(), "\"v1\"".to_string()),
                                ("signature".to_string(), "fixture-signature".to_string())
                            ]
                            .into_iter()
                            .collect(),
                            body: vec![],
                            kind: EgressRequestKind::Capability,
                            signing: EgressSigning::PlatformDefault,
                            network_access: Some(acl.clone()),
                            timeout_ms,
                            dns_pinning_required: false,
                            pinned_addrs: if pinned.is_empty() {
                                None
                            } else {
                                Some(("docs.example.test".to_string(), pinned))
                            },
                        }
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn egress_failures_preserve_policy_and_transport_diagnostics() {
        for (failure, expected, other) in [
            (
                EgressError::NetworkAccessDenied {
                    url: "https://blocked.example/path".to_string(),
                },
                "Outbound request blocked by network policy: https://blocked.example/path",
                false,
            ),
            (
                EgressError::InvalidRequest("invalid request".to_string()),
                "invalid request",
                false,
            ),
            (
                EgressError::SigningUnavailable,
                "outbound request signing unavailable",
                true,
            ),
            (
                EgressError::Transport("connection lost".to_string()),
                "connection lost",
                false,
            ),
        ] {
            let egress = Arc::new(MockEgress::with_responses(vec![Err(failure)]));
            let transport = EgressHttpTransport::new(egress.clone(), None);
            let error = match transport
                .execute(transport_request(TransportMethod::Get))
                .await
            {
                Err(error) => error,
                Ok(_) => panic!("failure became successful response"),
            };
            match error {
                TransportError::Other(message) if other => assert_eq!(message, expected),
                TransportError::Request(message) if !other => assert_eq!(message, expected),
                error => panic!("wrong transport error: {error:?}"),
            }
            assert_eq!(
                egress.requested_urls(),
                vec!["https://docs.example.test:8443/path?q=one"]
            );
        }
    }

    struct InterruptedBody;
    #[async_trait]
    impl EgressService for InterruptedBody {
        async fn send(&self, _: EgressRequest) -> EgressResult<EgressResponse> {
            panic!("adapter must use streaming egress")
        }
        async fn send_stream(&self, _: EgressRequest) -> EgressResult<EgressStreamResponse> {
            Ok(EgressStreamResponse {
                status: 200,
                headers: Default::default(),
                body: Box::pin(futures::stream::iter(vec![
                    Ok(vec![0, 255]),
                    Err(EgressError::Transport("body interrupted".to_string())),
                ])),
            })
        }
    }

    #[tokio::test]
    async fn partial_body_bytes_and_late_stream_errors_are_preserved() {
        let transport = EgressHttpTransport::new(Arc::new(InterruptedBody), None);
        let mut response = transport
            .execute(transport_request(TransportMethod::Get))
            .await
            .unwrap();
        assert_eq!(
            response.body.next().await.unwrap().unwrap().as_ref(),
            &[0, 255]
        );
        assert!(
            matches!(response.body.next().await,Some(Err(TransportError::Request(message))) if message=="body interrupted")
        );
        assert!(response.body.next().await.is_none());
    }
}

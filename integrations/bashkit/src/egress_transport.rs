//! Egress-backed HTTP transport for the `bashkit_shell` capability.
//!
//! Implements `knowledge/operations/egress.md` migration step 3 for bashkit via its
//! transport injection (bashkit >= 0.13, `knowledge/security/http-transport.md` there):
//! [`BashkitEgressTransport`] implements `bashkit::HttpTransport` over the
//! host `EgressService`, so bashkit keeps its entire HTTP pipeline — URL
//! allowlist, DNS/private-IP SSRF precheck (resolve-then-check producing
//! pinned addresses), per-hop redirect validation in curl/wget, credential
//! injection, bot-auth signing, and response caps — while every HTTP hop
//! crosses the egress boundary, where the per-request `NetworkAccessList`
//! and the deployment-wide system allowlist are enforced
//! (`knowledge/operations/system-allowlist.md`).
//!
//! curl/wget follow redirects manually and re-dispatch each hop, so egress
//! policy applies to redirect targets too. `pinned_addrs` carries bashkit's
//! resolve-then-check result into `EgressRequest.pinned_addrs`, closing the
//! DNS-rebind TOCTOU window at the egress client (TM-TOOL-018) — the same
//! contract as the web_fetch fetchkit transport.
//!
//! Error mapping is exit-code preserving: an egress policy denial becomes
//! `HttpTransportError::Denied`, which curl/wget surface as their native
//! `access denied` failure (exit 7), so agents see host policy exactly like
//! a bashkit allowlist denial. The response size cap bashkit communicates
//! is enforced here as well, mapping oversized bodies to `TooLarge`
//! (curl exit 63) before the body is handed back to the interpreter.

use crate::egress::{EgressError, EgressRequest, EgressRequestKind, EgressService, EgressSigning};
use crate::network_access::NetworkAccessList;
use async_trait::async_trait;
use bashkit::{HttpResponse, HttpTransport, HttpTransportError, HttpTransportRequest};
use futures::StreamExt;
use std::net::SocketAddr;
use std::sync::Arc;

/// bashkit transport that sends every curl/wget hop through the host egress boundary.
pub(crate) struct BashkitEgressTransport {
    egress: Arc<dyn EgressService>,
    /// Merged harness/agent/session access list, enforced at the egress
    /// boundary for every hop — the final enforcement point per
    /// `knowledge/operations/egress.md`.
    network_access: Option<NetworkAccessList>,
}

impl BashkitEgressTransport {
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
impl HttpTransport for BashkitEgressTransport {
    async fn execute(
        &self,
        request: HttpTransportRequest,
    ) -> Result<HttpResponse, HttpTransportError> {
        // Parse the URL to derive the pinned-address port; bashkit already
        // allowlist-validated it, so a parse failure here is a programming
        // error surfaced as a transport failure, not a policy denial.
        let url = url::Url::parse(&request.url)
            .map_err(|e| HttpTransportError::Transport(format!("invalid URL: {e}")))?;
        let host = url.host_str().unwrap_or_default().to_string();
        let port = url.port_or_known_default().unwrap_or(0);
        let pinned: Vec<SocketAddr> = request
            .pinned_addrs
            .iter()
            .map(|ip| SocketAddr::new(*ip, port))
            .collect();

        let mut egress_request = EgressRequest::new(
            request.method.as_str(),
            &request.url,
            EgressRequestKind::Capability,
        )
        // bashkit signs bot-auth headers before handing the hop to the
        // transport (re-signed per hop); platform-default signing
        // remains available for a future platform signer
        // (knowledge/operations/egress.md).
        .signing(EgressSigning::PlatformDefault)
        .network_access(self.network_access.clone())
        .pinned_addrs(host, pinned)
        .timeout_ms(u64::try_from(request.timeout.as_millis()).unwrap_or(u64::MAX));
        for (name, value) in &request.headers {
            egress_request = egress_request.header(name, value);
        }
        if let Some(body) = request.body {
            egress_request = egress_request.body(body);
        }
        // `connect_timeout` is not forwarded: the egress boundary owns
        // transport-level connection policy; the overall deadline above (and
        // bashkit's own enforcement around this call) bounds the request.

        let mut response = self
            .egress
            .send_stream(egress_request)
            .await
            .map_err(map_egress_error)?;

        // Enforce bashkit's response cap while streaming so a hostile endpoint
        // cannot force the worker to buffer an oversized body before the cap
        // is applied. bashkit re-checks after this call as a backstop.
        let mut body = Vec::new();
        while let Some(chunk) = response.body.next().await {
            let chunk = chunk.map_err(map_egress_error)?;
            let next_len = body.len().saturating_add(chunk.len());
            if next_len > request.max_response_bytes {
                return Err(HttpTransportError::TooLarge(format!(
                    "{} bytes (max: {} bytes)",
                    next_len, request.max_response_bytes
                )));
            }
            body.extend_from_slice(&chunk);
        }

        Ok(HttpResponse {
            status: response.status,
            headers: response.headers.into_iter().collect(),
            body,
        })
    }
}

/// Map egress failures onto bashkit transport errors so curl/wget exit codes
/// stay truthful: policy denial → `Denied` (exit 7), timeout → `Timeout`
/// (exit 28), everything else → `Transport` (exit 1).
fn map_egress_error(error: EgressError) -> HttpTransportError {
    match error {
        EgressError::NetworkAccessDenied { url } => {
            HttpTransportError::Denied(format!("outbound request blocked by network policy: {url}"))
        }
        EgressError::InvalidRequest(message) => HttpTransportError::Transport(message),
        EgressError::SigningUnavailable => {
            HttpTransportError::Transport("outbound request signing unavailable".to_string())
        }
        EgressError::Transport(message) => {
            // DirectEgressService surfaces reqwest deadline failures as
            // transport strings; classify them so scripts see curl exit 28.
            if message.contains("timed out") || message.contains("timeout") {
                HttpTransportError::Timeout
            } else {
                HttpTransportError::Transport(message)
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::egress::{EgressResponse, EgressResult, EgressStreamResponse};
    use std::sync::Mutex;

    /// Programmable egress mock: pops queued responses in order and records
    /// every request. Test URLs use public IP literals so bashkit's SSRF
    /// precheck validates the literal without DNS (hermetic in sandboxes).
    pub(crate) struct MockEgress {
        responses: Mutex<Vec<EgressResult<EgressResponse>>>,
        pub(crate) requests: Mutex<Vec<EgressRequest>>,
        pub(crate) send_calls: Mutex<usize>,
        pub(crate) stream_calls: Mutex<usize>,
    }

    impl MockEgress {
        pub(crate) fn with_responses(responses: Vec<EgressResult<EgressResponse>>) -> Self {
            Self {
                responses: Mutex::new(responses),
                requests: Mutex::new(Vec::new()),
                send_calls: Mutex::new(0),
                stream_calls: Mutex::new(0),
            }
        }

        pub(crate) fn ok(
            status: u16,
            headers: &[(&str, &str)],
            body: &str,
        ) -> EgressResult<EgressResponse> {
            Ok(EgressResponse {
                status,
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                body: body.as_bytes().to_vec(),
            })
        }
    }

    #[async_trait]
    impl EgressService for MockEgress {
        async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
            *self.send_calls.lock().unwrap() += 1;
            self.requests.lock().unwrap().push(request);
            let mut responses = self.responses.lock().unwrap();
            assert!(!responses.is_empty(), "MockEgress ran out of responses");
            responses.remove(0)
        }

        async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
            *self.stream_calls.lock().unwrap() += 1;
            self.requests.lock().unwrap().push(request);
            let mut responses = self.responses.lock().unwrap();
            assert!(!responses.is_empty(), "MockEgress ran out of responses");
            let response = responses.remove(0)?;
            Ok(EgressStreamResponse {
                status: response.status,
                headers: response.headers,
                body: Box::pin(futures::stream::once(async move { Ok(response.body) })),
            })
        }
    }

    // `HttpTransportRequest` is #[non_exhaustive], so it cannot be built
    // outside bashkit. End-to-end coverage of this transport therefore lives
    // in the capability tests (`bashkit_shell::tests::http_*`), which drive
    // real `curl` scripts through a `Bash` with this transport injected.
    // The mapping function is testable directly:

    #[test]
    fn maps_network_denial_to_denied() {
        let err = map_egress_error(EgressError::NetworkAccessDenied {
            url: "http://93.184.216.34/blocked".to_string(),
        });
        assert_eq!(
            err,
            HttpTransportError::Denied(
                "outbound request blocked by network policy: http://93.184.216.34/blocked"
                    .to_string()
            )
        );
    }

    #[test]
    fn maps_transport_timeout_to_timeout() {
        let err = map_egress_error(EgressError::Transport(
            "error sending request: operation timed out".to_string(),
        ));
        assert_eq!(err, HttpTransportError::Timeout);
    }

    #[test]
    fn maps_other_errors_to_transport() {
        let err = map_egress_error(EgressError::Transport("connection refused".to_string()));
        assert_eq!(
            err,
            HttpTransportError::Transport("connection refused".to_string())
        );
        let err = map_egress_error(EgressError::SigningUnavailable);
        assert!(matches!(err, HttpTransportError::Transport(_)));
    }
}

// Decisions:
// - Transport is a single-hop socket adapter. It NEVER follows redirects and NEVER
//   resolves DNS for policy: fetchkit owns redirect following (manual, per-hop validated)
//   and DNS policy (resolve-then-check). The transport only opens the connection and
//   streams bytes back. This keeps all security policy (TM-SSRF, TM-NET, TM-DOS) inside
//   fetchkit so a host application (e.g. Everruns) can swap the final socket transport
//   without being able to bypass fetchkit's safeguards.
// - pinned_addrs are produced by fetchkit's DnsPolicy (resolve-then-check). A custom
//   transport MUST connect only to those addresses when the list is non-empty
//   (TM-SSRF-001/005: prevents DNS rebinding). ReqwestTransport enforces this via
//   `reqwest::ClientBuilder::resolve()`.
// - The body is a boxed async byte stream so DefaultFetcher's body-size caps,
//   body timeout, and partial-content-on-timeout keep working mid-stream (TM-DOS-001/003).
//! Pluggable HTTP transport abstraction.
//!
//! [`HttpTransport`] lets a host application route fetchkit's outbound HTTP through
//! its own egress boundary while fetchkit retains all content logic and security
//! policy. fetchkit becomes a request/response adapter rather than owning the final
//! network transport.
//!
//! The default implementation, [`ReqwestTransport`], preserves the historical
//! behavior exactly: a fresh `reqwest::Client` per request, redirects disabled,
//! proxy env ignored unless opted in, and DNS pinned to fetchkit-validated addresses.

use crate::error::FetchError;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Duration;
use url::Url;

/// HTTP method for a transport request. Mirrors the GET/HEAD-only contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMethod {
    /// HTTP GET
    Get,
    /// HTTP HEAD
    Head,
}

/// A single outbound HTTP request handed to an [`HttpTransport`].
///
/// This represents exactly one hop. fetchkit performs URL validation, DNS policy
/// resolution, and manual redirect following itself; the transport only executes
/// the request as specified here.
pub struct TransportRequest {
    /// Request method (GET or HEAD).
    pub method: TransportMethod,
    /// Fully-resolved request URL for this hop.
    pub url: Url,
    /// Request headers (already includes User-Agent, Accept, conditional headers,
    /// and any bot-auth signature headers — signing stays in fetchkit).
    pub headers: Vec<(String, String)>,
    /// Connect + first-byte / overall timeout for this hop, if any.
    pub timeout: Option<Duration>,
    /// Pre-resolved, policy-validated socket addresses for the request host.
    ///
    /// Produced by fetchkit's [`DnsPolicy`](crate::DnsPolicy) (resolve-then-check).
    /// When non-empty a transport MUST connect only to these addresses — this is the
    /// SSRF / DNS-rebinding protection (TM-SSRF-001, TM-SSRF-005). When empty, the
    /// policy permits ambient resolution (e.g. `DnsPolicy::allow_all`).
    pub pinned_addrs: Vec<SocketAddr>,
    /// Whether ambient `HTTP_PROXY`/`HTTPS_PROXY`/`NO_PROXY` should be honored.
    ///
    /// Carried per-request because it is derived from `FetchOptions`. Defaults to
    /// `false` (proxy env ignored) per TM-NET-004.
    pub respect_proxy_env: bool,
}

/// Boxed async byte stream for a streaming response body.
pub type BodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, TransportError>> + Send>>;

/// A response produced by an [`HttpTransport`].
///
/// The body is a streaming byte source so fetchkit can apply its body-size cap,
/// body timeout, and partial-content-on-timeout logic while reading.
pub struct TransportResponse {
    /// HTTP status code.
    pub status: u16,
    /// Final URL after the transport executed the request (same as request URL;
    /// fetchkit follows redirects itself, so this normally equals the request URL).
    pub url: Url,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Streaming response body.
    pub body: BodyStream,
}

/// Errors a transport can return. Variants map onto [`FetchError`] without leaking
/// internal hostnames/IPs (TM-LEAK-001).
///
/// Custom transports use the generic [`Connect`](TransportError::Connect),
/// [`Timeout`](TransportError::Timeout), [`Request`](TransportError::Request), and
/// [`Other`](TransportError::Other) variants. The [`Reqwest`](TransportError::Reqwest)
/// variant lets the default [`ReqwestTransport`] preserve reqwest's precise error
/// classification (so historical [`FetchError`] mapping is unchanged); custom
/// transports never need it.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// Failed to establish a connection (DNS/connect/TLS).
    #[error("transport connect error")]
    Connect,
    /// Timed out waiting for the server.
    #[error("transport timeout")]
    Timeout,
    /// Error issuing the request or reading the response.
    #[error("transport request error: {0}")]
    Request(String),
    /// Any other transport failure.
    #[error("transport error: {0}")]
    Other(String),
    /// A raw reqwest error from the default transport (preserves precise classification).
    #[error("transport reqwest error")]
    Reqwest(#[source] reqwest::Error),
}

impl From<TransportError> for FetchError {
    fn from(err: TransportError) -> Self {
        // THREAT[TM-LEAK-001]: keep generic messages; do not surface resolved IPs/hosts.
        match err {
            // Reqwest errors keep the historical classification (connect/timeout/body/...).
            TransportError::Reqwest(e) => FetchError::from_reqwest(e),
            TransportError::Connect => {
                FetchError::RequestError("failed to connect to server".to_string())
            }
            TransportError::Timeout => FetchError::FirstByteTimeout,
            TransportError::Request(msg) => FetchError::RequestError(msg),
            TransportError::Other(msg) => FetchError::RequestError(msg),
        }
    }
}

/// A pluggable HTTP transport.
///
/// Implementors execute a single [`TransportRequest`] and return a streaming
/// [`TransportResponse`]. fetchkit retains all URL validation, DNS policy, redirect
/// following, and content handling; only the socket-level send moves here.
#[async_trait]
pub trait HttpTransport: Send + Sync {
    /// Execute a single HTTP request hop.
    async fn execute(&self, req: TransportRequest) -> Result<TransportResponse, TransportError>;
}

/// Default transport backed by `reqwest`.
///
/// Preserves the historical fetch behavior exactly:
/// - new `reqwest::Client` per request (TM-NET-003: no pool state leakage)
/// - redirects disabled (TM-SSRF-010: fetchkit follows redirects manually)
/// - ambient proxy env ignored unless `respect_proxy_env` (TM-NET-004)
/// - DNS pinned to fetchkit-validated addresses when `pinned_addrs` is non-empty
///   (TM-SSRF-001, TM-SSRF-005)
#[derive(Debug, Default, Clone)]
pub struct ReqwestTransport;

impl ReqwestTransport {
    /// Create a new default reqwest-backed transport.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn execute(&self, req: TransportRequest) -> Result<TransportResponse, TransportError> {
        // THREAT[TM-NET-003]: New client per request prevents connection-pool state leakage.
        let mut builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());

        if let Some(timeout) = req.timeout {
            builder = builder.connect_timeout(timeout).timeout(timeout);
        }

        if !req.respect_proxy_env {
            // THREAT[TM-NET-004]: Ignore ambient proxy env by default in shared runtimes.
            builder = builder.no_proxy();
        }

        // THREAT[TM-SSRF-001]: Pin connection to fetchkit's validated addresses.
        // THREAT[TM-SSRF-005]: Prevents reqwest from re-resolving (DNS rebinding).
        if !req.pinned_addrs.is_empty() {
            if let Some(host) = req.url.host_str() {
                builder = builder.resolve_to_addrs(host, &req.pinned_addrs);
            }
        }

        let client = builder
            .build()
            .map_err(|e| TransportError::Other(e.to_string()))?;

        let method = match req.method {
            TransportMethod::Get => reqwest::Method::GET,
            TransportMethod::Head => reqwest::Method::HEAD,
        };

        let mut request = client.request(method, req.url.clone());
        for (name, value) in &req.headers {
            request = request.header(name, value);
        }

        let response = request.send().await.map_err(TransportError::Reqwest)?;

        let status = response.status().as_u16();
        let final_url = response.url().clone();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.as_str().to_string(), v.to_string()))
            })
            .collect();

        let body: BodyStream = Box::pin(
            response
                .bytes_stream()
                .map(|chunk| chunk.map_err(TransportError::Reqwest)),
        );

        Ok(TransportResponse {
            status,
            url: final_url,
            headers,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transport_error_maps_to_fetch_error() {
        assert!(matches!(
            FetchError::from(TransportError::Timeout),
            FetchError::FirstByteTimeout
        ));
        assert!(matches!(
            FetchError::from(TransportError::Connect),
            FetchError::RequestError(_)
        ));
        assert!(matches!(
            FetchError::from(TransportError::Request("x".into())),
            FetchError::RequestError(_)
        ));
    }
}

use crate::url_validation::is_blocked_ip;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Connect/TLS handshake timeout applied to every provider HTTP client.
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Per-read inactivity timeout for streaming chat clients. This bounds a
/// silently stalled connection (no bytes for this long) without capping the
/// total time a long, actively-streaming response may take — an overall
/// `.timeout()` would do the latter and kill legitimate long streams. Set well
/// above the agent loop's own stall timeout (EVE-531, ~120s) so it only acts as
/// a transport-level backstop.
const HTTP_STREAM_READ_TIMEOUT: Duration = Duration::from_secs(300);
/// Overall request timeout for non-streaming reads (embeddings, vector store)
/// so a hung response body cannot block a request indefinitely.
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Idle pooled-connection lifetime, so reused HTTP keep-alive/HTTP-2
/// connections are eventually recycled.
const HTTP_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// Shorter idle lifetime for the streaming client. A provider/edge often closes
/// idle keep-alive sockets after ~10-30s; reusing one the peer already closed is
/// a dominant source of the mid-stream `error decoding response body` flake. The
/// streaming reconnect layer (`stream_reconnect`) recovers from those, but
/// recycling sooner avoids most of them outright — matching the official OpenAI
/// SDK transport (httpx), whose default `keepalive_expiry` is 5s. Kept a little
/// above 5s so genuinely rapid successive turns still reuse a warm connection.
const HTTP_STREAM_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(15);
/// DNS resolution is capped so a stuck resolver cannot stall a provider call
/// past the outbound HTTP timeout. Mirrors `url_validation`'s lookup cap.
const DNS_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

/// SSRF-guarding DNS resolver for the shared provider HTTP clients (EVE-623).
///
/// Provider `base_url`s are org-configurable and validated only at create time,
/// so a hostname that passed validation can later DNS-rebind to a private or
/// cloud-metadata address at request time. These streaming/request drivers hold
/// a `reqwest::Client` directly rather than routing each call through
/// the host egress service, so we enforce the same DNS-pinning contract
/// (TM-API-013, TM-TOOL-018) inside the client itself: every resolved address is
/// checked against [`is_blocked_ip`] and the connection is refused if any
/// resolved IP is private/internal. Combined with redirects disabled, this keeps
/// these clients from reaching `169.254.169.254`/loopback/RFC1918 regardless of
/// what the configured URL or a 3xx `Location` later points at.
///
/// Public so other direct (non-egress) clients, such as the server's blocking
/// Git smart-HTTP fetcher (EVE-1178), install the same guard instead of a copy.
/// reqwest connects only to the addresses this returns, so the check and the
/// connection see the same answer. IP-literal hosts never reach a resolver;
/// callers must still reject those statically with
/// [`crate::url_validation::validate_safe_url`].
#[derive(Clone, Default)]
pub struct SsrfGuardResolver {
    /// Resolver whose answers are guarded; `None` uses the system resolver.
    upstream: Option<Arc<dyn Resolve>>,
}

impl SsrfGuardResolver {
    /// Guard the system resolver's answers.
    pub fn system() -> Self {
        Self::default()
    }

    /// Guard another resolver's answers. Tests use this to simulate a hostname
    /// that resolves publicly at registration and privately at request time.
    pub fn wrapping(upstream: Arc<dyn Resolve>) -> Self {
        Self {
            upstream: Some(upstream),
        }
    }
}

/// Boxed error type expected by reqwest's [`Resolving`] future. reqwest's own
/// `BoxError` alias is crate-private, so we spell it out here.
type DnsBoxError = Box<dyn std::error::Error + Send + Sync>;

impl Resolve for SsrfGuardResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        let upstream = self.upstream.clone();
        Box::pin(async move {
            let addrs: Vec<std::net::SocketAddr> = match upstream {
                Some(upstream) => upstream.resolve(name).await?.collect(),
                None => {
                    // hyper strips the port before resolving; resolve with port 0
                    // and let reqwest apply the URL's actual port. We only inspect
                    // the IPs here.
                    tokio::time::timeout(
                        DNS_LOOKUP_TIMEOUT,
                        tokio::net::lookup_host(format!("{host}:0")),
                    )
                    .await
                    .map_err(|_| -> DnsBoxError {
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "DNS lookup timed out",
                        ))
                    })?
                    .map_err(|e| -> DnsBoxError { Box::new(e) })?
                    .collect()
                }
            };

            for addr in &addrs {
                if is_blocked_ip(addr.ip()) {
                    tracing::warn!(
                        host = %host,
                        resolved_ip = %addr.ip(),
                        "HTTP client blocked: hostname resolves to private/internal address"
                    );
                    return Err(Box::new(std::io::Error::other(format!(
                        "host {host} resolves to blocked address {} (private/internal)",
                        addr.ip()
                    ))) as DnsBoxError);
                }
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// Apply the shared SSRF-hardening to a provider HTTP client builder (EVE-623):
/// disable redirect following (a 3xx `Location` must never be auto-fetched, since
/// it can point at an internal address) and install the [`SsrfGuardResolver`].
fn harden_builder(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    builder
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(SsrfGuardResolver::system()))
}

/// Process-wide HTTP client shared by all streaming chat drivers.
///
/// `reqwest::Client` is internally reference-counted and built to be cloned and
/// shared; it carries no per-request credentials (auth headers are attached per
/// request), so one pool is safe to reuse across providers, credentials, and
/// reasoning steps. Sharing it is what lets TCP/TLS handshakes and HTTP/2
/// connections be reused across agent turns even though the driver structs are
/// rebuilt on every step (EVE-635).
pub fn shared_streaming_http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| streaming_http_client(HTTP_STREAM_READ_TIMEOUT))
        .clone()
}

/// Gemini long decode permits up to twenty minutes between reads in a hop.
/// Keeps the same credential-free pooling and SSRF guards as other streams.
pub fn shared_long_decode_http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| streaming_http_client(Duration::from_secs(1_200)))
        .clone()
}

fn streaming_http_client(read_timeout: Duration) -> reqwest::Client {
    crate::install_default_crypto_provider();
    harden_builder(
        reqwest::Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .read_timeout(read_timeout)
            .pool_idle_timeout(HTTP_STREAM_POOL_IDLE_TIMEOUT),
    )
    .build()
    // Fall back to a minimal but still SSRF-hardened client rather than
    // a bare `Client::new()`, so a builder failure can never silently
    // drop the redirect/DNS-pinning guard (EVE-623).
    .unwrap_or_else(|_| {
        harden_builder(reqwest::Client::builder())
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Process-wide HTTP client shared by non-streaming request/response drivers
/// (embeddings, vector store). Uses an overall request timeout because these
/// reads are bounded and must not hang forever.
pub fn shared_request_http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            crate::install_default_crypto_provider();
            harden_builder(
                reqwest::Client::builder()
                    .connect_timeout(HTTP_CONNECT_TIMEOUT)
                    .timeout(HTTP_REQUEST_TIMEOUT)
                    .pool_idle_timeout(HTTP_POOL_IDLE_TIMEOUT),
            )
            .build()
            // Fall back to a minimal but still SSRF-hardened client rather than
            // a bare `Client::new()`, so a builder failure can never silently
            // drop the redirect/DNS-pinning guard (EVE-623).
            .unwrap_or_else(|_| {
                harden_builder(reqwest::Client::builder())
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new())
            })
        })
        .clone()
}

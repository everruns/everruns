//! Runtime network policy for outbound A2A delegation (EVE-1173 / TM-AGENT-024).
//!
//! Discovery and every AgentCard interface request use a no-redirect HTTP
//! client. Unless the development hatch is on, each URL is DNS-pinned so the
//! connected address is the one that passed the public-IP checks.

use everruns_provider::url_validation::{validate_url_dns_pinned, validate_url_with_resolver};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// Connect timeout for outbound A2A discovery and transport (matches AG-UI).
pub(super) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) type DnsResolveFuture =
    Pin<Box<dyn Future<Output = std::io::Result<Vec<SocketAddr>>> + Send>>;
pub(super) type DnsResolver = Arc<dyn Fn(String, u16) -> DnsResolveFuture + Send + Sync>;

/// Build the HTTP client used for AgentCard discovery and every interface
/// request. Redirects are always disabled. Unless `allow_local` is set (dev
/// hatch only), each URL is DNS-pinned so the connected address is the one
/// that passed the public-IP checks.
pub(super) async fn hardened_a2a_http_client(
    urls: &[&str],
    allow_local: bool,
    resolver: Option<&DnsResolver>,
) -> std::result::Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT);
    if !allow_local {
        for raw in urls {
            let validated = match resolver {
                Some(resolve) => {
                    let resolve = Arc::clone(resolve);
                    validate_url_with_resolver(raw, move |host, port| {
                        let resolve = Arc::clone(&resolve);
                        async move { resolve(host, port).await }
                    })
                    .await
                }
                None => validate_url_dns_pinned(raw).await,
            };
            let (url, addrs) = validated.map_err(|e| format!("A2A URL unsafe: {e}"))?;
            // An IP literal comes back with no addresses: the static check
            // already validated it and there is nothing to pin.
            if let (Some(host), false) = (url.host_str(), addrs.is_empty()) {
                builder = builder.resolve_to_addrs(host, &addrs);
            }
        }
    }
    builder
        .build()
        .map_err(|e| format!("Failed to build A2A HTTP client: {e}"))
}

#[cfg(test)]
pub(super) fn controlled_dns_resolver<F, Fut>(resolve: F) -> DnsResolver
where
    F: Fn(String, u16) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = std::io::Result<Vec<SocketAddr>>> + Send + 'static,
{
    Arc::new(move |host, port| Box::pin(resolve(host, port)))
}

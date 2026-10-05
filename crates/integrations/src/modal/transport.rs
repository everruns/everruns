//! gRPC channels to Modal: TLS, keepalive, and HTTP CONNECT proxy support.
//!
//! Decision: Modal has no REST API; its SDKs speak gRPC to `api.modal.com` and
//! to a per-task command router. tonic dials TCP directly and ignores
//! `HTTPS_PROXY`, which every other integration honours through reqwest, so a
//! proxied deployment (and the CI/agent sandboxes) could not reach Modal at
//! all. The channel tunnels through the proxy itself with hyper-util's
//! `Tunnel` connector, and tonic layers TLS on top of the tunnel, so the TLS
//! session is still end to end with Modal.

use std::time::Duration;

use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::connect::proxy::Tunnel;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint, Uri};

/// Largest message either side may send. Matches the Modal SDKs.
pub(crate) const MAX_MESSAGE_SIZE: usize = 100 * 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Open a lazily connected channel to `url` (`https://host[:port]` or, for
/// tests, `http://host:port`).
pub(crate) fn channel(url: &str) -> Result<Channel, String> {
    let uri: Uri = url
        .parse()
        .map_err(|e| format!("Invalid Modal endpoint URL {url}: {e}"))?;
    let host = uri
        .host()
        .ok_or_else(|| format!("Modal endpoint URL has no host: {url}"))?
        .to_string();

    let mut endpoint = Endpoint::from(uri.clone())
        .connect_timeout(CONNECT_TIMEOUT)
        .http2_keep_alive_interval(Duration::from_secs(30))
        .keep_alive_timeout(Duration::from_secs(10))
        .keep_alive_while_idle(true)
        .initial_stream_window_size(Some(64 * 1024 * 1024))
        .initial_connection_window_size(Some(64 * 1024 * 1024));

    if uri.scheme_str() == Some("https") {
        // Native roots honour SSL_CERT_FILE, which is how an intercepting
        // corporate or agent proxy's CA is trusted; webpki roots cover hosts
        // without a system store.
        endpoint = endpoint
            .tls_config(
                ClientTlsConfig::new()
                    .with_native_roots()
                    .with_webpki_roots()
                    .domain_name(host.clone()),
            )
            .map_err(|e| format!("Failed to configure TLS for {url}: {e}"))?;
    } else if uri.scheme_str() != Some("http") {
        return Err(format!("Modal endpoint must be http or https: {url}"));
    }

    match proxy_for(&host, uri.scheme_str() == Some("https")) {
        Some(proxy) => {
            let mut http = HttpConnector::new();
            http.enforce_http(false);
            http.set_connect_timeout(Some(CONNECT_TIMEOUT));
            Ok(endpoint.connect_with_connector_lazy(Tunnel::new(proxy, http)))
        }
        None => Ok(endpoint.connect_lazy()),
    }
}

/// The proxy to tunnel through for `host`, from the conventional environment
/// variables, or `None` for a direct connection.
fn proxy_for(host: &str, https: bool) -> Option<Uri> {
    let vars: &[&str] = if https {
        &["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    } else {
        &["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
    };
    let proxy = vars
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()))?;
    let no_proxy = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .unwrap_or_default();
    if host_is_excluded(host, &no_proxy) {
        return None;
    }
    proxy.trim().parse().ok()
}

/// `NO_PROXY` matching: `*`, exact hosts, and domain suffixes (`.example.com`
/// or `example.com` both match `api.example.com`). CIDR entries are matched
/// only for the common loopback case, which is what local mocks need.
fn host_is_excluded(host: &str, no_proxy: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    no_proxy
        .split(',')
        .map(|entry| entry.trim().to_ascii_lowercase())
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            if entry == "*" {
                return true;
            }
            if entry == "127.0.0.0/8" {
                return host.starts_with("127.");
            }
            let suffix = entry.trim_start_matches("*.").trim_start_matches('.');
            host == suffix || host.ends_with(&format!(".{suffix}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_proxy_matches_hosts_and_suffixes() {
        let list = "localhost,127.0.0.1,.svc.cluster.local,example.com,*.internal";
        assert!(host_is_excluded("localhost", list));
        assert!(host_is_excluded("127.0.0.1", list));
        assert!(host_is_excluded("api.svc.cluster.local", list));
        assert!(host_is_excluded("example.com", list));
        assert!(host_is_excluded("api.example.com", list));
        assert!(host_is_excluded("db.internal", list));
        assert!(!host_is_excluded("api.modal.com", list));
        assert!(!host_is_excluded("notexample.com", list));
        assert!(host_is_excluded("anything", "*"));
        assert!(host_is_excluded("127.0.0.5", "127.0.0.0/8"));
    }

    #[test]
    fn rejects_unsupported_schemes() {
        assert!(channel("ftp://api.modal.com").is_err());
        assert!(channel("not a url").is_err());
    }
}

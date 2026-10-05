//! Everruns-side network boundary for Browserless browsers (EVE-1189).
//!
//! THREAT[TM-TOOL-015][TM-TOOL-056]: A Browserless browser resolves DNS and
//! follows redirects on its own. Checking only the URL a tool was given leaves
//! redirect hops, requests the page discovers, and hostnames that resolve (or
//! rebind) to private, loopback, link-local, or metadata addresses unchecked.
//!
//! Decision: the browser gets no network of its own. Every browser this crate
//! drives runs in a browser context whose proxy is a dead loopback port, so a
//! request the browser makes by itself fails. CDP `Fetch` pauses each request
//! before it leaves the browser, and Everruns performs it here instead:
//!
//! - static SSRF policy and the session network access list on every hop,
//! - DNS answers checked at connect time by [`SsrfGuardResolver`], so a
//!   hostname that rebinds to a private address is refused at the socket,
//! - redirects never followed here; a 3xx goes back to the browser, which
//!   issues the next hop as a new paused request that is checked again.
//!
//! The browser only ever sees responses this module produced, so content and
//! screenshots cannot carry an internal response. A request paused while no
//! Everruns client is attached is continued by Chrome into the dead proxy and
//! fails, which keeps the persistent browser fail-closed between tool calls.
//! WebSockets bypass `Fetch`; they also go to the dead proxy and fail.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use everruns_contracts::driver_helpers::SsrfGuardResolver;
use everruns_contracts::url_validation::validate_safe_url;
use everruns_contracts::runtime::SystemAllowlist;
use everruns_contracts::runtime::network_access::NetworkAccessList;
use reqwest::dns::Resolve;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tracing::debug;

/// Proxy for every browser context this crate creates. Port 9 (discard) on
/// loopback never runs an HTTP proxy, so anything `Fetch` did not answer
/// fails with `ERR_PROXY_CONNECTION_FAILED` instead of reaching a network.
pub(crate) const DEAD_PROXY_SERVER: &str = "http://127.0.0.1:9";
/// Chrome bypasses the proxy for loopback by default; `<-loopback>` removes
/// that implicit bypass so loopback requests hit the dead proxy too.
pub(crate) const DEAD_PROXY_BYPASS_LIST: &str = "<-loopback>";

/// Largest response body relayed to the browser. The body travels base64
/// encoded in one CDP message, so this also bounds the WebSocket frame size.
const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;
/// Concurrent upstream requests per browser connection.
const MAX_IN_FLIGHT: usize = 6;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Request headers Everruns does not forward: hop-by-hop headers, headers the
/// HTTP client owns, and `accept-encoding`, which is pinned to `identity`
/// because Chrome does not decode a fulfilled body.
const DROPPED_REQUEST_HEADERS: &[&str] = &[
    "accept-encoding",
    "connection",
    "content-length",
    "host",
    "keep-alive",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// Response headers that describe the upstream transfer, not the body the
/// browser receives (which is always the decoded, complete body).
const DROPPED_RESPONSE_HEADERS: &[&str] = &[
    "connection",
    "content-encoding",
    "content-length",
    "keep-alive",
    "proxy-connection",
    "trailer",
    "transfer-encoding",
];

/// A CDP command answering one `Fetch.requestPaused` event.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PausedAnswer {
    pub method: &'static str,
    pub params: Value,
}

impl PausedAnswer {
    fn fail(request_id: &str, reason: &'static str) -> Self {
        Self {
            method: "Fetch.failRequest",
            params: json!({ "requestId": request_id, "errorReason": reason }),
        }
    }

    /// Refuse a request on policy grounds.
    pub(crate) fn blocked(request_id: &str) -> Self {
        Self::fail(request_id, "BlockedByClient")
    }

    fn continue_local(request_id: &str) -> Self {
        Self {
            method: "Fetch.continueRequest",
            params: json!({ "requestId": request_id }),
        }
    }

    /// Whether this answer blocks the request.
    #[cfg(test)]
    pub fn is_failure(&self) -> bool {
        self.method == "Fetch.failRequest"
    }
}

/// Performs browser requests on the browser's behalf under Everruns policy.
pub struct BrowserEgress {
    client: reqwest::Client,
    network_access: Option<NetworkAccessList>,
    /// Host-wide allowlist (`EVERRUNS_SYSTEM_ALLOWLIST_ENABLED`). Page traffic
    /// now leaves from the Everruns host, so the host policy applies to it.
    system_allowlist: Option<Arc<SystemAllowlist>>,
    in_flight: Semaphore,
    /// Test-only escape so unit tests can serve responses from a loopback
    /// mock server. Production code never sets it.
    #[cfg(test)]
    allow_loopback_for_tests: bool,
}

impl std::fmt::Debug for BrowserEgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserEgress")
            .field("network_access", &self.network_access)
            .finish_non_exhaustive()
    }
}

impl BrowserEgress {
    /// Egress for one tool call, applying the session network access list.
    pub fn new(network_access: Option<NetworkAccessList>) -> Arc<Self> {
        // Like the platform's other runtime clients, an operator's
        // HTTP(S)_PROXY is honored. That proxy then resolves names; the static
        // and access-list checks still run on every hop.
        Self::build(network_access, Arc::new(SsrfGuardResolver::system()), true)
    }

    /// Egress for a tool call in `context`, under its session network policy.
    pub fn for_context(context: &everruns_contracts::runtime::tool_context::ToolContext) -> Arc<Self> {
        Self::new(context.network_access.clone())
    }

    /// Egress with an injected (already guarding) resolver whose answers are
    /// authoritative: the client connects directly, never via an env proxy.
    /// Tests use [`SsrfGuardResolver::wrapping`] to simulate DNS rebinding.
    pub fn with_resolver(
        network_access: Option<NetworkAccessList>,
        resolver: Arc<dyn Resolve>,
    ) -> Arc<Self> {
        Self::build(network_access, resolver, false)
    }

    fn build(
        network_access: Option<NetworkAccessList>,
        resolver: Arc<dyn Resolve>,
        honor_env_proxy: bool,
    ) -> Arc<Self> {
        everruns_contracts::install_default_crypto_provider();
        let mut builder = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(resolver);
        if !honor_env_proxy {
            builder = builder.no_proxy();
        }
        let client = builder.build().unwrap_or_else(|_| {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .dns_resolver(Arc::new(SsrfGuardResolver::system()))
                .build()
                .expect("build guarded browser egress client")
        });
        Arc::new(Self {
            client,
            network_access: network_access.filter(|access| !access.is_empty()),
            system_allowlist: SystemAllowlist::from_env(),
            in_flight: Semaphore::new(MAX_IN_FLIGHT),
            #[cfg(test)]
            allow_loopback_for_tests: false,
        })
    }

    /// Test egress that may reach `127.0.0.1` mock servers by IP literal.
    /// Every other host keeps the full policy.
    #[cfg(test)]
    pub(crate) fn allowing_loopback_for_tests(network_access: Option<NetworkAccessList>) -> Self {
        Self::loopback_test_egress(network_access, None)
    }

    /// As [`Self::allowing_loopback_for_tests`], with hostnames resolved by
    /// `resolver` (an [`SsrfGuardResolver`] in tests that exercise rebinding).
    #[cfg(test)]
    pub(crate) fn allowing_loopback_for_tests_with_resolver(resolver: Arc<dyn Resolve>) -> Self {
        Self::loopback_test_egress(None, Some(resolver))
    }

    #[cfg(test)]
    fn loopback_test_egress(
        network_access: Option<NetworkAccessList>,
        resolver: Option<Arc<dyn Resolve>>,
    ) -> Self {
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
        if let Some(resolver) = resolver {
            builder = builder.dns_resolver(resolver);
        }
        let client = builder.build().expect("client");
        Self {
            client,
            network_access,
            system_allowlist: SystemAllowlist::from_env(),
            in_flight: Semaphore::new(MAX_IN_FLIGHT),
            allow_loopback_for_tests: true,
        }
    }

    /// Policy check that needs no network: scheme, static SSRF ranges, and the
    /// session access list. `Err` carries a short reason for logs.
    fn check_url(&self, url: &str) -> Result<(), &'static str> {
        #[cfg(test)]
        let skip_static = self.allow_loopback_for_tests
            && reqwest::Url::parse(url).is_ok_and(|parsed| parsed.host_str() == Some("127.0.0.1"));
        #[cfg(not(test))]
        let skip_static = false;
        if !skip_static && validate_safe_url(url).is_err() {
            return Err("blocked address");
        }
        if let Some(access) = &self.network_access
            && !access.is_url_allowed(url)
        {
            return Err("network access list");
        }
        if let Some(allowlist) = &self.system_allowlist
            && !allowlist.is_url_allowed(url)
        {
            return Err("system allowlist");
        }
        Ok(())
    }

    /// Decide and, when allowed, perform one paused browser request.
    ///
    /// `None` when the event carries no request id (nothing to answer).
    pub(crate) async fn answer(&self, event_params: &Value) -> Option<PausedAnswer> {
        let request_id = event_params
            .get("requestId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())?;
        let request = event_params.get("request").cloned().unwrap_or(Value::Null);
        let url = request.get("url").and_then(Value::as_str).unwrap_or("");

        // Documents that never touch the network.
        if is_local_scheme(url) {
            return Some(PausedAnswer::continue_local(request_id));
        }

        if let Err(reason) = self.check_url(url) {
            debug!(host = %host_for_log(url), reason, "browser request blocked");
            return Some(PausedAnswer::blocked(request_id));
        }

        let Ok(_permit) = self.in_flight.acquire().await else {
            return Some(PausedAnswer::fail(request_id, "Failed"));
        };
        Some(match self.relay(request_id, &request, url).await {
            Ok(answer) => answer,
            Err(reason) => {
                debug!(host = %host_for_log(url), reason, "browser request failed");
                PausedAnswer::fail(request_id, reason)
            }
        })
    }

    async fn relay(
        &self,
        request_id: &str,
        request: &Value,
        url: &str,
    ) -> Result<PausedAnswer, &'static str> {
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET");
        let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| "Failed")?;
        let mut builder = self.client.request(method, url);
        if let Some(headers) = request.get("headers").and_then(Value::as_object) {
            for (name, value) in headers {
                if DROPPED_REQUEST_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                    continue;
                }
                if let Some(value) = value.as_str() {
                    builder = builder.header(name.as_str(), value);
                }
            }
        }
        builder = builder.header("accept-encoding", "identity");
        if let Some(body) = request_body(request)? {
            builder = builder.body(body);
        }

        let mut response = builder.send().await.map_err(|error| {
            if error.is_timeout() {
                "TimedOut"
            } else if error.is_connect() {
                // Includes a hostname whose DNS answer the guard refused.
                "AddressUnreachable"
            } else {
                "Failed"
            }
        })?;

        let status = response.status();
        let encoding = response
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim().to_ascii_lowercase());
        let headers: Vec<Value> = response
            .headers()
            .iter()
            .filter(|(name, _)| !DROPPED_RESPONSE_HEADERS.contains(&name.as_str()))
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| json!({ "name": name.as_str(), "value": value }))
            })
            .collect();

        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "Failed")? {
            if body.len() + chunk.len() > MAX_BODY_BYTES {
                return Err("Failed");
            }
            body.extend_from_slice(&chunk);
        }
        let body = decode_body(encoding.as_deref(), body)?;

        let mut params = json!({
            "requestId": request_id,
            "responseCode": status.as_u16(),
            "responseHeaders": headers,
            "body": base64::engine::general_purpose::STANDARD.encode(&body),
        });
        if let Some(reason) = status.canonical_reason() {
            params["responsePhrase"] = json!(reason);
        }
        Ok(PausedAnswer {
            method: "Fetch.fulfillRequest",
            params,
        })
    }
}

/// The paused request's body, from `postDataEntries` (exact bytes) or
/// `postData`. A body Chrome reports but did not include fails closed rather
/// than being sent empty.
fn request_body(request: &Value) -> Result<Option<Vec<u8>>, &'static str> {
    if let Some(entries) = request.get("postDataEntries").and_then(Value::as_array) {
        let mut body = Vec::new();
        for entry in entries {
            if let Some(bytes) = entry.get("bytes").and_then(Value::as_str) {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(bytes)
                    .map_err(|_| "Failed")?;
                body.extend_from_slice(&decoded);
            }
        }
        return Ok(Some(body));
    }
    if let Some(data) = request.get("postData").and_then(Value::as_str) {
        return Ok(Some(data.as_bytes().to_vec()));
    }
    if request.get("hasPostData").and_then(Value::as_bool) == Some(true) {
        return Err("Failed");
    }
    Ok(None)
}

/// Decode a body a server compressed although we asked for `identity`.
fn decode_body(encoding: Option<&str>, body: Vec<u8>) -> Result<Vec<u8>, &'static str> {
    use std::io::Read;
    let read_capped = |reader: &mut dyn Read| -> Result<Vec<u8>, &'static str> {
        let mut out = Vec::new();
        reader
            .take(MAX_BODY_BYTES as u64 + 1)
            .read_to_end(&mut out)
            .map_err(|_| "Failed")?;
        if out.len() > MAX_BODY_BYTES {
            return Err("Failed");
        }
        Ok(out)
    };
    match encoding {
        None | Some("") | Some("identity") => Ok(body),
        Some("gzip") | Some("x-gzip") => {
            read_capped(&mut flate2::read::GzDecoder::new(body.as_slice()))
        }
        Some("deflate") => read_capped(&mut flate2::read::ZlibDecoder::new(body.as_slice()))
            .or_else(|_| read_capped(&mut flate2::read::DeflateDecoder::new(body.as_slice()))),
        // An encoding we cannot decode would reach the page as garbage.
        Some(_) => Err("Failed"),
    }
}

fn is_local_scheme(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    lower.starts_with("data:") || lower.starts_with("blob:") || lower.starts_with("about:")
}

/// Host only, so a blocked URL's path and query are not written to logs.
pub(crate) fn host_for_log(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "<unparsed>".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::dns::{Addrs, Name, Resolving};
    use std::net::SocketAddr;
    use wiremock::matchers::{body_string, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn paused(url: &str) -> Value {
        json!({
            "requestId": "interception-job-1.0",
            "request": { "url": url, "method": "GET", "headers": { "Accept": "*/*" } }
        })
    }

    /// Resolver that answers every name with one fixed address, standing in
    /// for an attacker-controlled DNS server.
    struct FixedResolver(std::net::IpAddr);

    impl Resolve for FixedResolver {
        fn resolve(&self, _name: Name) -> Resolving {
            let addr = SocketAddr::new(self.0, 0);
            Box::pin(async move { Ok(Box::new(vec![addr].into_iter()) as Addrs) })
        }
    }

    fn rebinding_egress(ip: &str) -> Arc<BrowserEgress> {
        let upstream: Arc<dyn Resolve> = Arc::new(FixedResolver(ip.parse().unwrap()));
        BrowserEgress::with_resolver(None, Arc::new(SsrfGuardResolver::wrapping(upstream)))
    }

    fn assert_failed(answer: &PausedAnswer, reason: &str) {
        assert_eq!(answer.method, "Fetch.failRequest", "{answer:?}");
        assert_eq!(answer.params["errorReason"], reason, "{answer:?}");
        assert_eq!(answer.params["requestId"], "interception-job-1.0");
    }

    #[tokio::test]
    async fn private_loopback_link_local_and_metadata_hops_are_failed_without_a_request() {
        let egress = BrowserEgress::new(None);
        for url in [
            "http://127.0.0.1:8080/admin",
            "http://localhost/",
            "http://10.0.0.5/internal",
            "http://192.168.1.1/",
            "http://172.16.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://metadata.google.internal/computeMetadata/v1/",
            "http://[::1]/",
            "http://[fd00::1]/",
            "http://2130706433/",
            "file:///etc/passwd",
            "ftp://example.com/",
        ] {
            let answer = egress.answer(&paused(url)).await.expect("answer");
            assert_failed(&answer, "BlockedByClient");
        }
    }

    #[tokio::test]
    async fn hostnames_that_resolve_or_rebind_to_internal_addresses_are_refused_at_connect() {
        for ip in ["127.0.0.1", "10.1.2.3", "169.254.169.254", "fd00::1", "::1"] {
            let egress = rebinding_egress(ip);
            let answer = egress
                .answer(&paused("http://rebind.attacker.test/secret"))
                .await
                .expect("answer");
            assert_failed(&answer, "AddressUnreachable");
        }
    }

    #[tokio::test]
    async fn session_access_list_applies_to_every_hop() {
        let egress = BrowserEgress::new(Some(NetworkAccessList::allow_only(["example.com"])));
        let answer = egress
            .answer(&paused("https://evil.test/after-redirect"))
            .await
            .expect("answer");
        assert_failed(&answer, "BlockedByClient");
    }

    #[tokio::test]
    async fn host_system_allowlist_applies_to_page_requests() {
        let mut egress = BrowserEgress::allowing_loopback_for_tests(None);
        egress.system_allowlist = Some(SystemAllowlist::embedded());
        let answer = egress
            .answer(&paused("https://tenant-controlled.example/"))
            .await
            .expect("answer");
        assert_failed(&answer, "BlockedByClient");
    }

    #[tokio::test]
    async fn local_documents_continue_and_missing_ids_are_ignored() {
        let egress = BrowserEgress::new(Some(NetworkAccessList::allow_only(["example.com"])));
        let answer = egress
            .answer(&paused("data:text/html,hi"))
            .await
            .expect("answer");
        assert_eq!(answer.method, "Fetch.continueRequest");
        assert!(
            egress
                .answer(&json!({ "request": { "url": "https://example.com/" } }))
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn redirects_are_returned_to_the_browser_not_followed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/start"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", "http://169.254.169.254/latest/meta-data/"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let egress = BrowserEgress::allowing_loopback_for_tests(None);
        let answer = egress
            .answer(&paused(&format!("{}/start", server.uri())))
            .await
            .expect("answer");
        assert_eq!(answer.method, "Fetch.fulfillRequest");
        assert_eq!(answer.params["responseCode"], 302);
        let headers = answer.params["responseHeaders"].as_array().unwrap();
        assert!(
            headers.iter().any(|h| h["name"] == "location"
                && h["value"] == "http://169.254.169.254/latest/meta-data/")
        );

        // The browser's next hop is a new paused request, checked again.
        let next = BrowserEgress::new(None)
            .answer(&paused("http://169.254.169.254/latest/meta-data/"))
            .await
            .expect("answer");
        assert_failed(&next, "BlockedByClient");
    }

    #[tokio::test]
    async fn relays_method_headers_body_and_multiple_cookies() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/login"))
            .and(header("cookie", "a=1"))
            .and(header("accept-encoding", "identity"))
            .and(body_string("user=x&pw=y"))
            .respond_with(
                ResponseTemplate::new(200)
                    .append_header("Set-Cookie", "s=1; Path=/")
                    .append_header("Set-Cookie", "t=2; Path=/")
                    .insert_header("Content-Type", "text/html")
                    .set_body_string("<p>ok</p>"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let egress = BrowserEgress::allowing_loopback_for_tests(None);
        let answer = egress
            .answer(&json!({
                "requestId": "r1",
                "request": {
                    "url": format!("{}/login", server.uri()),
                    "method": "POST",
                    "headers": { "Cookie": "a=1", "Accept-Encoding": "gzip, br", "Host": "evil" },
                    "hasPostData": true,
                    "postDataEntries": [{ "bytes": base64::engine::general_purpose::STANDARD.encode("user=x&pw=y") }]
                }
            }))
            .await
            .expect("answer");
        assert_eq!(answer.method, "Fetch.fulfillRequest", "{answer:?}");
        let cookies: Vec<_> = answer.params["responseHeaders"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|h| h["name"] == "set-cookie")
            .map(|h| h["value"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(cookies, vec!["s=1; Path=/", "t=2; Path=/"]);
        let body = base64::engine::general_purpose::STANDARD
            .decode(answer.params["body"].as_str().unwrap())
            .unwrap();
        assert_eq!(body, b"<p>ok</p>");
    }

    #[tokio::test]
    async fn compressed_bodies_are_decoded_and_oversized_bodies_fail() {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(b"<title>zipped</title>").unwrap();
        let gz = encoder.finish().unwrap();

        let server = MockServer::start().await;
        Mock::given(path("/gz"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Encoding", "gzip")
                    .set_body_bytes(gz),
            )
            .mount(&server)
            .await;
        Mock::given(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; MAX_BODY_BYTES + 1]))
            .mount(&server)
            .await;
        Mock::given(path("/br"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Encoding", "br")
                    .set_body_bytes(b"???".to_vec()),
            )
            .mount(&server)
            .await;

        let egress = BrowserEgress::allowing_loopback_for_tests(None);
        let answer = egress
            .answer(&paused(&format!("{}/gz", server.uri())))
            .await
            .unwrap();
        assert!(
            !answer.params["responseHeaders"]
                .as_array()
                .unwrap()
                .iter()
                .any(|h| h["name"] == "content-encoding")
        );
        let body = base64::engine::general_purpose::STANDARD
            .decode(answer.params["body"].as_str().unwrap())
            .unwrap();
        assert_eq!(body, b"<title>zipped</title>");

        for route in ["/big", "/br"] {
            let answer = egress
                .answer(&paused(&format!("{}{route}", server.uri())))
                .await
                .unwrap();
            assert_failed_any(&answer);
        }
    }

    fn assert_failed_any(answer: &PausedAnswer) {
        assert!(answer.is_failure(), "{answer:?}");
    }
}

#[cfg(test)]
#[path = "browser_egress_e2e_tests.rs"]
mod e2e_tests;

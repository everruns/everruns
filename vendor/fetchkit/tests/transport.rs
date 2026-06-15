//! Integration tests for the pluggable HTTP transport.
//!
//! A custom in-memory transport proves fetchers route HTTP through the injected
//! transport and never touch the network, that pinned addrs flow from DNS policy,
//! and that DefaultFetcher's streaming caps still apply to a custom transport body.

use async_trait::async_trait;
use bytes::Bytes;
use fetchkit::{
    DnsPolicy, FetchOptions, FetchRequest, Fetcher, HttpMethod, HttpTransport, TransportError,
    TransportMethod, TransportRequest, TransportResponse,
};
use std::sync::{Arc, Mutex};

/// One canned response keyed by URL substring.
#[derive(Clone)]
struct Canned {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// In-memory transport. Records every request and replies with canned responses.
/// Connecting to the network would be a bug — this never does.
struct MockTransport {
    /// (url, method, pinned_addrs) recorded per call.
    calls: Mutex<Vec<(String, TransportMethod, Vec<std::net::SocketAddr>)>>,
    /// match-by-substring => response.
    routes: Vec<(String, Canned)>,
}

impl MockTransport {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            routes: Vec::new(),
        }
    }

    fn route(mut self, contains: &str, status: u16, ct: &str, body: &[u8]) -> Self {
        self.routes.push((
            contains.to_string(),
            Canned {
                status,
                headers: vec![("content-type".to_string(), ct.to_string())],
                body: body.to_vec(),
            },
        ));
        self
    }

    fn calls(&self) -> Vec<(String, TransportMethod, Vec<std::net::SocketAddr>)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl HttpTransport for MockTransport {
    async fn execute(&self, req: TransportRequest) -> Result<TransportResponse, TransportError> {
        self.calls.lock().unwrap().push((
            req.url.to_string(),
            req.method,
            req.pinned_addrs.clone(),
        ));

        let canned = self
            .routes
            .iter()
            .find(|(needle, _)| req.url.as_str().contains(needle.as_str()))
            .map(|(_, c)| c.clone())
            .unwrap_or(Canned {
                status: 404,
                headers: vec![("content-type".to_string(), "text/plain".to_string())],
                body: b"not found".to_vec(),
            });

        let body_bytes = Bytes::from(canned.body);
        let stream = futures::stream::once(async move { Ok(body_bytes) });

        Ok(TransportResponse {
            status: canned.status,
            url: req.url,
            headers: canned.headers,
            body: Box::pin(stream),
        })
    }
}

fn options_with(transport: Arc<dyn HttpTransport>) -> FetchOptions {
    FetchOptions {
        enable_markdown: true,
        enable_text: true,
        // allow_all => no DNS resolution; pinned_addrs stays empty so the mock host
        // does not need to resolve.
        dns_policy: DnsPolicy::allow_all(),
        transport: Some(transport),
        ..Default::default()
    }
}

#[tokio::test]
async fn default_fetcher_get_uses_injected_transport() {
    let mock = Arc::new(MockTransport::new().route(
        "example.test/page",
        200,
        "text/plain",
        b"hello via transport",
    ));
    let options = options_with(mock.clone());

    let fetcher = fetchkit::DefaultFetcher::new();
    let request = FetchRequest::new("https://example.test/page");
    let response = fetcher.fetch(&request, &options).await.unwrap();

    assert_eq!(response.status_code, 200);
    assert_eq!(response.content.as_deref(), Some("hello via transport"));

    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, TransportMethod::Get);
    assert!(calls[0].0.contains("example.test/page"));
}

#[tokio::test]
async fn default_fetcher_head_uses_injected_transport() {
    let mock = Arc::new(MockTransport::new().route("example.test/head", 200, "text/html", b""));
    let options = options_with(mock.clone());

    let fetcher = fetchkit::DefaultFetcher::new();
    let request = FetchRequest::new("https://example.test/head").method(HttpMethod::Head);
    let response = fetcher.fetch(&request, &options).await.unwrap();

    assert_eq!(response.status_code, 200);
    assert_eq!(response.method.as_deref(), Some("HEAD"));
    assert!(response.content.is_none());

    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, TransportMethod::Head);
}

#[tokio::test]
async fn default_fetcher_follows_redirect_through_transport_with_per_hop_validation() {
    // First hop redirects; transport returns a 302 with Location. fetchkit must follow
    // it (per-hop) through the transport and land on the final response.
    // The shared `route` helper only sets content-type, so use a bespoke transport that
    // can emit a Location header.
    struct RedirectTransport {
        calls: Mutex<Vec<String>>,
    }
    #[async_trait]
    impl HttpTransport for RedirectTransport {
        async fn execute(
            &self,
            req: TransportRequest,
        ) -> Result<TransportResponse, TransportError> {
            self.calls.lock().unwrap().push(req.url.to_string());
            if req.url.path() == "/start" {
                return Ok(TransportResponse {
                    status: 302,
                    url: req.url.clone(),
                    headers: vec![(
                        "location".to_string(),
                        "https://example.test/final".to_string(),
                    )],
                    body: Box::pin(futures::stream::once(async { Ok(Bytes::new()) })),
                });
            }
            Ok(TransportResponse {
                status: 200,
                url: req.url,
                headers: vec![("content-type".to_string(), "text/plain".to_string())],
                body: Box::pin(futures::stream::once(async {
                    Ok(Bytes::from_static(b"arrived"))
                })),
            })
        }
    }
    let redirect = Arc::new(RedirectTransport {
        calls: Mutex::new(Vec::new()),
    });
    let options = options_with(redirect.clone());

    let fetcher = fetchkit::DefaultFetcher::new();
    let request = FetchRequest::new("https://example.test/start");
    let response = fetcher.fetch(&request, &options).await.unwrap();

    assert_eq!(response.status_code, 200);
    assert_eq!(response.content.as_deref(), Some("arrived"));
    assert_eq!(response.redirect_chain.len(), 1);

    let calls = redirect.calls.lock().unwrap();
    assert_eq!(calls.len(), 2, "should follow exactly one redirect hop");
    assert!(calls[0].contains("/start"));
    assert!(calls[1].contains("/final"));
}

#[tokio::test]
async fn default_fetcher_enforces_body_cap_on_transport_stream() {
    // Body cap must apply to the transport's streamed bytes.
    let big = vec![b'a'; 50];
    let mock = Arc::new(MockTransport::new().route("example.test/big", 200, "text/plain", &big));
    let mut options = options_with(mock.clone());
    options.max_body_size = Some(10);

    let fetcher = fetchkit::DefaultFetcher::new();
    let request = FetchRequest::new("https://example.test/big");
    let response = fetcher.fetch(&request, &options).await.unwrap();

    assert_eq!(response.status_code, 200);
    assert_eq!(response.truncated, Some(true));
    let content = response.content.unwrap();
    // 10 captured bytes + truncation marker.
    assert!(content.starts_with("aaaaaaaaaa"));
    assert!(content.contains("content truncated"));
    assert_eq!(response.size, Some(10));
}

#[tokio::test]
async fn wikipedia_fetcher_uses_injected_transport() {
    let summary = br#"{"title":"Rust","extract":"A language.","description":"PL"}"#;
    let mock = Arc::new(
        MockTransport::new()
            .route("/page/summary/", 200, "application/json", summary)
            .route("/page/html/", 200, "text/html", b"<p>Body</p>"),
    );
    let options = options_with(mock.clone());

    let fetcher = fetchkit::WikipediaFetcher::new();
    let request = FetchRequest::new("https://en.wikipedia.org/wiki/Rust");
    let response = fetcher.fetch(&request, &options).await.unwrap();

    assert_eq!(response.status_code, 200);
    let content = response.content.unwrap();
    assert!(content.contains("# Rust"));

    // Both the summary and html API endpoints went through the mock transport — no network.
    let calls = mock.calls();
    assert!(calls.iter().any(|(u, _, _)| u.contains("/page/summary/")));
    assert!(calls.iter().any(|(u, _, _)| u.contains("/page/html/")));
}

#[tokio::test]
async fn tool_execute_uses_injected_transport() {
    let mock = Arc::new(MockTransport::new().route(
        "example.test/tool",
        200,
        "text/plain",
        b"tool via transport",
    ));

    let tool = fetchkit::Tool::builder()
        // allow_all => no DNS resolution, so the fake host never resolves.
        .block_private_ips(false)
        .transport(mock.clone())
        .build();

    // Manual Debug must render without exposing the trait object.
    assert!(format!("{tool:?}").contains("transport: \"<custom>\""));

    let response = tool
        .execute(FetchRequest::new("https://example.test/tool"))
        .await
        .unwrap();

    assert_eq!(response.status_code, 200);
    assert_eq!(response.content.as_deref(), Some("tool via transport"));

    // Exactly one HTTP exchange, all through the mock — no network.
    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, TransportMethod::Get);
    assert!(calls[0].0.contains("example.test/tool"));
}

#[tokio::test]
async fn tool_execute_with_saver_uses_injected_transport() {
    let mock = Arc::new(MockTransport::new().route(
        "example.test/file.bin",
        200,
        "application/octet-stream",
        b"binary via transport",
    ));

    let tool = fetchkit::Tool::builder()
        .block_private_ips(false)
        .enable_save_to_file(true)
        .transport(mock.clone())
        .build();

    let dir = tempfile::tempdir().unwrap();
    let saver = fetchkit::LocalFileSaver::new(Some(dir.path().to_path_buf()));

    let request = FetchRequest::new("https://example.test/file.bin").save_to_file("payload.bin");
    let response = tool
        .execute_with_saver(request, Some(&saver))
        .await
        .unwrap();

    assert_eq!(response.status_code, 200);
    assert_eq!(response.bytes_written, Some(20));
    let saved_path = response.saved_path.expect("saved_path must be set");
    assert_eq!(std::fs::read(&saved_path).unwrap(), b"binary via transport");

    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].0.contains("example.test/file.bin"));
}

#[test]
fn dns_policy_populates_pinned_addrs_for_resolved_host() {
    // block_private resolves and validates, producing a pinned addr for a public host.
    let policy = DnsPolicy::block_private_ips();
    // 127.0.0.1 resolves to loopback and is blocked => no pinned addr (error).
    assert!(policy.pinned_addrs("127.0.0.1", 80).is_err());

    // A literal public IP resolves to itself and is allowed => exactly one pinned addr.
    let addrs = policy
        .pinned_addrs("93.184.216.34", 443)
        .expect("public IP should validate");
    assert_eq!(addrs.len(), 1);
    assert_eq!(addrs[0].ip().to_string(), "93.184.216.34");
    assert_eq!(addrs[0].port(), 443);

    // allow_all does not resolve => empty pinned addrs (transport may resolve itself).
    let permissive = DnsPolicy::allow_all();
    assert!(permissive
        .pinned_addrs("example.com", 443)
        .unwrap()
        .is_empty());
}

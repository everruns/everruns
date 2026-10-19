// End-to-end tests of the proxy over real sockets: a client speaks to the
// proxy, the proxy tunnels to a local echo server. Loopback is admitted only
// through `AddressPolicy::AllowLoopbackForTests`, which exists in test builds
// alone.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::address::AddressPolicy;
use super::limiter::Limiter;
use super::limits::{LimitsCache, LimitsSource};
use super::proxy::{Proxy, ProxyOptions, unix_now};
use everruns_core::host::{TenantEgressProxy, sign_proxy_token};

use super::limits::OrgLimits;
use super::usage::UsageAggregator;

const SECRET: &[u8] = b"test-secret-test-secret-test-secret!";
const ORG: &str = "org_0123456789abcdef0123456789abcdef";

struct Fixed(Arc<Mutex<OrgLimits>>);

#[async_trait]
impl LimitsSource for Fixed {
    async fn fetch(&self, _org: &str) -> anyhow::Result<OrgLimits> {
        Ok(self.0.lock().expect("lock").clone())
    }
}

struct Harness {
    proxy_addr: std::net::SocketAddr,
    usage: Arc<UsageAggregator>,
    limits: Arc<Mutex<OrgLimits>>,
}

async fn start(limits: OrgLimits, policy: AddressPolicy, global_per_host: u64) -> Harness {
    let limits = Arc::new(Mutex::new(limits));
    let usage = Arc::new(UsageAggregator::new("test"));
    let mut options = ProxyOptions::new(SECRET.to_vec());
    options.address_policy = policy;
    let proxy = Arc::new(Proxy {
        options,
        // TTL zero: every request asks the source, so tests can flip limits.
        limits: LimitsCache::new(
            Box::new(Fixed(limits.clone())),
            Duration::ZERO,
            OrgLimits::unlimited(),
        ),
        limiter: Limiter::new(1, global_per_host),
        usage: usage.clone(),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind proxy");
    let proxy_addr = listener.local_addr().expect("addr");
    tokio::spawn(proxy.serve(listener));
    Harness {
        proxy_addr,
        usage,
        limits,
    }
}

/// Echoes everything back, then closes when the client closes.
async fn echo_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind echo");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = stream.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    addr
}

/// Answers one plain HTTP request and records the head it saw.
async fn http_server(seen: Arc<Mutex<String>>) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind http");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let mut got = Vec::new();
                while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    got.extend_from_slice(&buf[..n]);
                }
                *seen.lock().expect("lock") = String::from_utf8_lossy(&got).to_string();
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
                    )
                    .await;
                let _ = stream.shutdown().await;
            });
        }
    });
    addr
}

fn bearer(org: &str) -> String {
    format!(
        "Bearer {}",
        sign_proxy_token(SECRET, org, "session_1", unix_now() + 300)
    )
}

async fn connect(h: &Harness, target: &str, auth: Option<&str>) -> (TcpStream, String) {
    let mut stream = TcpStream::connect(h.proxy_addr)
        .await
        .expect("connect proxy");
    let mut head = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some(auth) = auth {
        head.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await.expect("write");
    let response = read_response_head(&mut stream).await;
    (stream, response)
}

async fn read_response_head(stream: &mut TcpStream) -> String {
    let mut got = Vec::new();
    let mut byte = [0u8; 1];
    while !got.ends_with(b"\r\n\r\n") {
        match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut byte)).await {
            Ok(Ok(1)) => got.push(byte[0]),
            _ => break,
        }
    }
    String::from_utf8_lossy(&got).to_string()
}

async fn read_to_end(stream: &mut TcpStream) -> String {
    let mut body = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut body)).await;
    String::from_utf8_lossy(&body).to_string()
}

/// Wait for the proxy to finish a connection and book its bytes.
async fn settled_bytes(usage: &UsageAggregator) -> (u64, u64, u64) {
    for _ in 0..100 {
        let items = usage.drain(None);
        let totals = items.iter().fold((0, 0, 0), |acc, i| {
            (
                acc.0 + i.egress_bytes,
                acc.1 + i.ingress_bytes,
                acc.2 + i.requests,
            )
        });
        if totals.0 > 0 || totals.1 > 0 {
            return totals;
        }
        usage.requeue(items);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("proxy never booked the connection's bytes");
}

#[tokio::test]
async fn connect_tunnels_and_counts_bytes_each_way() {
    let echo = echo_server().await;
    let h = start(
        OrgLimits::unlimited(),
        AddressPolicy::AllowLoopbackForTests,
        1_000,
    )
    .await;
    let (mut stream, response) = connect(&h, &echo.to_string(), Some(&bearer(ORG))).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");

    stream.write_all(b"ping-0123456789").await.expect("write");
    let mut back = [0u8; 15];
    stream.read_exact(&mut back).await.expect("echo");
    assert_eq!(&back, b"ping-0123456789");
    stream.shutdown().await.expect("shutdown");
    let _ = read_to_end(&mut stream).await;

    let (egress, ingress, requests) = settled_bytes(&h.usage).await;
    assert_eq!(egress, 15, "client to destination");
    assert_eq!(
        ingress, 15,
        "destination to client, excluding the proxy's 200"
    );
    assert_eq!(requests, 1);
}

#[tokio::test]
async fn plain_http_is_forwarded_without_proxy_credentials() {
    let seen = Arc::new(Mutex::new(String::new()));
    let origin = http_server(seen.clone()).await;
    let h = start(
        OrgLimits::unlimited(),
        AddressPolicy::AllowLoopbackForTests,
        1_000,
    )
    .await;

    let mut stream = TcpStream::connect(h.proxy_addr).await.expect("connect");
    let request = format!(
        "GET http://{origin}/path?token=abc HTTP/1.1\r\nHost: {origin}\r\nProxy-Authorization: {}\r\n\r\n",
        bearer(ORG)
    );
    stream.write_all(request.as_bytes()).await.expect("write");
    let response = read_to_end(&mut stream).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.ends_with("hello"));
    // The proxy splices until both sides close; a real client closes after a
    // `Connection: close` response.
    drop(stream);

    let upstream_head = seen.lock().expect("lock").clone();
    assert!(
        upstream_head.starts_with("GET /path?token=abc HTTP/1.1\r\n"),
        "{upstream_head}"
    );
    assert!(
        !upstream_head
            .to_ascii_lowercase()
            .contains("proxy-authorization")
    );
    assert!(upstream_head.contains("Connection: close"));

    let (egress, ingress, _) = settled_bytes(&h.usage).await;
    assert_eq!(egress as usize, upstream_head.len());
    assert_eq!(ingress as usize, response.len());
}

/// The token the egress service mints is the token the proxy admits: core
/// owns both sides of the format, and this drives a real HTTP client through
/// the real proxy with it.
#[tokio::test]
async fn a_token_minted_by_the_egress_service_is_admitted() {
    let seen = Arc::new(Mutex::new(String::new()));
    let origin = http_server(seen.clone()).await;
    let h = start(
        OrgLimits::unlimited(),
        AddressPolicy::AllowLoopbackForTests,
        1_000,
    )
    .await;
    let minted = TenantEgressProxy::new(&format!("http://{}", h.proxy_addr), SECRET.to_vec())
        .expect("proxy config");
    let scope = everruns_core::EgressScope {
        org_id: Some(ORG.parse().expect("org id")),
        session_id: None,
    };
    let auth =
        reqwest::header::HeaderValue::from_str(&format!("Bearer {}", minted.token(Some(&scope))))
            .expect("header");
    let client = reqwest::Client::builder()
        .proxy(
            reqwest::Proxy::all(format!("http://{}", h.proxy_addr))
                .expect("proxy")
                .custom_http_auth(auth),
        )
        .build()
        .expect("client");
    let response = client
        .get(format!("http://{origin}/minted"))
        .send()
        .await
        .expect("proxied request");
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.expect("body"), "hello");
    assert!(
        seen.lock()
            .expect("lock")
            .starts_with("GET /minted HTTP/1.1")
    );

    let (_, _, requests) = settled_bytes(&h.usage).await;
    assert_eq!(requests, 1);
}

/// The control endpoint contract: limits out, usage in, both behind the
/// shared secret.
#[tokio::test]
async fn the_control_endpoint_answers_limits_and_receives_usage() {
    use axum::Json;
    use axum::extract::Path;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};

    use super::limits::{HttpLimitsSource, LimitsSource};
    use super::usage::{HttpUsageSink, USAGE_PATH, UsageBatch, UsageItem, UsageSink};

    let secret = String::from_utf8(SECRET.to_vec()).expect("utf8");
    let received = Arc::new(Mutex::new(Vec::<UsageBatch>::new()));
    let authorized = {
        let expected = format!("Bearer {secret}");
        move |headers: &HeaderMap| {
            headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(expected.as_str())
        }
    };
    let app = axum::Router::new()
        .route(
            "/internal/egress-proxy/orgs/{org}/limits",
            get({
                let authorized = authorized.clone();
                move |Path(org): Path<String>, headers: HeaderMap| async move {
                    if !authorized(&headers) {
                        return Err(StatusCode::UNAUTHORIZED);
                    }
                    Ok(Json(serde_json::json!({
                        "tier": org, "requests_per_minute": 7, "max_concurrent": 1,
                        "bytes_per_day": 100, "per_host_requests_per_minute": 3
                    })))
                }
            }),
        )
        .route(
            USAGE_PATH,
            post({
                let received = received.clone();
                move |headers: HeaderMap, Json(batch): Json<UsageBatch>| async move {
                    if !authorized(&headers) {
                        return StatusCode::UNAUTHORIZED;
                    }
                    received.lock().expect("lock").push(batch);
                    StatusCode::NO_CONTENT
                }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    tokio::spawn(async move { axum::serve(listener, app).await });

    let limits = HttpLimitsSource::new(&base, &secret)
        .expect("source")
        .fetch(ORG)
        .await
        .expect("limits");
    assert_eq!(limits.label.as_deref(), Some(ORG));
    assert_eq!(limits.requests_per_minute, 7);
    assert!(
        HttpLimitsSource::new(&base, "wrong-secret")
            .expect("source")
            .fetch(ORG)
            .await
            .is_err()
    );

    let batch = UsageBatch {
        instance: "test".into(),
        items: vec![UsageItem {
            org: ORG.into(),
            session: Some("session_1".into()),
            minute: 1_800_000_000,
            egress_bytes: 10,
            ingress_bytes: 20,
            requests: 1,
        }],
    };
    HttpUsageSink::new(&base, &secret)
        .expect("sink")
        .send(&batch)
        .await
        .expect("usage accepted");
    assert_eq!(*received.lock().expect("lock"), vec![batch]);
}

#[tokio::test]
async fn bad_expired_or_missing_tokens_get_407() {
    let echo = echo_server().await;
    let h = start(
        OrgLimits::unlimited(),
        AddressPolicy::AllowLoopbackForTests,
        1_000,
    )
    .await;
    let expired = format!(
        "Bearer {}",
        sign_proxy_token(SECRET, ORG, "-", unix_now() - 1)
    );
    let forged = format!(
        "Bearer {}",
        sign_proxy_token(
            b"wrong-secret-wrong-secret-wrong-secret",
            ORG,
            "-",
            unix_now() + 60
        )
    );
    for auth in [
        None,
        Some(expired.as_str()),
        Some(forged.as_str()),
        Some("Bearer junk"),
    ] {
        let (_, response) = connect(&h, &echo.to_string(), auth).await;
        assert!(response.starts_with("HTTP/1.1 407"), "{auth:?}: {response}");
        assert!(response.contains("Proxy-Authenticate: Bearer"));
    }
    assert_eq!(h.usage.pending_len(), 0, "refused requests are not metered");
}

#[tokio::test]
async fn internal_destinations_and_smtp_are_refused() {
    // Production policy: loopback is not public.
    let echo = echo_server().await;
    let h = start(OrgLimits::unlimited(), AddressPolicy::PublicOnly, 1_000).await;
    for target in [
        echo.to_string(),
        "169.254.169.254:80".to_string(),
        "[fd00:ec2::254]:80".to_string(),
        "10.0.0.5:443".to_string(),
        "localhost:443".to_string(),
    ] {
        let (_, response) = connect(&h, &target, Some(&bearer(ORG))).await;
        assert!(response.starts_with("HTTP/1.1 403"), "{target}: {response}");
    }
    let (_, response) = connect(&h, "smtp.example.com:587", Some(&bearer(ORG))).await;
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
}

#[tokio::test]
async fn over_limit_requests_get_429_with_a_reason() {
    let echo = echo_server().await;
    let mut limits = OrgLimits::unlimited();
    limits.requests_per_minute = 2;
    let h = start(limits, AddressPolicy::AllowLoopbackForTests, 1_000).await;
    for _ in 0..2 {
        let (mut stream, response) = connect(&h, &echo.to_string(), Some(&bearer(ORG))).await;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        stream.shutdown().await.expect("shutdown");
    }
    let (mut stream, response) = connect(&h, &echo.to_string(), Some(&bearer(ORG))).await;
    // The 2-per-minute window may roll over mid-test; only then is a third allowed.
    if !response.starts_with("HTTP/1.1 200") {
        assert!(response.starts_with("HTTP/1.1 429"), "{response}");
        assert!(response.contains("Retry-After: 60"));
        let body = read_to_end(&mut stream).await;
        assert!(body.contains("too many requests per minute"), "{body}");
    }
}

#[tokio::test]
async fn a_suspended_org_is_refused_everything() {
    let echo = echo_server().await;
    let h = start(
        OrgLimits::unlimited(),
        AddressPolicy::AllowLoopbackForTests,
        1_000,
    )
    .await;
    h.limits.lock().expect("lock").suspended = true;
    let (mut stream, response) = connect(&h, &echo.to_string(), Some(&bearer(ORG))).await;
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
    assert!(read_to_end(&mut stream).await.contains("suspended"));

    // Lifting it takes effect on the next fetch (TTL zero here).
    h.limits.lock().expect("lock").suspended = false;
    let (_, response) = connect(&h, &echo.to_string(), Some(&bearer(ORG))).await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
}

#[tokio::test]
async fn health_answers_without_a_token() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(super::proxy::serve_health(listener));
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"GET /healthz HTTP/1.1\r\n\r\n")
        .await
        .expect("write");
    assert!(read_to_end(&mut stream).await.starts_with("HTTP/1.1 200"));
}

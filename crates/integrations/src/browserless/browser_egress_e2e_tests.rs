//! EVE-1189: the network guard against a real headless Chromium.
//!
//! Each test stands up an "internal" service on loopback that counts every TCP
//! connection, then drives a guarded browser at it through a redirect, a DNS
//! answer that points at it, page scripts, and a WebSocket. The browser's own
//! resolver is also rigged (`--host-resolver-rules`) to answer the attacker
//! hostname with loopback, so a request the browser made by itself would land.
//! Skips when no Chromium is installed (set `CHROMIUM_PATH` to run).

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use everruns_contracts::driver_helpers::SsrfGuardResolver;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use crate::browserless::browser_egress::BrowserEgress;
use crate::browserless::test_chromium::{connect_guarded, launch_chromium_with_args};

const SECRET: &str = "INTERNAL-SECRET-7f3a";

/// A loopback service standing in for an internal admin API or the metadata
/// endpoint. Counts connections of any kind (HTTP, WebSocket upgrade).
struct InternalService {
    port: u16,
    connections: Arc<AtomicUsize>,
}

impl InternalService {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let connections = Arc::new(AtomicUsize::new(0));
        let counter = connections.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf).await;
                    let body = SECRET;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\
                         Access-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        Self { port, connections }
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

/// Everruns' resolver answer for every name, as an attacker's DNS would give.
struct FixedResolver(std::net::IpAddr);

impl Resolve for FixedResolver {
    fn resolve(&self, _name: Name) -> Resolving {
        let addr = SocketAddr::new(self.0, 0);
        Box::pin(async move { Ok(Box::new(vec![addr].into_iter()) as Addrs) })
    }
}

fn rebinding_egress() -> Arc<BrowserEgress> {
    let upstream: Arc<dyn Resolve> = Arc::new(FixedResolver("127.0.0.1".parse().unwrap()));
    BrowserEgress::with_resolver(None, Arc::new(SsrfGuardResolver::wrapping(upstream)))
}

/// The browser's own DNS answers the attacker hostname with loopback.
const BROWSER_REBINDS: &str = "--host-resolver-rules=MAP rebind.attacker.test 127.0.0.1";

async fn page_text(session: &mut crate::browserless::cdp::CdpSession) -> String {
    session
        .evaluate("document.documentElement ? document.documentElement.outerHTML : ''")
        .await
        .ok()
        .and_then(|value| value["result"]["value"].as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

#[tokio::test]
async fn dns_rebinding_to_loopback_never_reaches_the_internal_service() {
    let Some(browser) = launch_chromium_with_args(&[BROWSER_REBINDS]).await else {
        eprintln!("skipping: no local Chromium found (set CHROMIUM_PATH to run)");
        return;
    };
    let internal = InternalService::start().await;
    let mut session = connect_guarded(&browser, rebinding_egress()).await;

    let error = session
        .navigate(&format!(
            "http://rebind.attacker.test:{}/latest/meta-data/",
            internal.port
        ))
        .await
        .expect_err("a hostname answering with loopback must not load");
    assert!(error.contains("ERR_ADDRESS_UNREACHABLE"), "{error}");
    assert!(!page_text(&mut session).await.contains(SECRET));
    assert_eq!(
        internal.connections(),
        0,
        "the internal service was reached"
    );
    session.disconnect().await;
}

#[tokio::test]
async fn redirect_hops_to_internal_hosts_are_blocked() {
    let Some(browser) = launch_chromium_with_args(&[BROWSER_REBINDS]).await else {
        eprintln!("skipping: no local Chromium found (set CHROMIUM_PATH to run)");
        return;
    };
    let internal = InternalService::start().await;
    let public = MockServer::start().await;
    for (route, target) in [
        (
            "/to-localhost",
            format!("http://localhost:{}/admin", internal.port),
        ),
        (
            "/to-metadata",
            "http://169.254.169.254/latest/meta-data/".to_string(),
        ),
        ("/to-private", "http://10.0.0.5/admin".to_string()),
        (
            "/to-rebinding-host",
            format!("http://rebind.attacker.test:{}/admin", internal.port),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", target.as_str()))
            .mount(&public)
            .await;
    }

    // Only the loopback "public" mock server is reachable; DNS answers loopback.
    let egress = Arc::new(BrowserEgress::allowing_loopback_for_tests_with_resolver(
        Arc::new(SsrfGuardResolver::wrapping(Arc::new(FixedResolver(
            "127.0.0.1".parse().unwrap(),
        )))),
    ));
    let mut session = connect_guarded(&browser, egress).await;
    for (route, expected) in [
        ("/to-localhost", "ERR_BLOCKED_BY_CLIENT"),
        ("/to-metadata", "ERR_BLOCKED_BY_CLIENT"),
        ("/to-private", "ERR_BLOCKED_BY_CLIENT"),
        ("/to-rebinding-host", "ERR_ADDRESS_UNREACHABLE"),
    ] {
        let error = session
            .navigate(&format!("{}{route}", public.uri()))
            .await
            .expect_err("a redirect to an internal host must fail the navigation");
        assert!(error.contains(expected), "{route}: {error}");
        assert!(!page_text(&mut session).await.contains(SECRET));
    }
    assert_eq!(
        internal.connections(),
        0,
        "the internal service was reached"
    );
    session.disconnect().await;
}

#[tokio::test]
async fn page_scripts_and_websockets_cannot_reach_internal_hosts_even_after_detach() {
    let Some(browser) = launch_chromium_with_args(&[BROWSER_REBINDS]).await else {
        eprintln!("skipping: no local Chromium found (set CHROMIUM_PATH to run)");
        return;
    };
    let internal = InternalService::start().await;
    let public = MockServer::start().await;
    let port = internal.port;
    let page = format!(
        r#"<!doctype html><title>attacker</title><body><p id="out">waiting</p><script>
        document.title = 'scripts-ran';
        const hit = (u) => fetch(u).then(r => r.text()).then(t => {{ document.getElementById('out').textContent = t; }}).catch(() => {{}});
        hit('http://localhost:{port}/secret');
        hit('http://rebind.attacker.test:{port}/secret');
        try {{ new WebSocket('ws://127.0.0.1:{port}/ws'); }} catch (e) {{}}
        setInterval(() => {{ hit('http://rebind.attacker.test:{port}/beacon'); try {{ new WebSocket('ws://127.0.0.1:{port}/ws'); }} catch (e) {{}} }}, 100);
        </script></body>"#
    );
    Mock::given(method("GET"))
        .and(path("/page"))
        .respond_with(
            // `set_body_raw` keeps text/html; `set_body_string` would force text/plain.
            ResponseTemplate::new(200).set_body_raw(page, "text/html"),
        )
        .mount(&public)
        .await;

    let egress = Arc::new(BrowserEgress::allowing_loopback_for_tests_with_resolver(
        Arc::new(SsrfGuardResolver::wrapping(Arc::new(FixedResolver(
            "127.0.0.1".parse().unwrap(),
        )))),
    ));
    let mut session = connect_guarded(&browser, egress).await;
    session
        .navigate(&format!("{}/page", public.uri()))
        .await
        .expect("the public page loads through Everruns");
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(session.get_title().await.unwrap(), "scripts-ran");
    assert!(!page_text(&mut session).await.contains(SECRET));
    assert_eq!(internal.connections(), 0, "reached while attached");

    // Between tool calls nothing answers Fetch; the page keeps running.
    session.disconnect().await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        internal.connections(),
        0,
        "reached after the client detached"
    );
    drop(browser);
}

/// Echoes the request's Cookie header so the test can see what the browser sent.
struct EchoCookies;

impl Respond for EchoCookies {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let cookies = request
            .headers
            .get("cookie")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        ResponseTemplate::new(200).set_body_raw(
            format!("<title>me</title><p id=\"c\">{cookies}</p>"),
            "text/html",
        )
    }
}

#[tokio::test]
async fn allowed_pages_load_through_everruns_with_cookies_and_subresources() {
    let Some(browser) = launch_chromium_with_args(&[]).await else {
        eprintln!("skipping: no local Chromium found (set CHROMIUM_PATH to run)");
        return;
    };
    let public = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/login"))
        .respond_with(
            ResponseTemplate::new(302)
                .append_header("Set-Cookie", "sid=abc; Path=/")
                .append_header("Set-Cookie", "theme=dark; Path=/")
                .insert_header("Location", "/me"),
        )
        .mount(&public)
        .await;
    Mock::given(method("GET"))
        .and(path("/me"))
        .respond_with(EchoCookies)
        .mount(&public)
        .await;
    let egress = Arc::new(BrowserEgress::allowing_loopback_for_tests(None));
    let mut session = connect_guarded(&browser, egress).await;
    session
        .navigate(&format!("{}/login", public.uri()))
        .await
        .expect("allowed redirect chain loads");
    let url = session.get_url().await.unwrap();
    assert!(url.ends_with("/me"), "{url}");
    let html = page_text(&mut session).await;
    assert!(html.contains("sid=abc"), "{html}");
    assert!(html.contains("theme=dark"), "{html}");
    let cookies = session.context_cookies().await.unwrap();
    assert_eq!(cookies.len(), 2, "{cookies:?}");
    session.disconnect().await;
}

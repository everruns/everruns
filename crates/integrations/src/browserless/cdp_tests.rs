use super::*;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio_tungstenite::accept_async;

/// A Browserless-like CDP server. `target_infos` and `contexts` describe
/// the browser's existing pages and created contexts. Navigating to a URL
/// containing `redirect-test` pauses a redirect hop to the metadata
/// service and reports the client's answer before completing navigation.
async fn spawn_mock_cdp_server_with(
    target_infos: Vec<Value>,
    contexts: Vec<&'static str>,
) -> (
    std::net::SocketAddr,
    UnboundedReceiver<Value>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = unbounded_channel();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut ws = accept_async(stream).await.expect("websocket handshake");
        let mut pending_navigate: Option<u64> = None;

        while let Some(message) = ws.next().await {
            let text = match message.expect("message") {
                Message::Text(text) => text,
                Message::Close(_) => break,
                _ => continue,
            };
            let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
            tx.send(parsed.clone()).ok();

            let id = parsed
                .get("id")
                .and_then(|value| value.as_u64())
                .expect("command id");
            let method = parsed
                .get("method")
                .and_then(|value| value.as_str())
                .expect("method");
            let page_session = parsed
                .get("sessionId")
                .and_then(|value| value.as_str())
                .filter(|session_id| session_id.ends_with("-page-session"));

            let response = match method {
                "Target.createBrowserContext" => {
                    assert_eq!(parsed["params"]["proxyServer"], DEAD_PROXY_SERVER);
                    assert_eq!(parsed["params"]["proxyBypassList"], "<-loopback>");
                    json!({ "id": id, "result": { "browserContextId": "guarded-context" } })
                }
                "Target.getBrowserContexts" => {
                    json!({ "id": id, "result": { "browserContextIds": contexts.clone() } })
                }
                "Target.getTargets" => {
                    json!({ "id": id, "result": { "targetInfos": target_infos.clone() } })
                }
                "Target.createTarget" => {
                    assert_eq!(parsed["params"]["url"], "about:blank");
                    assert!(
                        parsed["params"]["browserContextId"].is_string(),
                        "pages must be created inside a guarded context"
                    );
                    json!({ "id": id, "result": { "targetId": "created-page-target" } })
                }
                "Target.attachToTarget" => {
                    assert_eq!(parsed["params"]["flatten"], true);
                    let target_id = parsed["params"]["targetId"].as_str().expect("targetId");
                    let session_id = match target_id {
                        "existing-page-target" => "existing-page-session",
                        "created-page-target" => "created-page-session",
                        other => panic!("unexpected targetId: {other}"),
                    };
                    json!({ "id": id, "result": { "sessionId": session_id } })
                }
                "Fetch.continueRequest" | "Fetch.failRequest" | "Fetch.fulfillRequest" => {
                    assert!(page_session.is_some(), "Fetch answers use the page session");
                    let ack = json!({ "id": id, "result": {} });
                    ws.send(Message::Text(ack.to_string().into()))
                        .await
                        .expect("ack");
                    if let Some(navigate_id) = pending_navigate.take() {
                        let blocked = method == "Fetch.failRequest";
                        let result = if blocked {
                            json!({ "frameId": "frame-1", "errorText": "net::ERR_BLOCKED_BY_CLIENT" })
                        } else {
                            json!({ "frameId": "frame-1" })
                        };
                        json!({ "id": navigate_id, "result": result })
                    } else {
                        continue;
                    }
                }
                "Page.navigate"
                    if parsed["params"]["url"]
                        .as_str()
                        .is_some_and(|url| url.contains("redirect-test")) =>
                {
                    assert!(page_session.is_some());
                    pending_navigate = Some(id);
                    // The document answered 302; Chrome pauses the next hop.
                    json!({
                        "method": "Fetch.requestPaused",
                        "sessionId": page_session,
                        "params": {
                            "requestId": "interception-job-2.0",
                            "redirectedRequestId": "interception-job-1.0",
                            "resourceType": "Document",
                            "request": {
                                "url": "http://169.254.169.254/latest/meta-data/",
                                "method": "GET",
                                "headers": {}
                            }
                        }
                    })
                }
                "Fetch.enable"
                | "Page.enable"
                | "Page.navigate"
                | "Page.setLifecycleEventsEnabled"
                | "Runtime.evaluate" => match page_session {
                    Some(_) => json!({ "id": id, "result": { "frameId": "frame-1" } }),
                    None => json!({
                        "id": id,
                        "error": { "message": format!("{method} wasn't found") }
                    }),
                },
                "Browserless.reconnect" => {
                    assert!(
                        parsed.get("sessionId").is_none(),
                        "browser-wide commands must not carry a page sessionId"
                    );
                    json!({
                        "id": id,
                        "result": { "browserWSEndpoint": "ws://browserless/reconnect" }
                    })
                }
                other => panic!("unexpected method: {other}"),
            };

            ws.send(Message::Text(response.to_string().into()))
                .await
                .expect("send response");
        }
    });

    (addr, rx, server)
}

async fn spawn_mock_cdp_server(
    target_infos: Vec<Value>,
) -> (
    std::net::SocketAddr,
    UnboundedReceiver<Value>,
    tokio::task::JoinHandle<()>,
) {
    spawn_mock_cdp_server_with(target_infos, vec![]).await
}

fn guard() -> Arc<BrowserEgress> {
    BrowserEgress::new(None)
}

fn methods(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|msg| msg["method"].as_str().expect("method"))
        .collect()
}

fn drain_messages(rx: &mut UnboundedReceiver<Value>) -> Vec<Value> {
    let mut messages = Vec::new();
    while let Ok(message) = rx.try_recv() {
        messages.push(message);
    }
    messages
}

#[tokio::test]
async fn test_connect_timeout_on_unresponsive_server() {
    // Spin up a local TCP listener that accepts the connection but never
    // completes the WebSocket handshake, forcing the client-side timeout
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind test listener");
    let addr = listener.local_addr().expect("failed to get local addr");

    // Accept a single connection and keep it open without responding
    let _server = tokio::spawn(async move {
        if let Ok((stream, _peer)) = listener.accept().await {
            let _ = stream;
            futures_util::future::pending::<()>().await;
        }
    });

    let url = format!("ws://{addr}/unresponsive");
    let start = std::time::Instant::now();
    let result = CdpSession::connect(&url, guard(), None).await;
    let elapsed = start.elapsed();

    assert!(result.is_err(), "connect should fail due to timeout");
    let err = result.err().unwrap();
    // Should timeout within ~1s (test timeout), not hang indefinitely
    assert!(
        elapsed < Duration::from_secs(5),
        "connect should timeout, took {elapsed:?}"
    );
    // Ensure we exercised the timeout path specifically
    assert!(
        err.contains("timed out"),
        "unexpected error (expected timeout): {err}"
    );
}

#[test]
fn test_key_to_code() {
    assert_eq!(key_to_code("Enter"), "Enter");
    assert_eq!(key_to_code("Tab"), "Tab");
    assert_eq!(key_to_code("Escape"), "Escape");
    assert_eq!(key_to_code("Space"), "Space");
    assert_eq!(key_to_code(" "), "Space");
    assert_eq!(key_to_code("ArrowUp"), "ArrowUp");
    assert_eq!(key_to_code("SomeOtherKey"), "SomeOtherKey");
}

/// Reproduces the original EVE-188 bug: Browserless returns 400 on the root
/// WebSocket path. A mock server that only accepts `/chromium` verifies the
/// fix. The root path returns a raw "HTTP/1.1 400 Bad Request" so the
/// tungstenite client surfaces an HTTP 400 error.
#[tokio::test]
async fn test_connect_rejected_on_root_path_accepted_on_chromium() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");

    // Server: accept one connection, read the HTTP upgrade, reject with 400
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.expect("read");
        let request = String::from_utf8_lossy(&buf[..n]);

        if request.contains("GET / ") || !request.contains("GET /chromium") {
            // Reject root path — reproduces the original 400 bug
            stream
                .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                .await
                .expect("write 400");
        }
        stream.shutdown().await.ok();
    });

    // Connect to root path — should get 400
    let result = CdpSession::connect(&format!("ws://{addr}/"), guard(), None).await;
    assert!(result.is_err(), "root path should be rejected");
    let err = result.err().expect("should be Err");
    assert!(err.contains("400"), "error should mention 400: {err}");

    server.await.ok();
}

#[test]
fn test_map_ws_connect_error_403_returns_rest_mode_hint() {
    use tokio_tungstenite::tungstenite::http;

    let resp = http::Response::builder().status(403).body(None).unwrap();
    let err = tokio_tungstenite::tungstenite::Error::Http(Box::new(resp));
    let msg = map_ws_connect_error(err);
    assert!(
        msg.contains("REST-mode tools"),
        "403 error should suggest REST-mode tools: {msg}"
    );
    assert!(msg.contains("403"), "should mention status code: {msg}");
}

#[test]
fn test_map_ws_connect_error_401_returns_rest_mode_hint() {
    use tokio_tungstenite::tungstenite::http;

    let resp = http::Response::builder().status(401).body(None).unwrap();
    let err = tokio_tungstenite::tungstenite::Error::Http(Box::new(resp));
    let msg = map_ws_connect_error(err);
    assert!(
        msg.contains("REST-mode tools"),
        "401 error should suggest REST-mode tools: {msg}"
    );
}

#[test]
fn test_map_ws_connect_error_other_preserves_message() {
    let err =
        tokio_tungstenite::tungstenite::Error::Io(std::io::Error::other("connection refused"));
    let msg = map_ws_connect_error(err);
    assert!(
        msg.starts_with("CDP WebSocket connection failed:"),
        "non-HTTP error should use generic format: {msg}"
    );
}

/// CdpSession::connect against a server returning 403 should produce the
/// REST-mode hint, not a generic error.
#[tokio::test]
async fn test_connect_403_returns_rest_mode_hint() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await.expect("read");
        stream
            .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
            .await
            .expect("write 403");
        stream.shutdown().await.ok();
    });

    let result =
        CdpSession::connect(&format!("ws://{addr}/chromium?token=test"), guard(), None).await;
    assert!(result.is_err());
    let err = result.err().unwrap();
    assert!(
        err.contains("REST-mode tools"),
        "403 should suggest REST-mode: {err}"
    );

    server.await.ok();
}

#[tokio::test]
async fn test_new_browser_gets_a_guarded_context_and_armed_fetch() {
    // An unguarded page in the default context must never be attached.
    let (addr, mut messages, server) = spawn_mock_cdp_server(vec![json!({
        "targetId": "default-page-target",
        "type": "page",
        "url": "https://example.com"
    })])
    .await;

    let mut session =
        CdpSession::connect(&format!("ws://{addr}/chromium?token=test"), guard(), None)
            .await
            .expect("connect should succeed");
    assert_eq!(session.browser_context_id(), "guarded-context");

    session
        .navigate("https://example.com")
        .await
        .expect("navigate should succeed after attaching to the guarded page");
    let reconnect_url = session
        .reconnect(5000)
        .await
        .expect("reconnect should succeed");
    assert_eq!(reconnect_url, "ws://browserless/reconnect");
    session.disconnect().await;

    server.await.expect("server should exit cleanly");
    let messages = drain_messages(&mut messages);
    assert_eq!(
        methods(&messages),
        vec![
            "Target.createBrowserContext",
            "Target.createTarget",
            "Target.attachToTarget",
            "Fetch.enable",
            "Page.enable",
            "Page.navigate",
            "Page.setLifecycleEventsEnabled",
            "Runtime.evaluate",
            "Browserless.reconnect"
        ]
    );
    assert_eq!(messages[1]["params"]["browserContextId"], "guarded-context");
    assert_eq!(messages[2]["params"]["targetId"], "created-page-target");
    assert_eq!(
        messages[3]["params"]["patterns"],
        json!([{ "urlPattern": "*", "requestStage": "Request" }])
    );
    for message in &messages[3..8] {
        assert_eq!(message["sessionId"], "created-page-session");
    }
    assert!(
        messages[8].get("sessionId").is_none(),
        "Browserless.reconnect must stay on the browser root session"
    );
}

#[tokio::test]
async fn test_reconnect_reattaches_the_page_in_the_saved_guarded_context() {
    let (addr, mut messages, server) = spawn_mock_cdp_server_with(
        vec![
            json!({ "targetId": "default-page-target", "type": "page", "url": "https://a.test" }),
            json!({
                "targetId": "existing-page-target",
                "type": "page",
                "url": "https://b.test",
                "browserContextId": "saved-context"
            }),
        ],
        vec!["saved-context"],
    )
    .await;

    let session = CdpSession::connect(
        &format!("ws://{addr}/chromium?token=test"),
        guard(),
        Some("saved-context"),
    )
    .await
    .expect("connect should succeed");
    assert_eq!(session.browser_context_id(), "saved-context");
    assert_eq!(session.page_session_id(), "existing-page-session");
    session.disconnect().await;

    server.await.expect("server should exit cleanly");
    let messages = drain_messages(&mut messages);
    assert_eq!(
        methods(&messages),
        vec!["Target.getTargets", "Target.attachToTarget", "Fetch.enable"]
    );
    assert_eq!(messages[1]["params"]["targetId"], "existing-page-target");
}

#[tokio::test]
async fn test_reconnect_without_a_guarded_context_creates_one() {
    // A session saved before EVE-1189 has no context; its old page lives
    // in the unguarded default context and must not be reattached.
    let (addr, mut messages, server) = spawn_mock_cdp_server_with(
        vec![json!({ "targetId": "default-page-target", "type": "page", "url": "https://a.test" })],
        vec![],
    )
    .await;

    let session = CdpSession::connect(
        &format!("ws://{addr}/chromium?token=test"),
        guard(),
        Some("gone-context"),
    )
    .await
    .expect("connect should succeed");
    assert_eq!(session.browser_context_id(), "guarded-context");
    session.disconnect().await;

    server.await.expect("server should exit cleanly");
    let messages = drain_messages(&mut messages);
    assert_eq!(
        methods(&messages),
        vec![
            "Target.getTargets",
            "Target.getBrowserContexts",
            "Target.createBrowserContext",
            "Target.createTarget",
            "Target.attachToTarget",
            "Fetch.enable"
        ]
    );
}

/// EVE-1189 repro: a public URL whose response redirects to the metadata
/// service. The browser pauses the redirect hop; the session must fail it
/// while `Page.navigate` is still outstanding, and report the failed
/// navigation instead of handing the caller Chrome's error page.
#[tokio::test]
async fn test_redirect_hop_to_metadata_is_failed_while_navigate_waits() {
    let (addr, mut messages, server) = spawn_mock_cdp_server(vec![]).await;

    let mut session =
        CdpSession::connect(&format!("ws://{addr}/chromium?token=test"), guard(), None)
            .await
            .expect("connect should succeed");
    let error = session
        .navigate("https://public.example/redirect-test")
        .await
        .expect_err("a blocked redirect hop must fail the navigation");
    assert!(error.contains("ERR_BLOCKED_BY_CLIENT"), "{error}");
    session.disconnect().await;

    server.await.expect("server should exit cleanly");
    let messages = drain_messages(&mut messages);
    let answer = messages
        .iter()
        .find(|message| {
            message["method"]
                .as_str()
                .is_some_and(|method| method.starts_with("Fetch.") && method != "Fetch.enable")
        })
        .expect("the paused hop must be answered");
    assert_eq!(answer["method"], "Fetch.failRequest");
    assert_eq!(answer["params"]["requestId"], "interception-job-2.0");
    assert_eq!(answer["params"]["errorReason"], "BlockedByClient");
    assert_eq!(answer["sessionId"], "created-page-session");
}

#[tokio::test]
async fn test_close_only_connection_attaches_no_page() {
    let (addr, mut messages, server) = spawn_mock_cdp_server(vec![]).await;
    let session = CdpSession::connect_to_close(&format!("ws://{addr}/chromium?token=test"))
        .await
        .expect("connect should succeed");
    session.disconnect().await;
    server.await.expect("server should exit cleanly");
    assert!(drain_messages(&mut messages).is_empty());
}

/// Verify the CDP URL construction uses the /chromium path.
#[test]
fn test_cdp_url_uses_chromium_path() {
    for (base, token, expected) in [
        (
            "wss://production-sfo.browserless.io",
            "test_token_123",
            "wss://production-sfo.browserless.io/chromium?token=test_token_123",
        ),
        (
            "ws://127.0.0.1:3000",
            "local_token",
            "ws://127.0.0.1:3000/chromium?token=local_token",
        ),
    ] {
        assert_eq!(
            crate::browserless::browser_session_url(base, token),
            expected
        );
    }
}

#[test]
fn test_element_center_expression_scrolls_offscreen_elements_into_view() {
    let expr = element_center_expression("a[href=\"x\"]");
    assert!(expr.contains(r#"document.querySelector("a[href=\"x\"]")"#));
    let scroll = expr.find("scrollIntoView").expect("must scroll into view");
    let viewport_check = expr.find("window.innerHeight").expect("viewport check");
    assert!(
        viewport_check < scroll,
        "scroll only when the center is outside the viewport"
    );
    let last_measure = expr.rfind("c = center()").expect("re-measure");
    assert!(
        scroll < last_measure,
        "coordinates must be measured after scrolling"
    );
}

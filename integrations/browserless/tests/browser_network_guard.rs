//! EVE-1189: tool-level redirect/DNS-rebinding guard for the paths that used
//! to call the Browserless REST endpoints.
//!
//! Without a persistent browser, every tool now drives a one-shot guarded CDP
//! browser. A mock Browserless answers each navigation with a redirect hop to
//! the metadata service and records how the client answered it. The tool must
//! fail that hop and return an error, never page content or a screenshot.

use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::runtime::{
    connection_services::UserConnectionResolver, tool_context::ToolContext,
};
use everruns_contracts::typed_id::SessionId;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

const INTERNAL: &str = "INTERNAL-METADATA-RESPONSE";

struct TokenResolver;

#[async_trait]
impl UserConnectionResolver for TokenResolver {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<String>> {
        Ok(Some("test_api_token".to_string()))
    }
}

fn tool(name: &str) -> Box<dyn Tool> {
    use everruns_contracts::runtime::capabilities::Capability;
    everruns_integrations_browserless::BrowserlessCapability
        .tools()
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("Tool {name} not found"))
}

fn context() -> ToolContext {
    ToolContext::new(SessionId::new()).with_connection_resolver(Arc::new(TokenResolver))
}

/// Mock Browserless CDP endpoint. Every `Page.navigate` pauses a redirect hop
/// to the metadata service; navigation completes with the error Chrome would
/// report if the client failed it. Any page read returns [`INTERNAL`], which a
/// tool must never surface after a rejected navigation.
async fn spawn_redirecting_browserless(answers: UnboundedSender<Value>) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let answers = answers.clone();
            tokio::spawn(async move {
                let Ok(mut ws) = accept_async(stream).await else {
                    return;
                };
                let mut pending_navigate = None;
                while let Some(Ok(message)) = ws.next().await {
                    let Message::Text(text) = message else {
                        continue;
                    };
                    let parsed: Value = serde_json::from_str(&text).expect("json");
                    let id = parsed["id"].as_u64().expect("id");
                    let method = parsed["method"].as_str().expect("method").to_string();
                    let session = parsed.get("sessionId").cloned();
                    let reply = match method.as_str() {
                        "Target.createBrowserContext" => {
                            assert_eq!(parsed["params"]["proxyServer"], "http://127.0.0.1:9");
                            json!({ "id": id, "result": { "browserContextId": "ctx" } })
                        }
                        "Target.createTarget" => {
                            json!({ "id": id, "result": { "targetId": "page" } })
                        }
                        "Target.attachToTarget" => {
                            json!({ "id": id, "result": { "sessionId": "page-session" } })
                        }
                        "Fetch.failRequest" | "Fetch.fulfillRequest" | "Fetch.continueRequest" => {
                            answers.send(parsed.clone()).ok();
                            let ack = json!({ "id": id, "result": {} });
                            ws.send(Message::Text(ack.to_string().into())).await.ok();
                            let Some(navigate_id) = pending_navigate.take() else {
                                continue;
                            };
                            let result = if method == "Fetch.failRequest" {
                                json!({ "frameId": "f", "errorText": "net::ERR_BLOCKED_BY_CLIENT" })
                            } else {
                                json!({ "frameId": "f" })
                            };
                            json!({ "id": navigate_id, "result": result })
                        }
                        "Page.navigate" => {
                            pending_navigate = Some(id);
                            json!({
                                "method": "Fetch.requestPaused",
                                "sessionId": session,
                                "params": {
                                    "requestId": format!("hop-{id}"),
                                    "resourceType": "Document",
                                    "request": {
                                        "url": "http://169.254.169.254/latest/meta-data/iam/",
                                        "method": "GET",
                                        "headers": {}
                                    }
                                }
                            })
                        }
                        "Runtime.evaluate" => json!({
                            "id": id,
                            "result": { "result": { "type": "string", "value": INTERNAL } }
                        }),
                        "Page.captureScreenshot" => {
                            json!({ "id": id, "result": { "data": "SU5URVJOQUw=" } })
                        }
                        _ => json!({ "id": id, "result": {} }),
                    };
                    if ws
                        .send(Message::Text(reply.to_string().into()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            });
        }
    });
    addr
}

/// A server that refuses every WebSocket upgrade with 403, like a Browserless
/// token without CDP access.
async fn spawn_cdp_refusing_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await;
            let _ = stream
                .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                .await;
            let _ = stream.shutdown().await;
        }
    });
    addr
}

fn assert_rejected_without_content(name: &str, result: &ToolExecutionResult) {
    let rendered = format!("{result:?}");
    assert!(
        matches!(result, ToolExecutionResult::ToolError(_)),
        "{name} must fail a rejected navigation, got {rendered}"
    );
    assert!(
        rendered.contains("ERR_BLOCKED_BY_CLIENT"),
        "{name}: {rendered}"
    );
    assert!(!rendered.contains(INTERNAL), "{name} leaked: {rendered}");
    assert!(
        !rendered.contains("SU5URVJOQUw="),
        "{name} leaked a screenshot"
    );
}

fn drain(answers: &mut UnboundedReceiver<Value>) -> Vec<Value> {
    let mut out = Vec::new();
    while let Ok(answer) = answers.try_recv() {
        out.push(answer);
    }
    out
}

// Both scenarios mutate process-wide BROWSERLESS_* env vars, so they run in
// one test, in sequence.
#[tokio::test]
async fn one_shot_tools_fail_redirect_hops_and_self_hosted_never_falls_back_to_rest() {
    let (tx, mut answers) = unbounded_channel();
    let cdp = spawn_redirecting_browserless(tx).await;
    // SAFETY: this is the only test in this binary; nothing else reads these.
    unsafe { std::env::set_var("BROWSERLESS_WS_BASE", format!("ws://{cdp}")) };

    let ctx = context();
    let calls = [
        (
            "browserless_content",
            json!({ "url": "https://public.example/" }),
        ),
        (
            "browserless_screenshot",
            json!({ "url": "https://public.example/" }),
        ),
        (
            "browserless_navigate",
            json!({ "url": "https://public.example/" }),
        ),
        (
            "browserless_scrape",
            json!({ "url": "https://public.example/", "elements": [{ "selector": "body" }] }),
        ),
        (
            "browserless_interact",
            json!({
                "url": "https://public.example/",
                "steps": [{ "action": "wait", "wait_ms": 1 }],
                "return_content": true,
                "return_screenshot": true
            }),
        ),
    ];
    for (name, arguments) in calls {
        let result = tool(name).execute_with_context(arguments, &ctx).await;
        assert_rejected_without_content(name, &result);
        let answered = drain(&mut answers);
        assert_eq!(answered.len(), 1, "{name}: {answered:?}");
        assert_eq!(answered[0]["method"], "Fetch.failRequest", "{name}");
        assert_eq!(answered[0]["params"]["errorReason"], "BlockedByClient");
        assert_eq!(answered[0]["sessionId"], "page-session");
    }

    // A self-hosted Browserless that refuses CDP must not fall back to its
    // REST endpoints, whose browsers would fetch without the guard.
    let refusing = spawn_cdp_refusing_server().await;
    let rest = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_string(INTERNAL))
        .expect(0)
        .mount(&rest)
        .await;
    unsafe {
        std::env::set_var("BROWSERLESS_WS_BASE", format!("ws://{refusing}"));
        std::env::set_var("BROWSERLESS_API_BASE", rest.uri());
    }
    let result = tool("browserless_content")
        .execute_with_context(json!({ "url": "https://public.example/" }), &ctx)
        .await;
    let rendered = format!("{result:?}");
    assert!(
        matches!(result, ToolExecutionResult::ToolError(_)),
        "{rendered}"
    );
    assert!(
        rendered.contains("guarded Browserless browser"),
        "{rendered}"
    );
    assert!(!rendered.contains(INTERNAL), "{rendered}");

    unsafe {
        std::env::remove_var("BROWSERLESS_WS_BASE");
        std::env::remove_var("BROWSERLESS_API_BASE");
    }
}

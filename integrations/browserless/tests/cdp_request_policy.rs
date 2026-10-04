//! Fetch-domain request policy: a denied public host is failed, an allowed host continues.

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc::unbounded_channel;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

use everruns_integrations_browserless::cdp::CdpSession;

#[tokio::test]
async fn request_policy_fails_a_denied_host_and_continues_an_allowed_one() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, mut rx) = unbounded_channel();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut ws = accept_async(stream).await.expect("websocket handshake");

        while let Some(message) = ws.next().await {
            let text = match message.expect("message") {
                Message::Text(text) => text,
                Message::Close(_) => break,
                _ => continue,
            };
            let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
            tx.send(parsed.clone()).ok();
            let id = parsed["id"].as_u64().expect("command id");
            let method = parsed["method"].as_str().expect("method");
            let response = match method {
                "Target.getTargets" => json!({
                    "id": id,
                    "result": {
                        "targetInfos": [{
                            "targetId": "existing-page-target",
                            "type": "page",
                            "url": "about:blank"
                        }]
                    }
                }),
                "Target.attachToTarget" => {
                    json!({ "id": id, "result": { "sessionId": "existing-page-session" } })
                }
                "Fetch.enable" => json!({ "id": id, "result": {} }),
                "Page.enable" | "Page.setLifecycleEventsEnabled" | "Runtime.evaluate" => {
                    json!({ "id": id, "result": { "result": { "type": "undefined" } } })
                }
                "Page.navigate" => {
                    for (request_id, url) in [
                        ("allowed-request", "https://example.com/"),
                        ("denied-request", "https://evil.test/secret"),
                    ] {
                        ws.send(Message::Text(
                            json!({
                                "method": "Fetch.requestPaused",
                                "sessionId": "existing-page-session",
                                "params": {
                                    "requestId": request_id,
                                    "request": { "url": url }
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .expect("send paused event");
                    }
                    // The client answers both pauses before it will accept this result.
                    let mut answered = 0;
                    while answered < 2 {
                        let answer = ws.next().await.expect("policy answer").expect("frame");
                        let Message::Text(text) = answer else {
                            continue;
                        };
                        let answer: Value = serde_json::from_str(&text).expect("json");
                        tx.send(answer.clone()).ok();
                        let answer_id = answer["id"].as_u64().expect("id");
                        ws.send(Message::Text(
                            json!({ "id": answer_id, "result": {} }).to_string().into(),
                        ))
                        .await
                        .expect("ack policy answer");
                        answered += 1;
                    }
                    json!({ "id": id, "result": { "frameId": "frame-1" } })
                }
                other => panic!("unexpected method: {other}"),
            };
            ws.send(Message::Text(response.to_string().into()))
                .await
                .expect("send response");
        }
    });

    let mut session = CdpSession::connect(&format!("ws://{addr}/chromium?token=test"))
        .await
        .expect("connect");
    session
        .arm_request_policy(
            &everruns_core::network_access::NetworkAccessList::allow_only(["example.com"]),
        )
        .await
        .expect("arm");
    session
        .navigate("https://example.com/")
        .await
        .expect("navigate");
    session.disconnect().await;
    server.abort();

    let mut decisions = Vec::new();
    while let Ok(message) = rx.try_recv() {
        let method = message["method"].as_str().unwrap_or("");
        if method == "Fetch.continueRequest" || method == "Fetch.failRequest" {
            decisions.push((
                method.to_string(),
                message["params"]["requestId"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            ));
        }
    }
    assert_eq!(
        decisions,
        vec![
            (
                "Fetch.continueRequest".to_string(),
                "allowed-request".to_string()
            ),
            (
                "Fetch.failRequest".to_string(),
                "denied-request".to_string()
            ),
        ]
    );
}

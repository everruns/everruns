//! Responses WebSocket transport against a local mock server that speaks both
//! WebSocket mode and SSE on one port, like `api.openai.com/v1/responses`.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message as Frame;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

use crate::driver_registry::{ChatDriver, LlmCallConfig, LlmStreamEvent, Message, MessageRole};
use crate::runtime_provider::{BearerAuth, RuntimeProvider};

use super::*;

/// What the mock does with one `response.create` (argument: the 0-based index
/// of the request across all sockets).
enum Reply {
    /// Send these server events, then wait for the next request.
    Events(Vec<Value>),
    /// Close the socket without answering.
    Close,
    /// Send these events, then drop the TCP connection mid-response.
    EventsThenDrop(Vec<Value>),
    /// Send these events, then close the socket cleanly.
    EventsThenClose(Vec<Value>),
}

type Script = Arc<dyn Fn(usize) -> Reply + Send + Sync>;

#[derive(Default)]
struct Seen {
    /// Every `response.create`, tagged with the socket that carried it.
    creates: Mutex<Vec<(usize, Value)>>,
    /// `Authorization` header of each WebSocket handshake.
    handshake_auth: Mutex<Vec<String>>,
    sockets: AtomicUsize,
    /// Close frames received from the client.
    client_closes: AtomicUsize,
    http_posts: Mutex<Vec<Value>>,
}

struct MockResponses {
    base_url: String,
    seen: Arc<Seen>,
}

impl MockResponses {
    async fn start(script: Script, sse_body: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let seen = Arc::new(Seen::default());
        let server_seen = Arc::clone(&seen);
        tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let seen = Arc::clone(&server_seen);
                let script = Arc::clone(&script);
                let sse_body = sse_body.clone();
                tokio::spawn(async move {
                    if is_upgrade(&tcp).await {
                        serve_websocket(tcp, seen, script).await;
                    } else {
                        serve_sse(tcp, seen, sse_body).await;
                    }
                });
            }
        });
        Self { base_url, seen }
    }

    fn creates(&self) -> Vec<(usize, Value)> {
        self.seen.creates.lock().unwrap().clone()
    }

    fn sockets(&self) -> usize {
        self.seen.sockets.load(Ordering::SeqCst)
    }

    fn http_posts(&self) -> Vec<Value> {
        self.seen.http_posts.lock().unwrap().clone()
    }
}

async fn is_upgrade(tcp: &TcpStream) -> bool {
    let mut buf = vec![0u8; 8192];
    loop {
        let n = tcp.peek(&mut buf).await.unwrap_or(0);
        let head = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
        if head.contains("\r\n\r\n") || n == 0 || n == buf.len() {
            return head.contains("upgrade: websocket");
        }
        tokio::task::yield_now().await;
    }
}

// The handshake callback's error type is tungstenite's, not ours to shrink.
#[allow(clippy::result_large_err)]
async fn serve_websocket(tcp: TcpStream, seen: Arc<Seen>, script: Script) {
    let auth_seen = Arc::clone(&seen);
    let socket = tokio_tungstenite::accept_hdr_async(tcp, move |req: &Request, res: Response| {
        assert_eq!(req.uri().path(), "/v1/responses");
        let auth = req
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        auth_seen.handshake_auth.lock().unwrap().push(auth);
        Ok(res)
    })
    .await;
    let Ok(mut socket) = socket else {
        return;
    };
    let socket_index = seen.sockets.fetch_add(1, Ordering::SeqCst);
    while let Some(Ok(frame)) = socket.next().await {
        let text = match frame {
            Frame::Text(text) => text,
            Frame::Close(_) => {
                seen.client_closes.fetch_add(1, Ordering::SeqCst);
                continue;
            }
            _ => continue,
        };
        let create: Value = serde_json::from_str(&text).unwrap();
        let request_index = {
            let mut creates = seen.creates.lock().unwrap();
            creates.push((socket_index, create));
            creates.len() - 1
        };
        match script(request_index) {
            Reply::Events(events) => {
                for event in events {
                    socket.send(Frame::text(event.to_string())).await.unwrap();
                }
            }
            Reply::Close => {
                let _ = socket.close(None).await;
                return;
            }
            Reply::EventsThenDrop(events) => {
                for event in events {
                    socket.send(Frame::text(event.to_string())).await.unwrap();
                }
                return;
            }
            Reply::EventsThenClose(events) => {
                for event in events {
                    socket.send(Frame::text(event.to_string())).await.unwrap();
                }
                let _ = socket.close(None).await;
                return;
            }
        }
    }
}

async fn serve_sse(mut tcp: TcpStream, seen: Arc<Seen>, sse_body: String) {
    let mut request = Vec::new();
    let mut buf = [0u8; 4096];
    let body_start = loop {
        let n = tcp.read(&mut buf).await.unwrap();
        if n == 0 {
            return;
        }
        request.extend_from_slice(&buf[..n]);
        if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&request[..body_start]).to_ascii_lowercase();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    while request.len() < body_start + length {
        let n = tcp.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        request.extend_from_slice(&buf[..n]);
    }
    let body: Value = serde_json::from_slice(&request[body_start..]).unwrap_or(Value::Null);
    seen.http_posts.lock().unwrap().push(body);
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        sse_body.len(),
        sse_body
    );
    let _ = tcp.write_all(response.as_bytes()).await;
    let _ = tcp.shutdown().await;
}

fn created(id: &str) -> Value {
    json!({"type": "response.created", "sequence_number": 0,
        "response": {"id": id, "status": "in_progress", "output": []}})
}

fn completed(id: &str) -> Value {
    json!({"type": "response.completed", "sequence_number": 9,
        "response": {"id": id, "status": "completed", "service_tier": "ultrafast", "output": [],
            "usage": {"input_tokens": 10, "output_tokens": 2,
                "input_tokens_details": {"cached_tokens": 4}}}})
}

fn text_response(id: &str, text: &str) -> Vec<Value> {
    vec![
        created(id),
        json!({"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0,
            "content_index": 0, "delta": text, "sequence_number": 1}),
        completed(id),
    ]
}

fn tool_call_response(id: &str) -> Vec<Value> {
    vec![
        created(id),
        json!({"type": "response.output_item.added", "output_index": 0, "sequence_number": 1,
            "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1",
                "name": "get_weather", "arguments": ""}}),
        json!({"type": "response.function_call_arguments.delta", "item_id": "fc_1",
            "output_index": 0, "delta": "{\"city\":\"Paris\"}", "sequence_number": 2}),
        json!({"type": "response.output_item.done", "output_index": 0, "sequence_number": 3,
            "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1",
                "name": "get_weather", "arguments": "{\"city\":\"Paris\"}", "status": "completed"}}),
        completed(id),
    ]
}

fn sse_text(text: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\n",
        json!({"type": "response.output_text.delta", "delta": text}),
        json!({"type": "response.completed",
            "response": {"id": "resp_sse", "status": "completed", "output": []}})
    )
}

fn provider(base_url: &str) -> RuntimeProvider {
    RuntimeProvider::new("ws-test", OpenResponsesProtocolChatDriver::new())
        .base_url(base_url)
        .auth(BearerAuth::new("test-key"))
}

fn driver() -> OpenResponsesProtocolChatDriver {
    OpenResponsesProtocolChatDriver::new()
        .with_stateful_responses(true)
        .with_websocket_support(true)
        .with_retry_config(LlmRetryConfig::no_retry())
}

fn ws_config(previous_response_id: Option<&str>) -> LlmCallConfig {
    let mut config = LlmCallConfig::new("gpt-6-astra");
    config.speed = Some("ultrafast".into());
    config.previous_response_id = previous_response_id.map(str::to_string);
    config
        .driver_options
        .insert(OPENAI_WEBSOCKET_OPTION.into(), json!(true));
    config
}

async fn collect(
    driver: &OpenResponsesProtocolChatDriver,
    provider: &RuntimeProvider,
    messages: Vec<Message>,
    config: &LlmCallConfig,
) -> Vec<LlmStreamEvent> {
    let stream = driver
        .chat_completion_stream(provider.endpoint(), messages, config)
        .await
        .expect("stream should start");
    stream
        .map(|event| event.expect("no stream error"))
        .filter(|event| {
            futures::future::ready(!matches!(event, LlmStreamEvent::TextDelta(t) if t.is_empty()))
        })
        .collect()
        .await
}

fn texts(events: &[LlmStreamEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            LlmStreamEvent::TextDelta(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn done(events: &[LlmStreamEvent]) -> &crate::driver_registry::LlmCompletionMetadata {
    events
        .iter()
        .find_map(|event| match event {
            LlmStreamEvent::Done(metadata) => Some(metadata.as_ref()),
            _ => None,
        })
        .expect("a Done event")
}

#[tokio::test]
async fn websocket_events_map_like_sse_and_the_create_event_is_the_http_body() {
    let script: Script = Arc::new(|_| Reply::Events(tool_call_response("resp_1")));
    let server = MockResponses::start(script, sse_text("unused")).await;
    let provider = provider(&server.base_url);

    let events = collect(
        &driver(),
        &provider,
        vec![Message::text(MessageRole::User, "weather in Paris?")],
        &ws_config(None),
    )
    .await;

    let calls: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            LlmStreamEvent::ToolCalls(calls) => Some(calls.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(calls.len(), 1, "{events:?}");
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].name, "get_weather");
    assert_eq!(calls[0].arguments, json!({"city": "Paris"}));
    let metadata = done(&events);
    assert_eq!(metadata.response_id.as_deref(), Some("resp_1"));
    assert_eq!(metadata.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(metadata.prompt_tokens, Some(6));
    assert_eq!(metadata.cache_read_tokens, Some(4));
    assert_eq!(metadata.service_tier.as_deref(), Some("ultrafast"));

    let creates = server.creates();
    assert_eq!(creates.len(), 1);
    let create = &creates[0].1;
    assert_eq!(create["type"], "response.create");
    assert_eq!(create["model"], "gpt-6-astra");
    assert_eq!(create["service_tier"], "ultrafast");
    assert!(create.get("stream").is_none(), "{create}");
    assert!(create.get("background").is_none(), "{create}");
    assert_eq!(
        server.seen.handshake_auth.lock().unwrap().as_slice(),
        ["Bearer test-key"]
    );
    assert!(server.http_posts().is_empty(), "no SSE request expected");
}

#[tokio::test]
async fn a_tool_loop_continues_on_the_same_socket() {
    let script: Script = Arc::new(|index| {
        Reply::Events(match index {
            0 => tool_call_response("resp_1"),
            _ => text_response("resp_2", "It is sunny."),
        })
    });
    let server = MockResponses::start(script, sse_text("unused")).await;
    let provider = provider(&server.base_url);
    let driver = driver();

    let first = collect(
        &driver,
        &provider,
        vec![Message::text(MessageRole::User, "weather in Paris?")],
        &ws_config(None),
    )
    .await;
    let first_id = done(&first).response_id.clone().unwrap();

    let tool_call = crate::tool_types::ToolCall {
        id: "call_1".into(),
        name: "get_weather".into(),
        arguments: json!({"city": "Paris"}),
    };
    let mut assistant = Message::text(MessageRole::Assistant, "");
    assistant.tool_calls = Some(vec![tool_call]);
    let mut tool = Message::text(MessageRole::Tool, "sunny");
    tool.tool_call_id = Some("call_1".into());
    let second = collect(
        &driver,
        &provider,
        vec![
            Message::text(MessageRole::User, "weather in Paris?"),
            assistant,
            tool,
        ],
        &ws_config(Some(&first_id)),
    )
    .await;

    assert_eq!(texts(&second), ["It is sunny."]);
    assert_eq!(done(&second).response_id.as_deref(), Some("resp_2"));
    assert_eq!(server.sockets(), 1, "the second turn reuses the socket");
    let creates = server.creates();
    assert_eq!(creates.len(), 2);
    assert_eq!(creates[0].0, creates[1].0, "same socket");
    let continuation = &creates[1].1;
    assert_eq!(continuation["previous_response_id"], "resp_1");
    // Only the new items travel: the tool output, not the replayed transcript.
    let input = continuation["input"].as_array().unwrap();
    assert_eq!(input.len(), 1, "{continuation}");
    assert_eq!(input[0]["type"], "function_call_output");
    assert_eq!(input[0]["call_id"], "call_1");

    // A call that does not continue from the parked response opens its own.
    let third = collect(
        &driver,
        &provider,
        vec![Message::text(MessageRole::User, "new conversation")],
        &ws_config(None),
    )
    .await;
    assert_eq!(texts(&third), ["It is sunny."]);
    assert_eq!(server.sockets(), 2);
}

#[tokio::test]
async fn a_failed_handshake_falls_back_to_sse() {
    // A plain HTTP listener: the upgrade request gets no 101.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let posts = Arc::new(AtomicUsize::new(0));
    let upgrades = Arc::new(AtomicUsize::new(0));
    let (server_posts, server_upgrades) = (Arc::clone(&posts), Arc::clone(&upgrades));
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            if is_upgrade(&tcp).await {
                server_upgrades.fetch_add(1, Ordering::SeqCst);
                let mut tcp = tcp;
                let _ = tcp
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                    )
                    .await;
            } else {
                server_posts.fetch_add(1, Ordering::SeqCst);
                serve_sse(tcp, Arc::new(Seen::default()), sse_text("over SSE")).await;
            }
        }
    });

    let events = collect(
        &driver(),
        &provider(&base_url),
        vec![Message::text(MessageRole::User, "hi")],
        &ws_config(None),
    )
    .await;

    assert_eq!(texts(&events), ["over SSE"]);
    assert_eq!(upgrades.load(Ordering::SeqCst), 1);
    assert_eq!(posts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_socket_that_drops_or_errors_before_the_first_event_falls_back_to_sse() {
    for reply in ["close", "error"] {
        let script: Script = Arc::new(move |_| match reply {
            "close" => Reply::Close,
            _ => Reply::Events(vec![json!({"type": "error", "status": 429,
                "error": {"type": "rate_limit_error", "code": "rate_limit_exceeded",
                    "message": "slow down"}})]),
        });
        let server = MockResponses::start(script, sse_text("over SSE")).await;

        let events = collect(
            &driver(),
            &provider(&server.base_url),
            vec![Message::text(MessageRole::User, "hi")],
            &ws_config(None),
        )
        .await;

        assert_eq!(texts(&events), ["over SSE"], "{reply}");
        assert_eq!(server.creates().len(), 1, "{reply}");
        let posts = server.http_posts();
        assert_eq!(posts.len(), 1, "{reply}");
        assert_eq!(posts[0]["stream"], true, "the HTTP body is unchanged");
    }
}

#[tokio::test]
async fn a_drop_after_the_first_event_is_a_stream_error_not_a_resend() {
    let script: Script = Arc::new(|_| {
        Reply::EventsThenDrop(vec![
            created("resp_1"),
            json!({"type": "response.output_text.delta", "item_id": "msg_1",
                "output_index": 0, "content_index": 0, "delta": "partial", "sequence_number": 1}),
        ])
    });
    let server = MockResponses::start(script, sse_text("must not be used")).await;
    let provider = provider(&server.base_url);

    let stream = driver()
        .chat_completion_stream(
            provider.endpoint(),
            vec![Message::text(MessageRole::User, "hi")],
            &ws_config(None),
        )
        .await
        .expect("committed stream");
    let events: Vec<_> = stream.map(|event| event.unwrap()).collect().await;

    let delivered: Vec<_> = texts(&events)
        .into_iter()
        .filter(|text| !text.is_empty())
        .collect();
    assert_eq!(delivered, ["partial"]);
    let error = events
        .iter()
        .find_map(|event| match event {
            LlmStreamEvent::Error(error) => Some(error.to_string()),
            _ => None,
        })
        .expect("a stream error");
    assert!(error.contains("Stream error"), "{error}");
    assert!(server.http_posts().is_empty(), "never re-sent over HTTP");
}

#[tokio::test]
async fn without_the_option_the_call_stays_on_sse() {
    let script: Script = Arc::new(|_| Reply::Events(text_response("resp_ws", "over WS")));
    let server = MockResponses::start(script, sse_text("over SSE")).await;
    let mut config = ws_config(None);
    config.driver_options.clear();

    let events = collect(
        &driver(),
        &provider(&server.base_url),
        vec![Message::text(MessageRole::User, "hi")],
        &config,
    )
    .await;

    assert_eq!(texts(&events), ["over SSE"]);
    assert_eq!(server.sockets(), 0);
}

#[tokio::test]
async fn a_parked_socket_the_server_closed_is_replaced_by_a_fresh_one() {
    let script: Script = Arc::new(|index| match index {
        0 => Reply::EventsThenClose(text_response("resp_1", "first")),
        _ => Reply::Events(text_response("resp_2", "second")),
    });
    let server = MockResponses::start(script, sse_text("must not be used")).await;
    let provider = provider(&server.base_url);
    let driver = driver();

    let first = collect(
        &driver,
        &provider,
        vec![Message::text(MessageRole::User, "one")],
        &ws_config(None),
    )
    .await;
    assert_eq!(texts(&first), ["first"]);
    // Let the keeper observe the server's close.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let second = collect(
        &driver,
        &provider,
        vec![
            Message::text(MessageRole::User, "one"),
            Message::text(MessageRole::Assistant, "first"),
            Message::text(MessageRole::User, "two"),
        ],
        &ws_config(Some("resp_1")),
    )
    .await;

    assert_eq!(texts(&second), ["second"]);
    assert_eq!(server.sockets(), 2);
    assert!(server.http_posts().is_empty());
}

#[tokio::test]
async fn dropping_a_stream_mid_response_closes_the_socket() {
    let script: Script = Arc::new(|_| {
        Reply::Events(vec![
            created("resp_1"),
            json!({"type": "response.output_text.delta", "item_id": "msg_1",
                "output_index": 0, "content_index": 0, "delta": "partial", "sequence_number": 1}),
        ])
    });
    let server = MockResponses::start(script, sse_text("must not be used")).await;
    let provider = provider(&server.base_url);

    let mut stream = driver()
        .chat_completion_stream(
            provider.endpoint(),
            vec![Message::text(MessageRole::User, "hi")],
            &ws_config(None),
        )
        .await
        .expect("committed stream");
    loop {
        match stream.next().await.unwrap().unwrap() {
            LlmStreamEvent::TextDelta(text) if text == "partial" => break,
            _ => continue,
        }
    }
    drop(stream);

    for _ in 0..100 {
        if server.seen.client_closes.load(Ordering::SeqCst) == 1 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the client never sent a close frame");
}

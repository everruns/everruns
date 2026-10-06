//! ConverseStream wire tests: a response cut off at `max_tokens`, or carrying
//! tool input that does not parse, never hands a partial call to execution and
//! never substitutes `{}` for input the model did not finish.

use aws_sdk_bedrockruntime::Client;
use aws_sdk_bedrockruntime::config::{
    BehaviorVersion, Builder as BedrockConfigBuilder, Credentials, Region,
};
use aws_smithy_types::event_stream::{Header, HeaderValue, Message as Frame};
use everruns_contracts::driver_registry::{
    LlmCallConfig, LlmCompletionMetadata, LlmStreamEvent, Message, MessageRole,
};
use everruns_contracts::tool_types::ToolCall;
use futures::StreamExt;
use serde_json::{Value, json};

use super::{BedrockAuth, BedrockChatDriver};

fn event(kind: &'static str, payload: Value) -> Vec<u8> {
    let frame = Frame::new(serde_json::to_vec(&payload).unwrap())
        .add_header(Header::new(
            ":message-type",
            HeaderValue::String("event".into()),
        ))
        .add_header(Header::new(":event-type", HeaderValue::String(kind.into())))
        .add_header(Header::new(
            ":content-type",
            HeaderValue::String("application/json".into()),
        ));
    let mut bytes = Vec::new();
    aws_smithy_eventstream::frame::write_message_to(&frame, &mut bytes).unwrap();
    bytes
}

fn tool_start(index: u32, id: &str, name: &str) -> Vec<u8> {
    event(
        "contentBlockStart",
        json!({"contentBlockIndex": index, "start": {"toolUse": {"toolUseId": id, "name": name}}}),
    )
}

fn tool_input(index: u32, input: &str) -> Vec<u8> {
    event(
        "contentBlockDelta",
        json!({"contentBlockIndex": index, "delta": {"toolUse": {"input": input}}}),
    )
}

fn block_stop(index: u32) -> Vec<u8> {
    event("contentBlockStop", json!({"contentBlockIndex": index}))
}

fn message_stop(reason: &str) -> Vec<u8> {
    event("messageStop", json!({"stopReason": reason}))
}

/// Replay one ConverseStream body through the real driver and collect the
/// last tool-call set it handed on and the completion metadata.
async fn stream(frames: Vec<Vec<u8>>) -> (Vec<ToolCall>, LlmCompletionMetadata) {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path("/model/model/converse-stream"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/vnd.amazon.eventstream")
                .set_body_bytes(frames.concat()),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::from_conf(
        BedrockConfigBuilder::new()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .credentials_provider(Credentials::new("a", "s", None, None, "test"))
            .endpoint_url(server.uri())
            .build(),
    );
    let provider = everruns_contracts::Provider::new("bedrock", BedrockChatDriver::new())
        .auth(BedrockAuth { client });
    let mut events = provider
        .chat_completion_stream(
            vec![Message::text(MessageRole::User, "write the files")],
            &LlmCallConfig::new("model"),
        )
        .await
        .unwrap();
    let (mut calls, mut done) = (Vec::new(), None);
    while let Some(event) = events.next().await {
        match event.unwrap() {
            LlmStreamEvent::ToolCalls(set) => calls = set,
            LlmStreamEvent::Done(metadata) => done = Some(*metadata),
            _ => {}
        }
    }
    (calls, done.expect("stream ends with Done"))
}

/// `max_tokens` lands while the second call is still streaming its input. The
/// first call closed its block and runs; the cut one is dropped and counted,
/// never run (it used to run as `{}`, or fail the whole stream on bad JSON).
#[tokio::test]
async fn max_tokens_drops_the_cut_call_and_keeps_the_closed_one() {
    let (calls, metadata) = stream(vec![
        tool_start(0, "t1", "read_file"),
        tool_input(0, r#"{"path":"a.txt"}"#),
        block_stop(0),
        tool_start(1, "t2", "write_file"),
        tool_input(1, r#"{"path":"b.txt","content":"lor"#),
        message_stop("max_tokens"),
    ])
    .await;
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].id, "t1");
    assert_eq!(calls[0].arguments, json!({"path": "a.txt"}));
    assert_eq!(metadata.finish_reason.as_deref(), Some("length"));
    assert_eq!(
        metadata.provider_finish_reason.as_deref(),
        Some("max_tokens")
    );
    assert_eq!(metadata.tool_calls_dropped, 1);
    assert_eq!(metadata.tool_calls_truncated_executed, 1);
}

/// A call cut off before any input arrived used to run as `{}`.
#[tokio::test]
async fn max_tokens_before_any_input_drops_the_call() {
    let (calls, metadata) = stream(vec![
        tool_start(0, "t1", "write_file"),
        message_stop("max_tokens"),
    ])
    .await;
    assert!(calls.is_empty(), "{calls:?}");
    assert_eq!(metadata.finish_reason.as_deref(), Some("length"));
    assert_eq!(metadata.tool_calls_dropped, 1);
    assert_eq!(metadata.tool_calls_truncated_executed, 0);
}

/// A `tool_use` stop runs its calls; input that does not parse withholds that
/// call (counted) instead of failing the stream, and a call with no input is
/// an ordinary no-argument call.
#[tokio::test]
async fn tool_use_stop_withholds_only_unparseable_input() {
    let (calls, metadata) = stream(vec![
        tool_start(0, "t1", "list"),
        block_stop(0),
        tool_start(1, "t2", "bash"),
        tool_input(1, r#"{"command":"ls"#),
        block_stop(1),
        message_stop("tool_use"),
    ])
    .await;
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].id, "t1");
    assert_eq!(calls[0].arguments, json!({}));
    assert_eq!(metadata.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(metadata.tool_calls_dropped, 1);
    assert_eq!(metadata.tool_calls_truncated_executed, 0);
}

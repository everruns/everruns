//! Wire tests: a Responses stream cut off at the output limit, or carrying a
//! call whose arguments do not parse, never hands a partial call to execution
//! and never substitutes `{}` for its arguments.

use futures::StreamExt;
use serde_json::{Value, json};

use crate::driver_registry::{
    ChatDriver, LlmCompletionMetadata, LlmStreamEvent, Message, MessageRole,
};
use crate::llm_retry::LlmRetryConfig;
use crate::tool_types::ToolCall;

use super::tests_support::*;
use super::*;

fn frame(event: Value) -> String {
    format!("data: {event}\n\n")
}

fn function_call(id: &str, name: &str, arguments: &str, status: &str) -> Value {
    json!({
        "type": "function_call", "id": format!("fc_{id}"), "call_id": format!("call_{id}"),
        "name": name, "arguments": arguments, "status": status
    })
}

fn terminal(status: &str, details: Value, output: Vec<Value>) -> String {
    frame(json!({
        "type": format!("response.{status}"),
        "response": {
            "id": "resp_cut", "object": "response", "created_at": 1780000000,
            "status": status, "incomplete_details": details, "model": "gpt-5.5",
            "output": output,
            "usage": {"input_tokens": 10, "output_tokens": 4096, "total_tokens": 4106}
        }
    }))
}

/// Run one streamed response through the real driver and collect what it
/// handed on: the last tool-call set (the engine overwrites on each event)
/// and the completion metadata.
async fn stream(sse: String) -> (Vec<ToolCall>, LlmCompletionMetadata) {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider = crate::runtime_provider::RuntimeProvider::new(
        "cut",
        OpenResponsesProtocolChatDriver::new(),
    )
    .base_url(server.uri());
    let driver =
        OpenResponsesProtocolChatDriver::new().with_retry_config(LlmRetryConfig::no_retry());
    let mut events = driver
        .chat_completion_stream(
            provider.endpoint(),
            vec![Message::text(MessageRole::User, "write the files")],
            &auth_test_config(),
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

/// The response hits `max_output_tokens` after one call finished and while a
/// second was still streaming its arguments. The finished call is handed on;
/// the cut one is dropped and counted, never run as `{}`.
#[tokio::test]
async fn output_limit_drops_the_cut_call_and_keeps_the_finished_one() {
    let finished = function_call("1", "read_file", r#"{"path":"a.txt"}"#, "completed");
    let cut = function_call(
        "2",
        "write_file",
        r#"{"path":"b.txt","content":"lorem"#,
        "incomplete",
    );
    let sse = [
        frame(
            json!({"type": "response.output_item.added", "output_index": 0,
            "item": function_call("1", "read_file", "", "in_progress")}),
        ),
        frame(json!({"type": "response.output_item.done", "output_index": 0, "item": finished})),
        frame(
            json!({"type": "response.output_item.added", "output_index": 1,
            "item": function_call("2", "write_file", "", "in_progress")}),
        ),
        frame(
            json!({"type": "response.function_call_arguments.delta", "item_id": "fc_2",
            "output_index": 1, "delta": r#"{"path":"b.txt","content":"lorem"#}),
        ),
        terminal(
            "incomplete",
            json!({"reason": "max_output_tokens"}),
            vec![finished.clone(), cut],
        ),
    ]
    .concat();

    let (calls, metadata) = stream(sse).await;
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].name, "read_file");
    assert_eq!(calls[0].arguments, json!({"path": "a.txt"}));
    assert!(calls.iter().all(|call| call.name != "write_file"));
    assert_eq!(metadata.finish_reason.as_deref(), Some("length"));
    assert_eq!(
        metadata.provider_finish_reason.as_deref(),
        Some("max_output_tokens")
    );
    assert_eq!(metadata.tool_calls_dropped, 1);
    assert_eq!(metadata.tool_calls_truncated_executed, 1);
}

/// A cut-off call that only the terminal response describes (no streamed
/// frames) is still counted, and still never handed on.
#[tokio::test]
async fn output_limit_counts_a_cut_call_seen_only_in_the_terminal_response() {
    let cut = function_call("9", "bash", r#"{"command":"rm -rf"#, "incomplete");
    let sse = terminal(
        "incomplete",
        json!({"reason": "max_output_tokens"}),
        vec![cut],
    );
    let (calls, metadata) = stream(sse).await;
    assert!(calls.is_empty(), "{calls:?}");
    assert_eq!(metadata.finish_reason.as_deref(), Some("length"));
    assert_eq!(metadata.tool_calls_dropped, 1);
    assert_eq!(metadata.tool_calls_truncated_executed, 0);
}

/// A finished call whose arguments do not parse used to run as `{}`. It is
/// withheld and counted instead, and the response still reads as a tool turn
/// so the engine can tell the model the call did not run.
#[tokio::test]
async fn unparseable_arguments_are_withheld_not_replaced_by_an_empty_object() {
    let corrupt = function_call("3", "bash", r#"{"command":"ls"#, "completed");
    let sse = [
        frame(json!({"type": "response.output_item.done", "output_index": 0, "item": corrupt})),
        terminal("completed", Value::Null, vec![corrupt.clone()]),
    ]
    .concat();
    let (calls, metadata) = stream(sse).await;
    assert!(calls.is_empty(), "{calls:?}");
    assert_eq!(metadata.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(metadata.tool_calls_dropped, 1);
    assert_eq!(metadata.tool_calls_truncated_executed, 0);
}

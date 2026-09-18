//! Tests for the OpenResponses protocol: replay.

// Open Responses Protocol Driver
//
// Implementation of the Open Responses specification (https://www.openresponses.org/)
// an open-source, vendor-neutral API standard for multi-provider LLM interfaces.
//
// Rate limit handling: On 429 errors, the driver automatically retries with
// exponential backoff, respecting x-ratelimit-reset-* and retry-after headers.
// Retry metadata is included in the response for observability.
//
// The spec is inspired by and interoperable with the OpenAI Responses API, offering:
// - One spec, many providers (OpenAI, Anthropic, Gemini, local models)
// - Agentic loop support with tool calls and state machines
// - Semantic streaming events (not raw text deltas)
// - 40-80% better cache utilization vs Chat Completions API
// - Native stateful conversation support
//
// Specification: https://www.openresponses.org/specification
// GitHub: https://github.com/openresponses/openresponses
//
// The Chat Completions API remains supported for backward compatibility.

use serde_json::json;

use crate::driver_registry::{
    ChatDriver, LlmCallConfig, LlmMessage, LlmMessageContent, LlmMessageRole, LlmStreamEvent,
};
use crate::llm_retry::LlmRetryConfig;
use crate::openresponses_types::{self as types, StreamingEvent};
use crate::tool_types::ToolDefinition;

use super::*;

use super::tests_support::*;

/// Wire-level EVE-523 reproducer: drive the real `chat_completion_stream`
/// against a mock endpoint on a non-OpenAI host. Even with a
/// `previous_response_id` in config, the request on the wire must omit it and
/// carry the FULL transcript (user task + assistant turn + tool result), so a
/// stateless gateway that ignores `previous_response_id` still sees the task.
#[tokio::test]
async fn stateless_gateway_request_replays_full_transcript_on_the_wire() {
    use crate::tool_types::ToolCall;
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Keep the endpoint distinct from connections owned by earlier test runtimes.
    let server = MockServer::builder().start().await;
    // Any 200 lets the request through; we inspect the captured request, not
    // the (empty) streamed body.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let endpoint = crate::runtime_provider::RuntimeProvider::new(
        "stateless-test",
        OpenResponsesProtocolChatDriver::new(),
    )
    .base_url(format!("{}/v1", server.uri()))
    .auth(crate::runtime_provider::BearerAuth::new("test-key"));
    let driver = OpenResponsesProtocolChatDriver::new();

    let messages = vec![
        LlmMessage::text(LlmMessageRole::System, "You are helpful"),
        LlmMessage::text(LlmMessageRole::User, "upgrade dependencies"),
        LlmMessage {
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text("Let me look.".to_string()),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                name: "read_file".to_string(),
                arguments: json!({"path": "Cargo.toml"}),
            }]),
            tool_call_id: None,
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
            native_tool_calls: Vec::new(),
        },
        LlmMessage {
            role: LlmMessageRole::Tool,
            content: LlmMessageContent::Text("[package]…".to_string()),
            tool_calls: None,
            tool_call_id: Some("call_1".to_string()),
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
            native_tool_calls: Vec::new(),
        },
    ];

    let config = LlmCallConfig {
        speed: None,
        verbosity: None,
        model: "some/model".to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![],
        reasoning_effort: None,
        metadata: std::collections::HashMap::new(),
        // Continuation handle from a prior turn — must be ignored on a
        // stateless gateway.
        previous_response_id: Some("gen-turn-1".to_string()),
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: None,
        driver_options: Default::default(),
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
        limits: Default::default(),
        reasoning_state: None,
    };

    // Fire the request. The stream body is irrelevant for this assertion.
    let _ = driver
        .chat_completion_stream(endpoint.endpoint(), messages, &config)
        .await;

    let requests = server
        .received_requests()
        .await
        .expect("mock server recorded requests");
    assert_eq!(requests.len(), 1, "exactly one request should be sent");
    let body: serde_json::Value = requests[0].body_json().expect("request body is JSON");

    // previous_response_id must be absent (skipped) — the gateway would ignore it.
    assert!(
        body.get("previous_response_id").is_none(),
        "stateless gateway request must omit previous_response_id; body: {body}"
    );

    // The full transcript must be replayed: user message, assistant message,
    // function_call, and function_call_output (instructions carry the system msg).
    let input = body["input"].as_array().expect("input is an array");
    assert_eq!(
        input.len(),
        4,
        "full transcript must be replayed on a stateless gateway; got {input:?}"
    );
    assert_eq!(body["instructions"], "You are helpful");
    let has_user_task = input
        .iter()
        .any(|item| item["type"] == "message" && item["role"] == "user");
    assert!(
        has_user_task,
        "the original user task must be replayed; got {input:?}"
    );
    let has_tool_output = input
        .iter()
        .any(|item| item["type"] == "function_call_output");
    assert!(
        has_tool_output,
        "the latest tool result must still be present; got {input:?}"
    );
}

#[tokio::test]
async fn rejected_stateful_continuation_replays_repaired_transcript_once() {
    use crate::tool_types::ToolCall;
    use futures::StreamExt;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({
            "previous_response_id": "resp_tool_turn"
        })))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "type": "invalid_request_error",
                "message": "No tool output found for function call call_1"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let completed = r#"data: {"type":"response.completed","response":{"id":"resp_recovered","status":"completed","model":"gpt-5.4","output":[],"usage":{"input_tokens":4,"output_tokens":1,"total_tokens":5}}}

"#;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(completed),
        )
        .expect(1)
        .mount(&server)
        .await;

    let endpoint = crate::runtime_provider::RuntimeProvider::new(
        "stateful-test",
        OpenResponsesProtocolChatDriver::new(),
    )
    .base_url(format!("{}/v1", server.uri()))
    .auth(crate::runtime_provider::BearerAuth::new("test-key"));
    let driver = OpenResponsesProtocolChatDriver::new()
        .with_stateful_responses(true)
        .with_retry_config(LlmRetryConfig::no_retry());
    let messages = vec![
        LlmMessage::text(LlmMessageRole::User, "inspect the project"),
        LlmMessage {
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text(String::new()),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                name: "read_file".to_string(),
                arguments: json!({"path": "Cargo.toml"}),
            }]),
            tool_call_id: None,
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
            native_tool_calls: Vec::new(),
        },
        LlmMessage {
            role: LlmMessageRole::Tool,
            content: LlmMessageContent::Text("[package]".to_string()),
            tool_calls: None,
            tool_call_id: Some("call_1".to_string()),
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
            native_tool_calls: Vec::new(),
        },
    ];
    let config = LlmCallConfig {
        speed: None,
        verbosity: None,
        model: "gpt-5.4".to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![],
        reasoning_effort: Some(crate::model::ReasoningEffort::High),
        metadata: std::collections::HashMap::new(),
        previous_response_id: Some("resp_tool_turn".to_string()),
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: None,
        driver_options: Default::default(),
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
        limits: Default::default(),
        reasoning_state: None,
    };

    let mut stream = driver
        .chat_completion_stream(endpoint.endpoint(), messages, &config)
        .await
        .expect("continuation should recover");
    while let Some(event) = stream.next().await {
        event.expect("valid recovered event");
    }

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(requests.len(), 2);
    let first: serde_json::Value = requests[0].body_json().expect("first body");
    let second: serde_json::Value = requests[1].body_json().expect("second body");
    assert_eq!(first["previous_response_id"], "resp_tool_turn");
    assert!(
        first.get("include").is_none(),
        "stateful continuations must not request encrypted reasoning: {first}"
    );
    assert!(second.get("previous_response_id").is_none());
    let replay = second["input"].as_array().expect("replay input");
    assert!(replay.iter().any(|item| item["type"] == "function_call"));
    assert!(
        replay
            .iter()
            .any(|item| item["type"] == "function_call_output")
    );
}

#[tokio::test]
async fn openrouter_provider_does_not_send_hosted_tool_search() {
    use crate::tool_types::DeferrablePolicy;
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let endpoint = crate::runtime_provider::RuntimeProvider::new(
        "openrouter-test",
        OpenResponsesProtocolChatDriver::new(),
    )
    .base_url(format!("{}/v1", server.uri()))
    .auth(crate::runtime_provider::BearerAuth::new("test-key"));
    let driver = OpenResponsesProtocolChatDriver::new();

    let tools: Vec<ToolDefinition> = (0..16)
        .map(|i| {
            make_tool(
                &format!("tool_{i}"),
                Some("General"),
                DeferrablePolicy::Automatic,
            )
        })
        .collect();

    let config = LlmCallConfig {
        speed: None,
        verbosity: None,
        model: "gpt-5.4".to_string(),
        temperature: None,
        max_tokens: None,
        tools,
        reasoning_effort: None,
        metadata: std::collections::HashMap::new(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: Some(crate::driver_registry::ToolSearchConfig {
            enabled: true,
            threshold: 15,
        }),
        prompt_cache: None,
        driver_options: Default::default(),
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
        limits: Default::default(),
        reasoning_state: None,
    };

    let messages = vec![LlmMessage::text(LlmMessageRole::User, "hello")];
    let _ = driver
        .chat_completion_stream(endpoint.endpoint(), messages, &config)
        .await;

    let requests = server
        .received_requests()
        .await
        .expect("mock server recorded requests");
    assert_eq!(requests.len(), 1, "exactly one request should be sent");
    let body: serde_json::Value = requests[0].body_json().expect("request body is JSON");
    let tools = body["tools"].as_array().expect("tools is an array");

    assert!(
        tools.iter().all(|tool| tool["type"] == "function"),
        "OpenRouter should receive regular function tools, not hosted tool_search payloads: {tools:?}"
    );
    assert!(
        tools.iter().all(|tool| tool.get("defer_loading").is_none()),
        "OpenRouter tool schemas should not be deferred by hosted tool_search: {tools:?}"
    );
    assert_eq!(
        body["input"],
        json!([{"type": "message", "role": "user", "content": "hello"}])
    );
}

#[tokio::test]
async fn openai_provider_omits_openrouter_routing_controls() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let endpoint = crate::runtime_provider::RuntimeProvider::new(
        "openai-test",
        OpenResponsesProtocolChatDriver::new(),
    )
    .base_url(format!("{}/v1", server.uri()))
    .auth(crate::runtime_provider::BearerAuth::new("test-key"));
    let driver = OpenResponsesProtocolChatDriver::new();

    let mut metadata = std::collections::HashMap::new();
    metadata.insert("session_id".to_string(), "session_abc123".to_string());
    let config = LlmCallConfig {
        speed: None,
        verbosity: None,
        model: "gpt-5-mini".to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![],
        reasoning_effort: None,
        metadata,
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: None,
        // Opaque OpenRouter routing payload: the OpenAI driver must ignore
        // `driver_options` entries it does not own.
        driver_options: [(
            "openrouter/routing".to_string(),
            serde_json::json!({
                "models": ["openai/gpt-5-mini"],
                "route": "fallback",
            }),
        )]
        .into_iter()
        .collect(),
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
        limits: Default::default(),
        reasoning_state: None,
    };

    let messages = vec![LlmMessage::text(LlmMessageRole::User, "hello")];
    let _ = driver
        .chat_completion_stream(endpoint.endpoint(), messages, &config)
        .await;

    let requests = server
        .received_requests()
        .await
        .expect("mock server recorded requests");
    assert_eq!(requests.len(), 1, "exactly one request should be sent");
    let body: serde_json::Value = requests[0].body_json().expect("request body is JSON");

    assert!(body.get("models").is_none(), "body: {body}");
    assert!(body.get("route").is_none(), "body: {body}");
    assert!(body.get("provider").is_none(), "body: {body}");
    // The top-level session_id is OpenRouter-only; OpenAI must not receive it
    // even though the session id rides along in `metadata`.
    assert!(body.get("session_id").is_none(), "body: {body}");
    assert_eq!(body["metadata"]["session_id"], "session_abc123");
}

/// OpenAI-compatible gateways (e.g. OpenRouter) terminate the Responses SSE
/// stream with a chat-completions-style `[DONE]` sentinel that OpenAI's
/// native API does not send. It must be skipped, not surfaced as a spurious
/// `Error` event after the real completion. (EVE: caught by the OpenRouter
/// live chat smoke test.)
#[tokio::test]
async fn openresponses_stream_skips_done_sentinel() {
    use futures::StreamExt;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // A normal text delta followed by the trailing `[DONE]` sentinel.
    let body =
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\ndata: [DONE]\n\n";
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let endpoint = crate::runtime_provider::RuntimeProvider::new(
        "stream-test",
        OpenResponsesProtocolChatDriver::new(),
    )
    .base_url(format!("{}/v1", server.uri()))
    .auth(crate::runtime_provider::BearerAuth::new("test-key"));
    let driver = OpenResponsesProtocolChatDriver::new();
    let config = LlmCallConfig {
        speed: None,
        verbosity: None,
        model: "openai/gpt-5.6-luna".to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![],
        reasoning_effort: None,
        metadata: std::collections::HashMap::new(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: None,
        driver_options: Default::default(),
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
        limits: Default::default(),
        reasoning_state: None,
    };

    let stream = driver
        .chat_completion_stream(
            endpoint.endpoint(),
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &config,
        )
        .await
        .expect("stream should start");
    let events: Vec<_> = stream.collect().await;

    let mut text = String::new();
    for ev in &events {
        match ev.as_ref().expect("no transport error") {
            LlmStreamEvent::TextDelta(d) => text.push_str(d),
            LlmStreamEvent::Error(e) => {
                panic!("[DONE] sentinel must not surface as an error: {e}")
            }
            _ => {}
        }
    }
    assert_eq!(text, "hi");
}

/// Deterministic contract coverage for the live matrix's stochastic
/// `get_current_time` case: prove the usable tool schema reaches the wire
/// and a fragmented OpenResponses tool call survives streaming parse.
#[tokio::test]
async fn tool_call_contract_covers_request_wire_and_stream_parser() {
    use futures::StreamExt;
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let body = concat!(
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"get_current_time\",\"arguments\":\"\"}}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"{\\\"timezone\\\":\\\"UTC\\\"\"}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\",\\\"format\\\":\\\"iso8601\\\"}\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"get_current_time\",\"arguments\":\"{\\\"timezone\\\":\\\"UTC\\\",\\\"format\\\":\\\"iso8601\\\"}\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"model\":\"openai/gpt-5.6-luna\",\"output\":[],\"usage\":{\"input_tokens\":4,\"output_tokens\":2,\"total_tokens\":6}}}\n\n",
        "data: [DONE]\n\n",
    );
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let endpoint = crate::runtime_provider::RuntimeProvider::new(
        "openrouter-contract-test",
        OpenResponsesProtocolChatDriver::new(),
    )
    .base_url(format!("{}/v1", server.uri()))
    .auth(crate::runtime_provider::BearerAuth::new("test-key"));
    let driver = OpenResponsesProtocolChatDriver::new();
    let config = LlmCallConfig {
        speed: None,
        verbosity: None,
        model: "openai/gpt-5.6-luna".to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![ToolDefinition::Builtin(crate::tool_types::BuiltinTool {
            name: "get_current_time".to_string(),
            display_name: None,
            description: "Get the current time in a timezone.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "timezone": { "type": "string" },
                    "format": { "type": "string", "enum": ["iso8601", "unix", "human"] }
                },
                "required": ["timezone"]
            }),
            policy: crate::tool_types::ToolPolicy::Auto,
            category: None,
            deferrable: crate::tool_types::DeferrablePolicy::Never,
            hints: crate::tool_types::ToolHints::default(),
            full_parameters: None,
        })],
        reasoning_effort: None,
        metadata: std::collections::HashMap::new(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: None,
        driver_options: Default::default(),
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
        limits: Default::default(),
        reasoning_state: None,
    };

    let stream = driver
        .chat_completion_stream(
            endpoint.endpoint(),
            vec![LlmMessage::text(LlmMessageRole::User, "What time is it?")],
            &config,
        )
        .await
        .expect("stream should start");
    let events: Vec<_> = stream.collect().await;

    let tool_calls = events
        .iter()
        .filter_map(|event| match event.as_ref().expect("valid stream event") {
            LlmStreamEvent::ToolCalls(calls) => Some(calls),
            _ => None,
        })
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].id, "call_1");
    assert_eq!(tool_calls[0].name, "get_current_time");
    assert_eq!(
        tool_calls[0].arguments,
        json!({"timezone": "UTC", "format": "iso8601"})
    );
    assert!(events.iter().any(|event| matches!(
        event.as_ref(),
        Ok(LlmStreamEvent::Done(metadata))
            if metadata.finish_reason.as_deref() == Some("tool_calls")
    )));

    let requests = server.received_requests().await.expect("captured request");
    let request: serde_json::Value = requests[0].body_json().expect("request JSON");
    let tool = &request["tools"][0];
    assert_eq!(tool["type"], "function");
    assert_eq!(tool["name"], "get_current_time");
    assert_eq!(tool["parameters"]["type"], "object");
    assert_eq!(tool["strict"], true);
    assert_eq!(
        tool["parameters"]["required"],
        json!(["format", "timezone"])
    );
    assert_eq!(
        tool["parameters"]["properties"]["format"]["type"],
        json!(["string", "null"])
    );
    assert_eq!(tool["parameters"]["additionalProperties"], false);
}

// ========================================================================
// Compact endpoint tests
// ========================================================================

// ========================================================================
// OpenAI Thinking/Reasoning Support Tests
// ========================================================================

#[test]
fn test_reasoning_input_item_serialization() {
    let item = ResponsesInputItem::Reasoning {
        r#type: "reasoning".to_string(),
        id: "rs_00000001".to_string(),
        encrypted_content: "encrypted_reasoning_context_here".to_string(),
        summary: Vec::new(),
    };

    let json = serde_json::to_value(&item).unwrap();
    assert_eq!(json["type"], "reasoning");
    // The API rejects a reasoning input item with no `summary` key, so an
    // empty summary must still serialize as `[]` rather than vanish.
    assert_eq!(
        json["summary"],
        serde_json::json!([]),
        "summary is required even when empty"
    );
    assert_eq!(json["id"], "rs_00000001");
    assert_eq!(
        json["encrypted_content"],
        "encrypted_reasoning_context_here"
    );
}

/// Every replayed reasoning item carries `summary`, and carries the
/// provider's own summary segments when it had them.
///
/// The Responses API rejects a reasoning input item without the key —
/// `400 … \`input[1]\` missing required field \`summary\`` — which took
/// `main` red against a live provider once reasoning replay went out under
/// provider-issued ids. Most artifacts carry no summary (the provider only
/// sends one when the request asks), so the empty case is the common one.
#[test]
fn test_build_input_reasoning_items_always_carry_a_summary() {
    let messages = vec![
        LlmMessage::text(LlmMessageRole::User, "Think"),
        LlmMessage {
            configuration_update: None,
            native_tool_calls: Vec::new(),
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text("No summary on this one.".to_string()),
            tool_calls: None,
            tool_call_id: None,
            phase: None,
            reasoning: vec![
                crate::reasoning::ReasoningContentPart::opaque("openai")
                    .with_item_id("rs_bare")
                    .with_encrypted("enc_bare"),
            ],
        },
        LlmMessage {
            configuration_update: None,
            native_tool_calls: Vec::new(),
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text("This one was summarized.".to_string()),
            tool_calls: None,
            tool_call_id: None,
            phase: None,
            reasoning: vec![
                crate::reasoning::ReasoningContentPart::opaque("openai")
                    .with_item_id("rs_summarized")
                    .with_encrypted("enc_summarized")
                    .with_text(crate::reasoning::ReasoningText::Summary {
                        parts: vec!["First I checked.".to_string(), "Then I read.".to_string()],
                    }),
            ],
        },
    ];

    let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);
    let reasoning: Vec<serde_json::Value> = input
        .iter()
        .map(|item| serde_json::to_value(item).unwrap())
        .filter(|json| json["type"] == "reasoning")
        .collect();
    assert_eq!(reasoning.len(), 2, "both artifacts must be replayed");

    for item in &reasoning {
        assert!(
            item.get("summary").is_some(),
            "summary is required on every reasoning input item: {item}"
        );
    }

    assert_eq!(
        reasoning[0]["summary"],
        serde_json::json!([]),
        "an artifact with no summary replays an empty one, not a missing key"
    );
    assert_eq!(
        reasoning[1]["summary"],
        serde_json::json!([
            { "type": "summary_text", "text": "First I checked." },
            { "type": "summary_text", "text": "Then I read." },
        ]),
        "the provider's own summary segments replay verbatim"
    );
}

#[test]
fn test_build_input_replays_reasoning_before_its_message() {
    let messages = vec![
        LlmMessage::text(LlmMessageRole::User, "Think about this deeply"),
        LlmMessage {
            configuration_update: None,
            native_tool_calls: Vec::new(),
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text("I have thought about this.".to_string()),
            tool_calls: None,
            tool_call_id: None,
            phase: None,
            reasoning: vec![
                crate::reasoning::ReasoningContentPart::opaque("openai")
                    .with_item_id("rs_reply")
                    .with_encrypted("encrypted_reasoning_token_123"),
            ],
        },
        LlmMessage::text(LlmMessageRole::User, "What else?"),
    ];

    let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

    // Should have: user message, reasoning item, assistant message, user message
    assert_eq!(input.len(), 4);

    // First is user message
    let json = serde_json::to_value(&input[0]).unwrap();
    assert_eq!(json["role"], "user");
    assert_eq!(json["content"], "Think about this deeply");

    // Second is the reasoning item, ahead of the message it belongs to, and
    // keyed by the id the provider issued.
    let json = serde_json::to_value(&input[1]).unwrap();
    assert_eq!(json["type"], "reasoning");
    assert_eq!(json["id"], "rs_reply");
    assert_eq!(json["encrypted_content"], "encrypted_reasoning_token_123");

    // Third is assistant message
    let json = serde_json::to_value(&input[2]).unwrap();
    assert_eq!(json["role"], "assistant");
    assert_eq!(json["content"], "I have thought about this.");

    // Fourth is second user message
    let json = serde_json::to_value(&input[3]).unwrap();
    assert_eq!(json["role"], "user");
}

#[test]
fn test_build_input_replays_reasoning_with_tool_calls() {
    use crate::tool_types::ToolCall;

    let messages = vec![
        LlmMessage::text(LlmMessageRole::User, "What time is it? Think carefully."),
        LlmMessage {
            configuration_update: None,
            native_tool_calls: Vec::new(),
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text("Let me check.".to_string()),
            tool_calls: Some(vec![ToolCall {
                id: "call_123".to_string(),
                name: "get_time".to_string(),
                arguments: json!({}),
            }]),
            tool_call_id: None,
            phase: None,
            reasoning: vec![
                crate::reasoning::ReasoningContentPart::opaque("openai")
                    .with_item_id("rs_tool")
                    .with_encrypted("encrypted_token_xyz"),
            ],
        },
        LlmMessage {
            role: LlmMessageRole::Tool,
            content: LlmMessageContent::Text("10:30 AM".to_string()),
            tool_calls: None,
            tool_call_id: Some("call_123".to_string()),
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
            native_tool_calls: Vec::new(),
        },
    ];

    let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

    // Should have: user, reasoning, assistant, function_call, function_call_output
    assert_eq!(input.len(), 5);

    // Reasoning item comes before assistant message
    let json = serde_json::to_value(&input[1]).unwrap();
    assert_eq!(json["type"], "reasoning");
    assert_eq!(json["id"], "rs_tool");
    assert_eq!(json["encrypted_content"], "encrypted_token_xyz");

    // Assistant message
    let json = serde_json::to_value(&input[2]).unwrap();
    assert_eq!(json["role"], "assistant");

    // Function call
    let json = serde_json::to_value(&input[3]).unwrap();
    assert_eq!(json["type"], "function_call");
    assert_eq!(json["call_id"], "call_123");

    // Function call output
    let json = serde_json::to_value(&input[4]).unwrap();
    assert_eq!(json["type"], "function_call_output");
}

#[test]
fn test_build_input_without_thinking_signature() {
    // Assistant message with thinking but NO thinking_signature should not emit reasoning item
    let messages = vec![
        LlmMessage::text(LlmMessageRole::User, "Hello"),
        LlmMessage {
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text("Hi there!".to_string()),
            tool_calls: None,
            tool_call_id: None,
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
            native_tool_calls: Vec::new(),
        },
    ];

    let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

    // Should have: user message, assistant message (no reasoning item)
    assert_eq!(input.len(), 2);

    // Verify no reasoning item
    let json = serde_json::to_value(&input[0]).unwrap();
    assert_eq!(json["role"], "user");

    let json = serde_json::to_value(&input[1]).unwrap();
    assert_eq!(json["role"], "assistant");
}

#[test]
fn test_handle_streaming_event_reasoning_encrypted_content() {
    use std::sync::Mutex;

    let input_tokens = Mutex::new(0u32);
    let output_tokens = Mutex::new(0u32);
    let cache_read_tokens = Mutex::new(None);
    let accumulated_tool_calls = Mutex::new(ToolCallStream::default());
    let finish_reason = Mutex::new(None);
    let deferred_events = Mutex::new(Vec::new());

    // Create an OutputItemDone event with Reasoning item containing encrypted_content
    let event = StreamingEvent::OutputItemDone {
        sequence_number: 5,
        output_index: 0,
        item: Some(types::OutputItem::Reasoning {
            id: "rs_001".to_string(),
            summary: vec![],
            content: None,
            encrypted_content: Some("encrypted_reasoning_data".to_string()),
        }),
    };

    let result = handle_streaming_event(
        event,
        &input_tokens,
        &output_tokens,
        &cache_read_tokens,
        &accumulated_tool_calls,
        &finish_reason,
        &deferred_events,
        "gpt-5".to_string(),
        None,
    );

    // Should emit a reasoning artifact carrying the provider id and the
    // encrypted payload needed to replay it.
    match result {
        LlmStreamEvent::ReasoningItem(item) => {
            assert_eq!(item.provider, "openai");
            assert_eq!(item.item_id.as_deref(), Some("rs_001"));
            assert_eq!(item.encrypted.as_deref(), Some("encrypted_reasoning_data"));
            assert!(item.text.is_none());
            assert!(item.tokens.is_none());
        }
        other => panic!("Expected ReasoningItem event, got {:?}", other),
    }
}

#[test]
fn output_item_added_message_surfaces_native_phase_hint() {
    use std::sync::Mutex;

    // EVE-774: OpenAI Responses stamps the assistant item's phase on
    // `response.output_item.added` (before any text delta). The driver must
    // surface it as a mid-stream `MessagePhase` hint.
    for (wire, expected) in [
        (
            "commentary",
            crate::execution_phase::ExecutionPhase::Commentary,
        ),
        (
            "final_answer",
            crate::execution_phase::ExecutionPhase::FinalAnswer,
        ),
    ] {
        let event: StreamingEvent = serde_json::from_value(serde_json::json!({
            "type": "response.output_item.added",
            "sequence_number": 1,
            "output_index": 0,
            "item": {
                "type": "message",
                "id": "msg_001",
                "status": "in_progress",
                "role": "assistant",
                "content": [],
                "phase": wire,
            }
        }))
        .expect("output_item.added should deserialize");

        let result = handle_streaming_event(
            event,
            &Mutex::new(0),
            &Mutex::new(0),
            &Mutex::new(None),
            &Mutex::new(ToolCallStream::default()),
            &Mutex::new(None),
            &Mutex::new(Vec::new()),
            "gpt-5".to_string(),
            None,
        );

        match result {
            LlmStreamEvent::MessagePhase(phase) => assert_eq!(phase, expected),
            other => panic!("Expected MessagePhase({expected:?}), got {other:?}"),
        }
    }
}

#[test]
fn output_item_added_message_without_phase_is_noop() {
    use std::sync::Mutex;

    // A message item that carries no phase yields no hint (empty text delta),
    // never a fabricated phase.
    let event: StreamingEvent = serde_json::from_value(serde_json::json!({
        "type": "response.output_item.added",
        "sequence_number": 1,
        "output_index": 0,
        "item": {
            "type": "message",
            "id": "msg_002",
            "status": "in_progress",
            "role": "assistant",
            "content": [],
        }
    }))
    .expect("output_item.added should deserialize");

    let result = handle_streaming_event(
        event,
        &Mutex::new(0),
        &Mutex::new(0),
        &Mutex::new(None),
        &Mutex::new(ToolCallStream::default()),
        &Mutex::new(None),
        &Mutex::new(Vec::new()),
        "gpt-5".to_string(),
        None,
    );

    match result {
        LlmStreamEvent::TextDelta(d) => assert!(d.is_empty()),
        other => panic!("Expected empty TextDelta, got {other:?}"),
    }
}

#[test]
fn response_failed_preserves_provider_error_code() {
    use std::sync::Mutex;

    let event: StreamingEvent = serde_json::from_value(serde_json::json!({
        "type": "response.failed",
        "sequence_number": 7,
        "response": {
            "id": "resp_failed",
            "object": "response",
            "created_at": 1,
            "status": "failed",
            "model": "gpt-5",
            "output": [],
            "tools": [],
            "error": {
                "code": "processing_error",
                "message": "An error occurred while processing your request."
            }
        }
    }))
    .expect("response.failed should deserialize");

    let result = handle_streaming_event(
        event,
        &Mutex::new(0),
        &Mutex::new(0),
        &Mutex::new(None),
        &Mutex::new(ToolCallStream::default()),
        &Mutex::new(None),
        &Mutex::new(Vec::new()),
        "gpt-5".to_string(),
        None,
    );

    let LlmStreamEvent::Error(error) = result else {
        panic!("expected structured stream error");
    };
    assert_eq!(error.code.as_deref(), Some("processing_error"));
    assert!(crate::llm_retry::is_transient_stream_error(&error));
}

#[test]
fn test_handle_streaming_event_reasoning_without_encrypted_content() {
    use std::sync::Mutex;

    let input_tokens = Mutex::new(0u32);
    let output_tokens = Mutex::new(0u32);
    let cache_read_tokens = Mutex::new(None);
    let accumulated_tool_calls = Mutex::new(ToolCallStream::default());
    let finish_reason = Mutex::new(None);
    let deferred_events = Mutex::new(Vec::new());

    // Create an OutputItemDone event with Reasoning item but NO encrypted_content
    let event = StreamingEvent::OutputItemDone {
        sequence_number: 5,
        output_index: 0,
        item: Some(types::OutputItem::Reasoning {
            id: "rs_001".to_string(),
            summary: vec![types::ContentPart::SummaryText {
                text: "Some summary".to_string(),
            }],
            content: None,
            encrypted_content: None, // No encrypted content
        }),
    };

    let result = handle_streaming_event(
        event,
        &input_tokens,
        &output_tokens,
        &cache_read_tokens,
        &accumulated_tool_calls,
        &finish_reason,
        &deferred_events,
        "gpt-5".to_string(),
        None,
    );

    // Should still emit the artifact carrying the safe summary even when no
    // encrypted content is present so the durable reasoning record survives.
    match result {
        LlmStreamEvent::ReasoningItem(item) => {
            assert_eq!(item.provider, "openai");
            assert_eq!(item.item_id.as_deref(), Some("rs_001"));
            assert!(item.encrypted.is_none());
            assert_eq!(
                item.text,
                Some(crate::reasoning::ReasoningText::Summary {
                    parts: vec!["Some summary".to_string()],
                })
            );
        }
        other => panic!("Expected ReasoningItem event, got {:?}", other),
    }
}

#[test]
fn test_handle_streaming_event_reasoning_drops_plaintext_content() {
    use std::sync::Mutex;

    let input_tokens = Mutex::new(0u32);
    let output_tokens = Mutex::new(0u32);
    let cache_read_tokens = Mutex::new(None);
    let accumulated_tool_calls = Mutex::new(ToolCallStream::default());
    let finish_reason = Mutex::new(None);
    let deferred_events = Mutex::new(Vec::new());

    // Reasoning item with plaintext content and a non-summary content part in `summary`.
    // Both must be excluded from the emitted ReasonItem.
    let event = StreamingEvent::OutputItemDone {
        sequence_number: 5,
        output_index: 0,
        item: Some(types::OutputItem::Reasoning {
            id: "rs_002".to_string(),
            summary: vec![
                types::ContentPart::SummaryText {
                    text: "safe summary".to_string(),
                },
                types::ContentPart::ReasoningText {
                    text: "SECRET hidden reasoning".to_string(),
                },
            ],
            content: Some(vec![types::ContentPart::ReasoningText {
                text: "SECRET hidden reasoning".to_string(),
            }]),
            encrypted_content: Some("opaque".to_string()),
        }),
    };

    let result = handle_streaming_event(
        event,
        &input_tokens,
        &output_tokens,
        &cache_read_tokens,
        &accumulated_tool_calls,
        &finish_reason,
        &deferred_events,
        "gpt-5".to_string(),
        None,
    );

    match result {
        LlmStreamEvent::ReasoningItem(item) => {
            assert_eq!(
                item.text,
                Some(crate::reasoning::ReasoningText::Summary {
                    parts: vec!["safe summary".to_string()],
                })
            );
            assert_eq!(item.encrypted.as_deref(), Some("opaque"));
        }
        other => panic!("Expected ReasoningItem event, got {:?}", other),
    }
}

#[test]
fn test_handle_streaming_event_reasoning_delta() {
    use std::sync::Mutex;

    let input_tokens = Mutex::new(0u32);
    let output_tokens = Mutex::new(0u32);
    let cache_read_tokens = Mutex::new(None);
    let accumulated_tool_calls = Mutex::new(ToolCallStream::default());
    let finish_reason = Mutex::new(None);
    let deferred_events = Mutex::new(Vec::new());

    // Raw reasoning from o-series reaches the reasoning channel, not text.
    let event = StreamingEvent::ReasoningDelta {
        sequence_number: 3,
        item_id: "rs_001".to_string(),
        output_index: 0,
        content_index: 0,
        delta: "Let me reason about this...".to_string(),
        obfuscation: None,
    };

    let result = handle_streaming_event(
        event,
        &input_tokens,
        &output_tokens,
        &cache_read_tokens,
        &accumulated_tool_calls,
        &finish_reason,
        &deferred_events,
        "o3".to_string(),
        None,
    );

    match result {
        LlmStreamEvent::ReasoningDelta { delta, summary } => {
            assert_eq!(delta, "Let me reason about this...");
            assert!(!summary, "raw chain-of-thought is not a summary");
        }
        _ => panic!("Expected ReasoningDelta, got {:?}", result),
    }
}

#[test]
fn test_handle_streaming_event_reasoning_summary_delta() {
    use std::sync::Mutex;

    let input_tokens = Mutex::new(0u32);
    let output_tokens = Mutex::new(0u32);
    let cache_read_tokens = Mutex::new(None);
    let accumulated_tool_calls = Mutex::new(ToolCallStream::default());
    let finish_reason = Mutex::new(None);
    let deferred_events = Mutex::new(Vec::new());

    // A reasoning summary is a reasoning artifact. Routing it to the
    // assistant-text channel persisted it as the model's answer and
    // replayed it as the model's own prior output.
    let event = StreamingEvent::ReasoningSummaryDelta {
        sequence_number: 4,
        item_id: "rs_002".to_string(),
        output_index: 0,
        summary_index: 0,
        delta: "Breaking down the problem...".to_string(),
        obfuscation: None,
    };

    let result = handle_streaming_event(
        event,
        &input_tokens,
        &output_tokens,
        &cache_read_tokens,
        &accumulated_tool_calls,
        &finish_reason,
        &deferred_events,
        "gpt-5.2".to_string(),
        None,
    );

    match result {
        LlmStreamEvent::ReasoningDelta { delta, summary } => {
            assert_eq!(delta, "Breaking down the problem...");
            assert!(
                summary,
                "a reasoning summary must be labelled as such, not passed \
                 off as raw chain-of-thought"
            );
        }
        other => panic!(
            "reasoning summary must reach the reasoning channel, never \
             assistant text; got {other:?}"
        ),
    }
}

#[test]
fn test_request_reasoning_none_is_omitted() {
    // When reasoning effort is "none", the reasoning field should be omitted
    // to avoid API errors on models that don't support reasoning params
    let config = LlmCallConfig {
        speed: None,
        verbosity: None,
        model: "gpt-5.2".to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![],
        reasoning_effort: Some(crate::model::ReasoningEffort::None),
        metadata: std::collections::HashMap::new(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: None,
        driver_options: Default::default(),
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
        limits: Default::default(),
        reasoning_state: None,
    };

    // Simulate the driver's filter logic
    let reasoning = config
        .reasoning_effort
        .filter(crate::model::ReasoningEffort::requests_reasoning)
        .map(|effort| ResponsesReasoning {
            effort: effort.as_str().to_string(),
            summary: "detailed".to_string(),
        });

    assert!(
        reasoning.is_none(),
        "reasoning should be None for effort=none"
    );
}

#[test]
fn test_request_reasoning_high_is_included() {
    // When reasoning effort is "high", the reasoning field should be present
    let config = LlmCallConfig {
        speed: None,
        verbosity: None,
        model: "gpt-5.2".to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![],
        reasoning_effort: Some(crate::model::ReasoningEffort::High),
        metadata: std::collections::HashMap::new(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: None,
        driver_options: Default::default(),
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
        limits: Default::default(),
        reasoning_state: None,
    };

    let reasoning = config
        .reasoning_effort
        .filter(crate::model::ReasoningEffort::requests_reasoning)
        .map(|effort| ResponsesReasoning {
            effort: effort.as_str().to_string(),
            summary: "detailed".to_string(),
        });

    assert!(
        reasoning.is_some(),
        "reasoning should be present for effort=high"
    );
    let r = reasoning.unwrap();
    assert_eq!(r.effort, "high");
    assert_eq!(r.summary, "detailed");
}

#[test]
fn test_request_reasoning_none_case_insensitive() {
    // "None", "NONE", "none" should all be filtered out
    for effort in &["none", "None", "NONE"] {
        let reasoning = Some(effort.to_string())
            .as_ref()
            .filter(|e| !e.eq_ignore_ascii_case("none"))
            .cloned();

        assert!(
            reasoning.is_none(),
            "effort={effort:?} should be filtered out"
        );
    }
}

#[test]
fn test_build_input_assistant_without_thinking_or_tools() {
    // Plain assistant message (no thinking, no tool calls) should just be a message
    let messages = vec![
        LlmMessage::text(LlmMessageRole::User, "Hello"),
        LlmMessage {
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text("Hi there!".to_string()),
            tool_calls: None,
            tool_call_id: None,
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
            native_tool_calls: Vec::new(),
        },
    ];

    let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

    assert_eq!(input.len(), 2);
    let json = serde_json::to_value(&input[1]).unwrap();
    assert_eq!(json["role"], "assistant");
    assert!(json.get("type").is_none() || json["type"] == "message");
}

/// Each reasoning item replays under the id the provider issued for it.
///
/// This previously asserted only that synthesized ids were *unique*, which
/// a counter satisfies. Uniqueness was never the requirement: the API
/// resolves reasoning items by the `rs_…` id it handed out, so an id the
/// provider never issued is not usable however distinct it is.
#[test]
fn test_build_input_reasoning_items_keep_provider_ids() {
    let messages = vec![
        LlmMessage::text(LlmMessageRole::User, "First question"),
        LlmMessage {
            configuration_update: None,
            native_tool_calls: Vec::new(),
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text("First answer.".to_string()),
            tool_calls: None,
            tool_call_id: None,
            phase: None,
            reasoning: vec![
                crate::reasoning::ReasoningContentPart::opaque("openai")
                    .with_item_id("rs_alpha")
                    .with_encrypted("encrypted_1"),
            ],
        },
        LlmMessage::text(LlmMessageRole::User, "Second question"),
        LlmMessage {
            configuration_update: None,
            native_tool_calls: Vec::new(),
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text("Second answer.".to_string()),
            tool_calls: None,
            tool_call_id: None,
            phase: None,
            reasoning: vec![
                crate::reasoning::ReasoningContentPart::opaque("openai")
                    .with_item_id("rs_beta")
                    .with_encrypted("encrypted_2"),
            ],
        },
    ];

    let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

    // user, reasoning_1, assistant, user, reasoning_2, assistant
    assert_eq!(input.len(), 6);

    let r1 = serde_json::to_value(&input[1]).unwrap();
    let r2 = serde_json::to_value(&input[4]).unwrap();

    assert_eq!(r1["type"], "reasoning");
    assert_eq!(r1["id"], "rs_alpha");
    assert_eq!(r1["encrypted_content"], "encrypted_1");
    assert_eq!(r2["type"], "reasoning");
    assert_eq!(r2["id"], "rs_beta");
    assert_eq!(r2["encrypted_content"], "encrypted_2");
}

#[test]
fn test_build_input_with_phases_enabled() {
    use crate::execution_phase::ExecutionPhase;

    let messages = vec![
        LlmMessage::text(LlmMessageRole::System, "You are helpful"),
        LlmMessage::text(LlmMessageRole::User, "Hello"),
        LlmMessage {
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text("Working on it...".to_string()),
            tool_calls: Some(vec![crate::tool_types::ToolCall {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: json!({}),
            }]),
            tool_call_id: None,
            phase: Some(ExecutionPhase::Commentary),
            reasoning: Vec::new(),
            configuration_update: None,
            native_tool_calls: Vec::new(),
        },
        LlmMessage {
            role: LlmMessageRole::Tool,
            content: LlmMessageContent::Text("result".to_string()),
            tool_calls: None,
            tool_call_id: Some("call_1".to_string()),
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
            native_tool_calls: Vec::new(),
        },
    ];

    // With supports_phases=true, assistant message should include phase
    let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, true);
    let assistant_json = serde_json::to_value(&input[1]).unwrap();
    assert_eq!(assistant_json["phase"], "commentary");

    // With supports_phases=false, phase should be absent
    let (_, input_no_phases) = OpenResponsesProtocolChatDriver::build_input(&messages, false);
    let assistant_json_no = serde_json::to_value(&input_no_phases[1]).unwrap();
    assert!(assistant_json_no.get("phase").is_none() || assistant_json_no["phase"].is_null());
}

// ========================================================================
// tool_search / convert_tools_with_search tests
// ========================================================================

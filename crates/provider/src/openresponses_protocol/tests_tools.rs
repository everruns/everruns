//! Tests for the OpenResponses protocol: tools.

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

use futures::StreamExt;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

pub use crate::compact::{CompactContent, CompactInputItem, CompactRequest};
use crate::driver_registry::{
    ChatDriver, LlmCallConfig, LlmMessage, LlmMessageRole, LlmStreamEvent,
};
use crate::error::LlmErrorKind;
use crate::llm_retry::LlmRetryConfig;
use crate::openresponses_types::StreamingEvent;
use crate::tool_types::ToolDefinition;

use super::*;

use super::tests_support::*;

#[test]
fn test_hosted_tool_search_completed_event_preserves_response_id() {
    let event_json = r#"{
        "type": "response.completed",
        "sequence_number": 8,
        "response": {
            "id": "resp_tool_search",
            "object": "response",
            "created_at": 1780000000,
            "status": "completed",
            "model": "gpt-5.5",
            "output": [
                {
                    "type": "tool_search_call",
                    "execution": "server",
                    "call_id": null,
                    "status": "completed",
                    "arguments": { "paths": ["Math"] }
                },
                {
                    "type": "tool_search_output",
                    "execution": "server",
                    "call_id": null,
                    "status": "completed",
                    "tools": [
                        {
                            "type": "namespace",
                            "name": "Math",
                            "description": "Tools for Math",
                            "tools": [
                                {
                                    "type": "function",
                                    "name": "add",
                                    "description": "Add numbers.",
                                    "defer_loading": true,
                                    "parameters": {
                                        "type": "object",
                                        "properties": {
                                            "a": { "type": "number" },
                                            "b": { "type": "number" }
                                        },
                                        "required": ["a", "b"],
                                        "additionalProperties": false
                                    }
                                }
                            ]
                        }
                    ]
                },
                {
                    "type": "function_call",
                    "id": "fc_123",
                    "call_id": "call_123",
                    "name": "add",
                    "namespace": "Math",
                    "arguments": "{\"a\":7,\"b\":3}",
                    "status": "completed"
                }
            ],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5,
                "total_tokens": 15
            }
        }
    }"#;

    let event: StreamingEvent = serde_json::from_str(event_json).unwrap();
    let stream_event = handle_streaming_event(
        event,
        &Mutex::new(0),
        &Mutex::new(0),
        &Mutex::new(None),
        &Mutex::new(ToolCallStream::default()),
        &Mutex::new(Some("tool_calls".to_string())),
        &Mutex::new(Vec::new()),
        "gpt-5.5".to_string(),
        None,
    );

    match stream_event {
        LlmStreamEvent::Done(metadata) => {
            assert_eq!(metadata.response_id.as_deref(), Some("resp_tool_search"));
            assert_eq!(metadata.finish_reason.as_deref(), Some("tool_calls"));
        }
        other => panic!("expected Done event, got {other:?}"),
    }
}

#[test]
fn test_completed_event_normalizes_cache_inclusive_prompt_tokens() {
    // OpenAI reports `input_tokens` inclusive of cached reads. The driver
    // must normalize to the disjoint convention: prompt_tokens carries only
    // the non-cached remainder (input − cached), with cache reported on top.
    let event_json = r#"{
        "type": "response.completed",
        "sequence_number": 9,
        "response": {
            "id": "resp_cache",
            "object": "response",
            "created_at": 1780000000,
            "status": "completed",
            "model": "gpt-5.5",
            "output": [],
            "usage": {
                "input_tokens": 1000,
                "output_tokens": 20,
                "total_tokens": 1020,
                "input_tokens_details": { "cached_tokens": 800, "cache_write_tokens": 150 }
            }
        }
    }"#;

    let event: StreamingEvent = serde_json::from_str(event_json).unwrap();
    let stream_event = handle_streaming_event(
        event,
        &Mutex::new(0),
        &Mutex::new(0),
        &Mutex::new(None),
        &Mutex::new(ToolCallStream::default()),
        &Mutex::new(None),
        &Mutex::new(Vec::new()),
        "gpt-5.5".to_string(),
        None,
    );

    match stream_event {
        LlmStreamEvent::Done(metadata) => {
            // 1000 reported − 800 read − 150 written = 50 ordinary input.
            assert_eq!(metadata.prompt_tokens, Some(50));
            assert_eq!(metadata.cache_creation_tokens, Some(150));
            assert_eq!(metadata.cache_read_tokens, Some(800));
            // total_tokens stays the true prompt+output total (1000 + 20).
            assert_eq!(metadata.total_tokens, Some(1020));
        }
        other => panic!("expected Done event, got {other:?}"),
    }
}

#[test]
fn test_incomplete_event_maps_output_limit_to_length() {
    let event_json = r#"{
        "type": "response.incomplete",
        "sequence_number": 10,
        "response": {
            "id": "resp_incomplete",
            "object": "response",
            "created_at": 1780000000,
            "status": "incomplete",
            "incomplete_details": { "reason": "max_output_tokens" },
            "model": "gpt-5.5",
            "output": [],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5,
                "total_tokens": 15
            }
        }
    }"#;

    let event: StreamingEvent = serde_json::from_str(event_json).unwrap();
    let stream_event = handle_streaming_event(
        event,
        &Mutex::new(0),
        &Mutex::new(0),
        &Mutex::new(None),
        &Mutex::new(ToolCallStream::default()),
        &Mutex::new(None),
        &Mutex::new(Vec::new()),
        "gpt-5.5".to_string(),
        None,
    );

    match stream_event {
        LlmStreamEvent::Done(metadata) => {
            assert_eq!(metadata.finish_reason.as_deref(), Some("length"));
        }
        other => panic!("expected Done event, got {other:?}"),
    }
}

#[test]
fn test_sanitize_parameters_adds_missing_properties() {
    let params = json!({"type": "object", "additionalProperties": false});
    let sanitized = OpenResponsesProtocolChatDriver::sanitize_parameters(&params);
    assert_eq!(
        sanitized,
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    );
}

#[test]
fn test_sanitize_parameters_preserves_existing_properties() {
    let params = json!({"type": "object", "properties": {"x": {"type": "string"}}, "additionalProperties": false});
    let sanitized = OpenResponsesProtocolChatDriver::sanitize_parameters(&params);
    assert_eq!(sanitized, params);
}

#[test]
fn test_sanitize_parameters_ignores_non_object_types() {
    let params = json!({"type": "string"});
    let sanitized = OpenResponsesProtocolChatDriver::sanitize_parameters(&params);
    assert_eq!(sanitized, params);
}

#[test]
fn test_sanitize_parameters_rewrites_resend_email_lookaround() {
    let params = json!({
        "type": "object",
        "properties": {
            "email": {
                "type": "string",
                "pattern": "^(?!\\.)(?!.*\\.\\.)([A-Za-z0-9_'+\\-\\.]*)[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\\-]*\\.)+[A-Za-z]{2,}$"
            }
        }
    });

    let sanitized = OpenResponsesProtocolChatDriver::sanitize_parameters(&params);
    let pattern = sanitized["properties"]["email"]["pattern"]
        .as_str()
        .unwrap();

    assert!(!pattern.contains("(?!"));
    assert!(pattern.contains('@'));
}

// ========================================================================
// Provider-owned request auth (EVE-618 / EVE-856)
// ========================================================================

#[tokio::test]
async fn auth_headers_reach_wire_with_explicit_precedence_and_successful_response() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer};
    for (static_header, caller_override) in [(false, false), (true, false), (false, true)] {
        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(successful_auth_stream())
            .expect(1)
            .mount(&server)
            .await;
        let provider = crate::runtime_provider::RuntimeProvider::new(
            "auth-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(format!("{}/v1", server.uri()));
        let provider = if static_header {
            provider.auth(crate::runtime_provider::StaticHeaderAuth::new(
                "API-Key",
                "static-key",
            ))
        } else {
            provider.auth(crate::runtime_provider::BearerAuth::new("wire-key"))
        };
        let mut config = auth_test_config();
        if caller_override {
            config.extra_headers = vec![
                ("AUTHORIZATION".into(), "Bearer caller".into()),
                ("X-Route".into(), "caller-route".into()),
            ];
        }
        let driver = OpenResponsesProtocolChatDriver::new()
            .with_retry_config(LlmRetryConfig::no_retry())
            .with_request_extension(Arc::new(HeaderInjectingExtension));
        let stream = driver
            .chat_completion_stream(
                provider.endpoint(),
                vec![LlmMessage::text(LlmMessageRole::User, "hi")],
                &config,
            )
            .await
            .unwrap();
        assert_authenticated_stream(stream).await;
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let headers = &requests[0].headers;
        let expected = if caller_override {
            "Bearer caller"
        } else if static_header {
            "Bearer decoration"
        } else {
            "Bearer wire-key"
        };
        assert_eq!(
            headers
                .get_all("authorization")
                .iter()
                .map(|h| h.to_str().unwrap())
                .collect::<Vec<_>>(),
            vec![expected]
        );
        assert_eq!(
            headers.get("api-key").map(|h| h.to_str().unwrap()),
            static_header.then_some("static-key")
        );
        assert_eq!(
            headers["x-route"],
            if caller_override {
                "caller-route"
            } else {
                "fallback"
            }
        );
        assert_eq!(headers["content-type"], "application/json");
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["routing_marker"], "decorated");
        assert_eq!(body["model"], "gpt-5.4");
    }
}
#[tokio::test]
async fn refreshed_tokens_and_signed_payload_reach_each_retry_attempt() {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer token-1"))
        .respond_with(ResponseTemplate::new(503).set_body_string("overloaded"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer token-2"))
        .respond_with(successful_auth_stream())
        .expect(1)
        .mount(&server)
        .await;
    let signed = Arc::new(Mutex::new(Vec::new()));
    let provider = crate::runtime_provider::RuntimeProvider::new(
        "auth-test",
        OpenResponsesProtocolChatDriver::new(),
    )
    .base_url(format!("{}/v1", server.uri()))
    .auth(RecordingAuth {
        requests: signed.clone(),
        fail_on: None,
    });
    let driver = OpenResponsesProtocolChatDriver::new()
        .with_retry_config(auth_retry_config())
        .with_request_extension(Arc::new(HeaderInjectingExtension));
    let stream = driver
        .chat_completion_stream(
            provider.endpoint(),
            vec![LlmMessage::text(LlmMessageRole::User, "hi")],
            &auth_test_config(),
        )
        .await
        .unwrap();
    assert_authenticated_stream(stream).await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].headers["authorization"], "Bearer token-1");
    assert_eq!(requests[1].headers["authorization"], "Bearer token-2");
    assert_eq!(requests[0].body, requests[1].body);
    let signed = signed.lock().unwrap();
    assert_eq!(signed.len(), 2);
    for (attempt, request) in signed.iter().zip(&requests) {
        assert_eq!(attempt.method, "POST");
        assert_eq!(attempt.url, format!("{}/v1/responses", server.uri()));
        assert_eq!(attempt.body, request.body);
        let body: Value = serde_json::from_slice(&attempt.body).unwrap();
        assert_eq!(body["routing_marker"], "decorated");
    }
}

#[tokio::test]
async fn auth_failure_aborts_before_sending_or_reusing_an_expired_token() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    for fail_on in [1, 2] {
        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).set_body_string("overloaded"))
            .expect((fail_on - 1) as u64)
            .mount(&server)
            .await;
        let signed = Arc::new(Mutex::new(Vec::new()));
        let provider = crate::runtime_provider::RuntimeProvider::new(
            "auth-test",
            OpenResponsesProtocolChatDriver::new(),
        )
        .base_url(format!("{}/v1", server.uri()))
        .auth(RecordingAuth {
            requests: signed.clone(),
            fail_on: Some(fail_on),
        });
        let driver = OpenResponsesProtocolChatDriver::new().with_retry_config(auth_retry_config());
        let result = driver
            .chat_completion_stream(
                provider.endpoint(),
                vec![LlmMessage::text(LlmMessageRole::User, "hi")],
                &auth_test_config(),
            )
            .await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("auth failure must abort"),
        };
        assert_eq!(error.llm_error_kind(), Some(LlmErrorKind::Authentication));
        assert!(error.to_string().contains("token refresh refused"));
        assert_eq!(signed.lock().unwrap().len(), fail_on);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), fail_on - 1);
        if fail_on == 2 {
            assert_eq!(requests[0].headers["authorization"], "Bearer token-1");
        }
    }
}

#[test]
fn function_tools_serialize_strict_only_for_compatible_schemas() {
    let mut compatible = make_tool("lookup", None, crate::tool_types::DeferrablePolicy::Never);
    match &mut compatible {
        ToolDefinition::Builtin(tool) => {
            tool.parameters = json!({
                "type": "object", "properties": {"query": {"type": "string"}}
            })
        }
        ToolDefinition::ClientSide(_) => unreachable!(),
    }
    let serialized =
        serde_json::to_value(&OpenResponsesProtocolChatDriver::convert_tools(&[compatible])[0])
            .unwrap();
    assert_eq!(serialized["strict"], true);
    assert_eq!(serialized["parameters"]["required"], json!(["query"]));

    let mut incompatible = make_tool("lookup", None, crate::tool_types::DeferrablePolicy::Never);
    match &mut incompatible {
        ToolDefinition::Builtin(tool) => {
            tool.parameters = json!({
                "type": "object", "allOf": [{"type": "object"}]
            })
        }
        ToolDefinition::ClientSide(_) => unreachable!(),
    }
    let serialized =
        serde_json::to_value(&OpenResponsesProtocolChatDriver::convert_tools(&[incompatible])[0])
            .unwrap();
    assert!(serialized.get("strict").is_none());
    assert!(serialized["parameters"].get("allOf").is_some());
}
#[tokio::test]
async fn compact_request_preserves_endpoint_query_and_complete_contract() {
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::builder().start().await;
    Mock::given(method("POST")).and(path("/v1/responses/compact"))
        .and(query_param("api-version", "preview"))
        .and(header("authorization", "Bearer compact-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "output":[{"type":"message","role":"user","content":"keep me"},{"type":"compaction","encrypted_content":"opaque"}],
            "usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120,"cost":0.03}
        }))).expect(1).mount(&server).await;
    let provider = crate::runtime_provider::RuntimeProvider::new(
        "compact-test",
        OpenResponsesProtocolChatDriver::new(),
    )
    .base_url(format!("{}/v1/responses?api-version=preview", server.uri()))
    .auth(crate::runtime_provider::BearerAuth::new("compact-key"));
    let driver =
        OpenResponsesProtocolChatDriver::new().with_retry_config(LlmRetryConfig::no_retry());
    let result = ChatDriver::compact(
        &driver,
        provider.endpoint(),
        CompactRequest {
            reasoning_state: None,
            model: "model-compact".into(),
            input: vec![CompactInputItem::Message {
                role: "user".into(),
                content: CompactContent::Text("keep me".into()),
            }],
            previous_response_id: None,
            instructions: Some("preserve facts".into()),
        },
    )
    .await
    .unwrap()
    .expect("advertised compact capability must return output");
    assert!(ChatDriver::supports_compact(&driver));
    assert_eq!(
        serde_json::to_value(&result.output).unwrap(),
        json!([
            {"type":"message","role":"user","content":"keep me"},
            {"type":"compaction","encrypted_content":"opaque"}
        ])
    );
    let usage = result.usage.unwrap();
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.total_tokens,
            usage.cost
        ),
        (Some(100), Some(20), Some(120), Some(0.03))
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body,
        json!({"model":"model-compact","input":[{"type":"message","role":"user","content":"keep me"}],"instructions":"preserve facts"})
    );
}
#[test]
fn cache_key_tracks_stable_prefix_and_family_but_not_turn_input() {
    let base = cache_config();
    let instructions = Some("stable system prompt".into());
    let key = |config: &LlmCallConfig,
               instructions: &Option<String>,
               tools: &Option<Vec<ResponsesTool>>,
               input: &[ResponsesInputItem]| {
        OpenResponsesProtocolChatDriver::build_prompt_cache_key(config, input, instructions, tools)
    };
    let expected = key(&base, &instructions, &None, &[]).unwrap();
    assert_eq!(expected.len(), 64);
    assert!(expected.starts_with("everruns:"));
    assert!(expected[9..].bytes().all(|byte| byte.is_ascii_hexdigit()));
    let (_, changed_input) = OpenResponsesProtocolChatDriver::build_input(
        &[LlmMessage::text(LlmMessageRole::User, "different turn")],
        false,
    );
    assert_eq!(
        key(&base, &instructions, &None, &changed_input),
        Some(expected.clone())
    );
    let mut disabled = base.clone();
    disabled.prompt_cache.as_mut().unwrap().enabled = false;
    assert_eq!(key(&disabled, &instructions, &None, &[]), None);
    disabled.prompt_cache = None;
    assert_eq!(key(&disabled, &instructions, &None, &[]), None);
    for field in ["session_id", "model", "instructions", "tools"] {
        let mut config = base.clone();
        let mut prompt = instructions.clone();
        let mut tools = None;
        match field {
            "session_id" => {
                config
                    .metadata
                    .insert("session_id".into(), "session-two".into());
            }
            "model" => config.model = "other-model".into(),
            "instructions" => prompt = Some("different system prompt".into()),
            "tools" => {
                tools = Some(OpenResponsesProtocolChatDriver::convert_tools(&[
                    make_tool("lookup", None, crate::tool_types::DeferrablePolicy::Never),
                ]))
            }
            _ => unreachable!(),
        }
        assert_ne!(
            key(&config, &prompt, &tools, &[]).unwrap(),
            expected,
            "{field}"
        );
    }
    // More specific scopes take precedence; unrelated metadata is not part of the prefix.
    let mut scoped = base.clone();
    scoped.metadata.extend([
        ("agent_id".into(), "agent".into()),
        ("harness_id".into(), "harness".into()),
        ("org_id".into(), "org".into()),
        ("trace_id".into(), "trace".into()),
    ]);
    assert_eq!(
        key(&scoped, &instructions, &None, &[]),
        Some(expected.clone())
    );
    let mut previous = Some(expected);
    for scope in ["session_id", "agent_id", "harness_id", "org_id"] {
        scoped.metadata.remove(scope);
        let current = key(&scoped, &instructions, &None, &[]).unwrap();
        if let Some(previous) = previous {
            assert_ne!(current, previous);
        }
        previous = Some(current);
    }
}
#[test]
fn tool_search_has_complete_stable_wire_order_and_threshold_boundary() {
    let tools = search_tools();
    let expected = expected_search_tools();
    let generated: Vec<_> = (0..32)
        .map(|_| OpenResponsesProtocolChatDriver::convert_tools_with_search(&tools, 6))
        .collect();
    let keys: HashSet<_> = generated
        .iter()
        .map(|tools| {
            OpenResponsesProtocolChatDriver::build_prompt_cache_key(
                &cache_config(),
                &[],
                &None,
                &Some(tools.clone()),
            )
            .unwrap()
        })
        .collect();
    assert_eq!(
        keys.len(),
        1,
        "identical tool sets must produce one cache key"
    );
    for actual in generated {
        assert_eq!(serde_json::to_value(actual).unwrap(), expected);
    }
    let fallback = OpenResponsesProtocolChatDriver::convert_tools_with_search(&tools, 7);
    let expected_fallback: Vec<Value> = ["z", "first", "a", "loose", "second", "b"].into_iter().map(|name| json!({"type":"function","name":name,"description":format!("{name} description"),"parameters":{"type":"object","properties":{},"required":[],"additionalProperties":false},"strict":true})).collect();
    assert_eq!(
        serde_json::to_value(fallback).unwrap(),
        json!(expected_fallback)
    );
    assert_eq!(
        serde_json::to_value(OpenResponsesProtocolChatDriver::convert_tools_with_search(
            &[],
            1
        ))
        .unwrap(),
        json!([])
    );
}

#[tokio::test]
async fn equivalent_search_requests_keep_cache_key_and_complete_tool_payload() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::builder().start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string("data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-cache\",\"status\":\"completed\",\"output\":[]}}\n\n")).expect(2).mount(&server).await;
    let provider = crate::runtime_provider::RuntimeProvider::new(
        "cache-test",
        OpenResponsesProtocolChatDriver::new(),
    )
    .base_url(server.uri());
    let driver = OpenResponsesProtocolChatDriver::new()
        .with_native_features(false, true)
        .with_retry_config(LlmRetryConfig::no_retry());
    let mut config = cache_config();
    config.tools = search_tools();
    config.tool_search = Some(crate::driver_registry::ToolSearchConfig {
        enabled: true,
        threshold: 6,
    });
    for input in ["first turn", "second turn"] {
        let mut stream = driver
            .chat_completion_stream(
                provider.endpoint(),
                vec![
                    LlmMessage::text(LlmMessageRole::System, "stable system prompt"),
                    LlmMessage::text(LlmMessageRole::User, input),
                ],
                &config,
            )
            .await
            .unwrap();
        let mut completions = 0;
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                LlmStreamEvent::Done(metadata) => {
                    assert_eq!(metadata.finish_reason.as_deref(), Some("stop"));
                    completions += 1;
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
        assert_eq!(completions, 1);
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let bodies: Vec<Value> = requests
        .iter()
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect();
    for body in &bodies {
        assert_eq!(body["tools"], expected_search_tools());
        assert_eq!(body["instructions"], "stable system prompt");
        assert_eq!(body["prompt_cache_key"].as_str().unwrap().len(), 64);
    }
    assert_ne!(bodies[0]["input"], bodies[1]["input"]);
    assert_eq!(bodies[0]["prompt_cache_key"], bodies[1]["prompt_cache_key"]);
}

#[test]
fn file_part_serializes_to_input_file() {
    let part = ResponsesContentPart::InputFile {
        r#type: "input_file".to_string(),
        input_file: ResponsesInputFile {
            file_data: Some("data:application/pdf;base64,JVBERi0=".to_string()),
            file_url: None,
            filename: Some("report.pdf".to_string()),
        },
    };
    let v = serde_json::to_value(&part).unwrap();
    assert_eq!(v["type"], serde_json::json!("input_file"));
    assert_eq!(
        v["input_file"]["file_data"],
        serde_json::json!("data:application/pdf;base64,JVBERi0=")
    );
    assert_eq!(v["input_file"]["filename"], serde_json::json!("report.pdf"));
    assert!(v["input_file"].get("file_url").is_none());
}

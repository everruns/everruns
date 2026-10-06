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
use crate::driver_registry::{ChatDriver, LlmCallConfig, LlmStreamEvent, Message, MessageRole};
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
fn test_json_fallback_does_not_recover_incomplete_function_call() {
    let response = json!({
        "output": [{
            "type": "function_call",
            "id": "fc_partial",
            "call_id": "call_partial",
            "name": "bash",
            "arguments": "{}",
            "status": "incomplete"
        }]
    });
    let mut calls = ToolCallStream::default();
    calls.observe_response_json(&response);
    assert!(calls.take_unemitted().is_none());
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
            "output": [{
                "type": "function_call",
                "id": "fc_partial",
                "call_id": "call_partial",
                "name": "bash",
                "arguments": "{\"command\":\"rm -rf",
                "status": "incomplete"
            }],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5,
                "total_tokens": 15
            }
        }
    }"#;

    let event: StreamingEvent = serde_json::from_str(event_json).unwrap();
    let deferred = Mutex::new(Vec::new());
    let stream_event = handle_streaming_event(
        event,
        &Mutex::new(0),
        &Mutex::new(0),
        &Mutex::new(None),
        &Mutex::new(ToolCallStream::default()),
        &Mutex::new(None),
        &deferred,
        "gpt-5.5".to_string(),
        None,
    );

    match stream_event {
        LlmStreamEvent::Done(metadata) => {
            assert_eq!(metadata.finish_reason.as_deref(), Some("length"));
            assert_eq!(
                metadata.provider_finish_reason.as_deref(),
                Some("max_output_tokens")
            );
            // The incomplete item was never handed on, so nothing ran truncated.
            assert_eq!(metadata.tool_calls_truncated_executed, 0);
        }
        other => panic!("expected Done event, got {other:?}"),
    }
    assert!(deferred.lock().unwrap().is_empty());
}

/// A call already handed on mid-stream (via `output_item.done`) before the
/// response ended incomplete may run with cut-off arguments: count it. A
/// completed response counts only calls whose arguments fell back to `{}`.
#[test]
fn test_terminal_event_counts_calls_that_may_run_truncated() {
    for (status, details, arguments, expected) in [
        (
            "incomplete",
            json!({"reason": "max_output_tokens"}),
            "{}",
            1,
        ),
        ("completed", Value::Null, "{\"command\":\"rm -rf", 1),
        ("completed", Value::Null, "{\"command\":\"ls\"}", 0),
        ("completed", Value::Null, "", 0),
    ] {
        let event = json!({
            "type": format!("response.{status}"),
            "sequence_number": 10,
            "response": {
                "id": "resp", "object": "response", "created_at": 1780000000,
                "status": status, "incomplete_details": details, "model": "gpt-5.5",
                "output": [],
                "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
            }
        });
        let mut calls = ToolCallStream::default();
        calls.observe_item("fc_1", "call_1", "bash", arguments);
        calls.mark_complete("fc_1", "call_1");
        let LlmStreamEvent::Done(metadata) = handle_streaming_event(
            serde_json::from_value(event).unwrap(),
            &Mutex::new(0),
            &Mutex::new(0),
            &Mutex::new(None),
            &Mutex::new(calls),
            &Mutex::new(None),
            &Mutex::new(Vec::new()),
            "gpt-5.5".to_string(),
            None,
        ) else {
            panic!("expected Done");
        };
        assert_eq!(
            metadata.tool_calls_truncated_executed, expected,
            "{status} {arguments}"
        );
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
                vec![Message::text(MessageRole::User, "hi")],
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
            vec![Message::text(MessageRole::User, "hi")],
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
                vec![Message::text(MessageRole::User, "hi")],
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
    assert_eq!(serialized["strict"], false);
    assert!(serialized["parameters"].get("allOf").is_some());
}

#[test]
fn client_question_tools_keep_kind_specific_fields_optional() {
    let schema = json!({
        "type": "object",
        "properties": {
            "questions": {
                "type": "array", "minItems": 1, "maxItems": 4,
                "items": {
                    "type": "object",
                    "properties": {
                        "question": {"type": "string"},
                        "secret_name": {"type": "string", "minLength": 1},
                        "purpose": {"type": "string", "minLength": 1}
                    },
                    "required": ["question"],
                    "additionalProperties": false
                }
            },
            "expires_at": {"type": "string", "format": "date-time", "readOnly": true}
        },
        "required": ["questions"],
        "additionalProperties": false
    });
    let tool = ToolDefinition::function("ask_user", "Ask a question", schema.clone());
    for defer_loading in [None, Some(true)] {
        let wire = serde_json::to_value(OpenResponsesProtocolChatDriver::function_tool(
            &tool,
            defer_loading,
        ))
        .unwrap();
        assert_eq!(wire["strict"], false);
        assert_eq!(wire["parameters"], schema);
        assert_eq!(wire["defer_loading"], json!(defer_loading));
    }
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
        &[Message::text(MessageRole::User, "different turn")],
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

/// The model initially sees namespace descriptions, not deferred functions.
/// Generic categories must describe their functions so search can select them.
#[test]
fn tool_search_namespace_description_exposes_deferred_tool_purposes() {
    use crate::tool_types::DeferrablePolicy::{Automatic, Never};
    let tools = vec![
        make_tool("get_current_time", Some("Core"), Automatic),
        make_tool("add", Some("Testing"), Automatic),
        make_tool("subtract", Some("Testing"), Automatic),
        make_tool("hidden_eager", Some("Testing"), Never),
    ];
    let wire = serde_json::to_value(OpenResponsesProtocolChatDriver::convert_tools_with_search(
        &tools, 1,
    ))
    .unwrap();
    let namespaces: Vec<_> = wire
        .as_array()
        .unwrap()
        .iter()
        .filter(|tool| tool["type"] == "namespace")
        .collect();
    let core = namespaces[0]["description"].as_str().unwrap();
    assert!(core.contains("get_current_time description"));
    let testing = namespaces[1]["description"].as_str().unwrap();
    assert!(testing.contains("add description"));
    assert!(testing.contains("subtract description"));
    assert!(!testing.contains("hidden_eager"));
    for namespace in namespaces {
        for function in namespace["tools"].as_array().unwrap() {
            assert_eq!(function["defer_loading"], true);
        }
        assert!(
            !namespace["description"]
                .as_str()
                .unwrap()
                .contains("properties")
        );
    }
}

#[test]
fn tool_search_namespace_description_bounds_unicode_purposes_without_dropping_tools() {
    use crate::tool_types::DeferrablePolicy::Automatic;
    let tools: Vec<_> = (0..100)
        .map(|index| {
            let mut tool = make_tool(&format!("lookup_{index}"), Some("Core"), Automatic);
            let ToolDefinition::Builtin(ref mut definition) = tool else {
                unreachable!()
            };
            definition.description = "目的".repeat(1000);
            tool
        })
        .collect();
    let wire = serde_json::to_value(OpenResponsesProtocolChatDriver::convert_tools_with_search(
        &tools, 1,
    ))
    .unwrap();
    let description = wire[0]["description"].as_str().unwrap();
    assert!(description.chars().count() <= 4096);
    assert!(description.contains("lookup_0: 目的"));
    assert_eq!(wire[0]["tools"].as_array().unwrap().len(), 100);
}

/// EVE-1164: OpenAI hosted tool search fails with `server_error` when a
/// deferred namespace is named `File Operations` (whitespace). The wire name
/// must be a provider-safe identifier; the description keeps the human label.
#[test]
fn tool_search_normalizes_file_operations_namespace_for_openai() {
    use crate::tool_types::DeferrablePolicy::Automatic;

    let tools = vec![make_tool(
        "list_directory",
        Some("File Operations"),
        Automatic,
    )];
    let wire = serde_json::to_value(OpenResponsesProtocolChatDriver::convert_tools_with_search(
        &tools, 1,
    ))
    .unwrap();

    assert_eq!(
        wire,
        json!([
            {
                "type": "namespace",
                "name": "File_Operations",
                "description": "Tools for File Operations: list_directory: list_directory description",
                "tools": [{
                    "type": "function",
                    "name": "list_directory",
                    "description": "list_directory description",
                    "parameters": {
                        "type": "object",
                        "properties": {},
                        "required": [],
                        "additionalProperties": false
                    },
                    "strict": true,
                    "defer_loading": true
                }]
            },
            {"type": "tool_search"}
        ])
    );
}

#[test]
fn tool_search_namespace_normalization_covers_safe_names_punctuation_unicode_and_collisions() {
    use crate::tool_types::DeferrablePolicy::Automatic;

    assert_eq!(
        normalize_tool_search_namespace_name("Alpha"),
        "Alpha",
        "already-safe names stay unchanged"
    );
    assert_eq!(
        normalize_tool_search_namespace_name("File Operations"),
        "File_Operations"
    );
    assert_eq!(
        normalize_tool_search_namespace_name("MCP Servers / GitHub"),
        "MCP_Servers_GitHub"
    );
    assert_eq!(
        normalize_tool_search_namespace_name("  spaced--name!!  "),
        "spaced_name"
    );
    assert_eq!(
        normalize_tool_search_namespace_name("123start"),
        "n_123start",
        "leading digits need an alphabetic prefix"
    );
    assert_eq!(
        normalize_tool_search_namespace_name("文件操作"),
        "namespace",
        "Unicode-only labels fall back instead of emitting an empty name"
    );
    assert_eq!(normalize_tool_search_namespace_name(""), "namespace");
    assert_eq!(normalize_tool_search_namespace_name("!!!"), "namespace");

    // Distinct categories that normalize to the same identifier must not merge.
    let tools = vec![
        make_tool("a", Some("File Operations"), Automatic),
        make_tool("b", Some("File_Operations"), Automatic),
        make_tool("c", Some("File-Operations"), Automatic),
    ];
    let generated: Vec<_> = (0..16)
        .map(|_| {
            serde_json::to_value(OpenResponsesProtocolChatDriver::convert_tools_with_search(
                &tools, 1,
            ))
            .unwrap()
        })
        .collect();
    let first = &generated[0];
    for actual in &generated[1..] {
        assert_eq!(
            actual, first,
            "normalized namespace assignment must be stable across builds"
        );
    }

    let namespaces: Vec<&str> = first
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "namespace")
        .map(|item| item["name"].as_str().unwrap())
        .collect();
    // BTreeMap category order: "File Operations" < "File-Operations" < "File_Operations".
    assert_eq!(
        namespaces,
        vec!["File_Operations", "File_Operations_2", "File_Operations_3"],
        "colliding categories keep separate provider namespaces in category order"
    );

    let descriptions: Vec<&str> = first
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "namespace")
        .map(|item| item["description"].as_str().unwrap())
        .collect();
    assert_eq!(
        descriptions,
        vec![
            "Tools for File Operations: a: a description",
            "Tools for File-Operations: c: c description",
            "Tools for File_Operations: b: b description",
        ],
        "descriptions keep the original category labels"
    );

    // Each namespace still carries only its own functions.
    let functions: Vec<Vec<&str>> = first
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "namespace")
        .map(|item| {
            item["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tool| tool["name"].as_str().unwrap())
                .collect()
        })
        .collect();
    assert_eq!(functions, vec![vec!["a"], vec!["c"], vec!["b"]]);
}

#[tokio::test]
async fn file_operations_namespace_request_and_namespaced_call_dispatch() {
    use crate::tool_types::DeferrablePolicy::Automatic;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::builder().start().await;
    let sse = concat!(
        "data: {\"type\":\"response.output_item.done\",\"item\":{",
        "\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",",
        "\"name\":\"list_directory\",\"namespace\":\"File_Operations\",",
        "\"arguments\":\"{\\\"path\\\":\\\".\\\"}\",\"status\":\"completed\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{",
        "\"id\":\"resp_ns\",\"status\":\"completed\",\"output\":[",
        "{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",",
        "\"name\":\"list_directory\",\"namespace\":\"File_Operations\",",
        "\"arguments\":\"{\\\"path\\\":\\\".\\\"}\",\"status\":\"completed\"}",
        "]}}\n\n"
    );
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
        "ns-normalize",
        OpenResponsesProtocolChatDriver::new(),
    )
    .base_url(server.uri());
    let driver = OpenResponsesProtocolChatDriver::new()
        .with_native_features(false, true)
        .with_retry_config(LlmRetryConfig::no_retry());
    let mut config = auth_test_config();
    config.tools = vec![make_tool(
        "list_directory",
        Some("File Operations"),
        Automatic,
    )];
    config.tool_search = Some(crate::driver_registry::ToolSearchConfig {
        enabled: true,
        threshold: 1,
    });

    let mut stream = driver
        .chat_completion_stream(
            provider.endpoint(),
            vec![Message::text(
                MessageRole::User,
                "List the current directory.",
            )],
            &config,
        )
        .await
        .unwrap();

    let mut saw_list_directory = false;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            LlmStreamEvent::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "list_directory");
                assert_eq!(calls[0].arguments, json!({"path": "."}));
                saw_list_directory = true;
            }
            LlmStreamEvent::Done(metadata) => {
                assert_eq!(metadata.finish_reason.as_deref(), Some("tool_calls"));
            }
            LlmStreamEvent::TextDelta(delta) if delta.is_empty() => {}
            other => panic!("unexpected event: {other:?}"),
        }
    }
    assert!(
        saw_list_directory,
        "namespaced function_call must dispatch by function name"
    );

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["tools"],
        json!([
            {
                "type": "namespace",
                "name": "File_Operations",
                "description": "Tools for File Operations: list_directory: list_directory description",
                "tools": [{
                    "type": "function",
                    "name": "list_directory",
                    "description": "list_directory description",
                    "parameters": {
                        "type": "object",
                        "properties": {},
                        "required": [],
                        "additionalProperties": false
                    },
                    "strict": true,
                    "defer_loading": true
                }]
            },
            {"type": "tool_search"}
        ])
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
                    Message::text(MessageRole::System, "stable system prompt"),
                    Message::text(MessageRole::User, input),
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

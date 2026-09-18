//! Tests for the OpenResponses protocol: request.

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
    ChatDriver, LlmCallConfig, LlmMessage, LlmMessageContent, LlmMessageRole,
};

use super::*;

use super::tests_support::*;

#[test]
fn explicit_cache_wire_options_are_model_gated() {
    let mut config = LlmCallConfig {
        reasoning_state: None,
        speed: None,
        verbosity: None,
        model: "gpt-6-astra".to_string(),
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
    };
    config.prompt_cache = Some(crate::driver_registry::PromptCacheConfig {
        enabled: true,
        strategy: crate::driver_registry::PromptCacheStrategy::Explicit,
        ..Default::default()
    });
    let original = json!({"instructions": "Stable policy", "input": [{"role": "user", "content": "Changing question"}]});
    let mut body = original.clone();
    apply_cache_options(&mut body, &config, true);
    assert_eq!(
        body["prompt_cache_options"],
        json!({"ttl":"30m","mode":"explicit"})
    );
    assert!(body.get("instructions").is_none());
    assert_eq!(
        body["input"][0]["content"][0],
        json!({"type":"input_text","text":"Stable policy","prompt_cache_breakpoint":{"mode":"explicit"}})
    );
    assert_eq!(body["input"][1], original["input"][0]);
    let mut gateway = original.clone();
    apply_cache_options(&mut gateway, &config, false);
    assert_eq!(gateway, original);
    config.model = "gpt-5.5".into();
    let mut older = original.clone();
    apply_cache_options(&mut older, &config, true);
    assert_eq!(older, original);
    config.model = "gpt-5.6-sol".into();
    config.prompt_cache.as_mut().unwrap().strategy =
        crate::driver_registry::PromptCacheStrategy::Auto;
    let mut implicit = original.clone();
    apply_cache_options(&mut implicit, &config, true);
    assert_eq!(implicit["instructions"], original["instructions"]);
    assert_eq!(implicit["prompt_cache_options"]["mode"], "implicit");
}

#[test]
fn test_request_serialization() {
    let request = ResponsesRequest {
        include: None,
        text: None,
        service_tier: None,
        model: "gpt-5.2".to_string(),
        input: vec![ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("Hello".to_string()),
            phase: None,
        }],
        instructions: Some("You are helpful".to_string()),
        previous_response_id: None,
        temperature: None,
        max_output_tokens: None,
        stream: true,
        tools: None,
        reasoning: None,
        metadata: None,
        prompt_cache_key: None,
        parallel_tool_calls: None,
    };

    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["model"], "gpt-5.2");
    assert_eq!(json["stream"], true);
    assert_eq!(json["instructions"], "You are helpful");
    assert!(json["input"].is_array());
}

#[test]
fn test_request_with_reasoning() {
    let request = ResponsesRequest {
        include: None,
        text: None,
        service_tier: None,
        model: "o3".to_string(),
        input: vec![ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("Think about this".to_string()),
            phase: None,
        }],
        instructions: None,
        previous_response_id: None,
        temperature: None,
        max_output_tokens: None,
        stream: true,
        tools: None,
        reasoning: Some(ResponsesReasoning {
            effort: "high".to_string(),
            summary: "detailed".to_string(),
        }),
        metadata: None,
        prompt_cache_key: None,
        parallel_tool_calls: None,
    };

    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["reasoning"]["effort"], "high");
    assert_eq!(json["reasoning"]["summary"], "detailed");
}

#[test]
fn test_request_with_metadata() {
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("session_id".to_string(), "session_abc123".to_string());
    metadata.insert("agent_id".to_string(), "agent_xyz789".to_string());

    let request = ResponsesRequest {
        include: None,
        text: None,
        service_tier: None,
        model: "gpt-5.2".to_string(),
        input: vec![ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("Hello".to_string()),
            phase: None,
        }],
        instructions: None,
        previous_response_id: None,
        temperature: None,
        max_output_tokens: None,
        stream: true,
        tools: None,
        reasoning: None,
        metadata: Some(metadata),
        prompt_cache_key: None,
        parallel_tool_calls: None,
    };

    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["metadata"]["session_id"], "session_abc123");
    assert_eq!(json["metadata"]["agent_id"], "agent_xyz789");
}

/// EVE-598: the Responses request serializes `parallel_tool_calls` only when
/// the config sets it, preserving provider defaults when `None`.
#[test]
fn test_request_serializes_parallel_tool_calls() {
    let make = |flag: Option<bool>| ResponsesRequest {
        include: None,
        text: None,
        service_tier: None,
        model: "gpt-5.4".to_string(),
        input: vec![ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("Hello".to_string()),
            phase: None,
        }],
        instructions: None,
        previous_response_id: None,
        temperature: None,
        max_output_tokens: None,
        stream: true,
        tools: None,
        reasoning: None,
        metadata: None,
        prompt_cache_key: None,
        parallel_tool_calls: flag,
    };

    // None → field omitted entirely (provider default preserved).
    let json = serde_json::to_value(make(None)).unwrap();
    assert!(json.get("parallel_tool_calls").is_none());

    // Some(true) → field present and true.
    let json = serde_json::to_value(make(Some(true))).unwrap();
    assert_eq!(json["parallel_tool_calls"], true);

    // Some(false) → field present and false.
    let json = serde_json::to_value(make(Some(false))).unwrap();
    assert_eq!(json["parallel_tool_calls"], false);
}

/// The speed selector serializes as `service_tier` only when set,
/// preserving the provider's default ("auto") routing when `None`.
#[test]
fn test_request_serializes_service_tier() {
    let make = |tier: Option<&str>| ResponsesRequest {
        service_tier: tier.map(str::to_string),
        model: "gpt-5.4".to_string(),
        input: vec![ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("Hello".to_string()),
            phase: None,
        }],
        instructions: None,
        previous_response_id: None,
        temperature: None,
        max_output_tokens: None,
        stream: true,
        tools: None,
        reasoning: None,
        metadata: None,
        prompt_cache_key: None,
        parallel_tool_calls: None,
        text: None,
        include: None,
    };

    let json = serde_json::to_value(make(None)).unwrap();
    assert!(json.get("service_tier").is_none());

    let json = serde_json::to_value(make(Some("priority"))).unwrap();
    assert_eq!(json["service_tier"], "priority");

    let json = serde_json::to_value(make(Some("flex"))).unwrap();
    assert_eq!(json["service_tier"], "flex");
}

/// Verbosity serializes as a nested `text.verbosity` object only when set,
/// preserving the provider's default output length when `None`.
#[test]
fn test_request_serializes_verbosity() {
    let make = |verbosity: Option<&str>| ResponsesRequest {
        include: None,
        service_tier: None,
        text: verbosity.map(|v| ResponsesText {
            verbosity: Some(v.to_string()),
        }),
        model: "gpt-5.6-sol".to_string(),
        input: vec![ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("Hello".to_string()),
            phase: None,
        }],
        instructions: None,
        previous_response_id: None,
        temperature: None,
        max_output_tokens: None,
        stream: true,
        tools: None,
        reasoning: None,
        metadata: None,
        prompt_cache_key: None,
        parallel_tool_calls: None,
    };

    let json = serde_json::to_value(make(None)).unwrap();
    assert!(json.get("text").is_none());

    let json = serde_json::to_value(make(Some("low"))).unwrap();
    assert_eq!(json["text"]["verbosity"], "low");

    let json = serde_json::to_value(make(Some("high"))).unwrap();
    assert_eq!(json["text"]["verbosity"], "high");
}

#[test]
fn test_function_call_output_serialization() {
    let item = ResponsesInputItem::FunctionCallOutput {
        r#type: "function_call_output".to_string(),
        call_id: "call_123".to_string(),
        output: r#"{"result": 42}"#.to_string(),
    };

    let json = serde_json::to_value(&item).unwrap();
    assert_eq!(json["type"], "function_call_output");
    assert_eq!(json["call_id"], "call_123");
    assert_eq!(json["output"], r#"{"result": 42}"#);
}

#[test]
fn test_multipart_content_serialization() {
    let content = ResponsesContent::Parts(vec![
        ResponsesContentPart::InputText {
            r#type: "input_text".to_string(),
            text: "Look at this image".to_string(),
        },
        ResponsesContentPart::InputImage {
            r#type: "input_image".to_string(),
            image_url: "data:image/png;base64,abc123".to_string(),
        },
    ]);

    let json = serde_json::to_value(&content).unwrap();
    assert!(json.is_array());
    assert_eq!(json[0]["type"], "input_text");
    assert_eq!(json[1]["type"], "input_image");
}

#[test]
fn test_tool_serialization() {
    let tool = ResponsesTool::Function {
        r#type: "function".to_string(),
        name: "get_weather".to_string(),
        description: "Get weather for a location".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "location": {"type": "string"}
            },
            "required": ["location"]
        }),
        strict: Some(true),
        defer_loading: None,
    };

    let json = serde_json::to_value(&tool).unwrap();
    assert_eq!(json["type"], "function");
    assert_eq!(json["name"], "get_weather");
    assert!(json["parameters"]["properties"]["location"].is_object());
}

#[test]
fn test_build_input_extracts_system_as_instructions() {
    let messages = vec![
        LlmMessage::text(LlmMessageRole::System, "You are a helpful assistant"),
        LlmMessage::text(LlmMessageRole::User, "Hello"),
    ];

    let (instructions, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

    assert_eq!(
        instructions,
        Some("You are a helpful assistant".to_string())
    );
    assert_eq!(input.len(), 1); // Only user message, system converted to instructions
}

#[test]
fn test_build_input_concatenates_multiple_system_messages() {
    // The agent system prompt plus a later system message (e.g. infinity
    // context's hidden-history notice or compaction's summary) must both
    // survive — the later one must not overwrite the real system prompt.
    let messages = vec![
        LlmMessage::text(LlmMessageRole::System, "You are a helpful assistant"),
        LlmMessage::text(LlmMessageRole::User, "Hello"),
        LlmMessage::text(
            LlmMessageRole::System,
            "[IMPORTANT: 3 earlier messages are NOT visible in this context.]",
        ),
    ];

    let (instructions, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

    assert_eq!(
        instructions,
        Some(
            "You are a helpful assistant\n\n[IMPORTANT: 3 earlier messages are NOT visible in this context.]"
                .to_string()
        )
    );
    assert_eq!(input.len(), 1); // Only the user message remains as input
}

#[test]
fn test_convert_role() {
    assert_eq!(
        OpenResponsesProtocolChatDriver::convert_role(&LlmMessageRole::System),
        "developer"
    );
    assert_eq!(
        OpenResponsesProtocolChatDriver::convert_role(&LlmMessageRole::User),
        "user"
    );
    assert_eq!(
        OpenResponsesProtocolChatDriver::convert_role(&LlmMessageRole::Assistant),
        "assistant"
    );
    assert_eq!(
        OpenResponsesProtocolChatDriver::convert_role(&LlmMessageRole::Tool),
        "tool"
    );
}

#[test]
fn test_function_call_serialization() {
    let item = ResponsesInputItem::FunctionCall {
        r#type: "function_call".to_string(),
        call_id: "call_abc123".to_string(),
        name: "get_current_time".to_string(),
        arguments: r#"{"timezone":"UTC"}"#.to_string(),
    };

    let json = serde_json::to_value(&item).unwrap();
    assert_eq!(json["type"], "function_call");
    assert_eq!(json["call_id"], "call_abc123");
    assert_eq!(json["name"], "get_current_time");
    assert_eq!(json["arguments"], r#"{"timezone":"UTC"}"#);
}

#[test]
fn test_build_input_with_tool_calls() {
    use crate::tool_types::ToolCall;

    // Simulate a conversation with tool calls:
    // 1. User asks a question
    // 2. Assistant calls a tool
    // 3. Tool returns result
    let messages = vec![
        LlmMessage::text(LlmMessageRole::System, "You are helpful"),
        LlmMessage::text(LlmMessageRole::User, "What time is it?"),
        LlmMessage {
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text(String::new()),
            tool_calls: Some(vec![ToolCall {
                id: "call_xyz789".to_string(),
                name: "get_current_time".to_string(),
                arguments: json!({"timezone": "UTC"}),
            }]),
            tool_call_id: None,
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
            native_tool_calls: Vec::new(),
        },
        LlmMessage {
            role: LlmMessageRole::Tool,
            content: LlmMessageContent::Text("2025-01-19T10:30:00Z".to_string()),
            tool_calls: None,
            tool_call_id: Some("call_xyz789".to_string()),
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
            native_tool_calls: Vec::new(),
        },
    ];

    let (instructions, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

    // System message becomes instructions
    assert_eq!(instructions, Some("You are helpful".to_string()));

    // Should have: user message, function_call, function_call_output
    assert_eq!(input.len(), 3);

    // Verify the function_call is present (second item, since assistant had empty content)
    let json = serde_json::to_value(&input[1]).unwrap();
    assert_eq!(json["type"], "function_call");
    assert_eq!(json["call_id"], "call_xyz789");
    assert_eq!(json["name"], "get_current_time");

    // Verify the function_call_output is present
    let json = serde_json::to_value(&input[2]).unwrap();
    assert_eq!(json["type"], "function_call_output");
    assert_eq!(json["call_id"], "call_xyz789");
    assert_eq!(json["output"], "2025-01-19T10:30:00Z");
}

#[test]
fn test_build_input_with_tool_calls_and_text() {
    use crate::tool_types::ToolCall;

    // Assistant message with both text content and tool calls
    let messages = vec![
        LlmMessage::text(LlmMessageRole::User, "What time is it?"),
        LlmMessage {
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text("Let me check the time for you.".to_string()),
            tool_calls: Some(vec![ToolCall {
                id: "call_abc".to_string(),
                name: "get_time".to_string(),
                arguments: json!({}),
            }]),
            tool_call_id: None,
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
            native_tool_calls: Vec::new(),
        },
    ];

    let (_, input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

    // Should have: user message, assistant message, function_call
    assert_eq!(input.len(), 3);

    // First is user message
    let json = serde_json::to_value(&input[0]).unwrap();
    assert_eq!(json["role"], "user");

    // Second is assistant message with text
    let json = serde_json::to_value(&input[1]).unwrap();
    assert_eq!(json["role"], "assistant");

    // Third is function_call
    let json = serde_json::to_value(&input[2]).unwrap();
    assert_eq!(json["type"], "function_call");
    assert_eq!(json["call_id"], "call_abc");
}

// ========================================================================
// EVE-488: Stateful Responses continuations must not double-send context.
//
// When `previous_response_id` is set, the OpenAI Responses provider already
// holds the prior transcript server-side. Re-sending it in `input` causes
// double-counting. These tests pin the invariant that the delta-trim helper
// only keeps items strictly after the most recent assistant turn, and
// that the request-building path applies the trim when (and only when) a
// continuation handle is present.
// ========================================================================

/// Issue reproducer: a stateful continuation must not carry the full prior
/// transcript in `input` alongside `previous_response_id`. After trimming,
/// only the new tool result and any fresh user input should remain.
#[test]
fn openresponses_requests_should_not_mix_previous_response_id_with_full_transcript() {
    use crate::tool_types::ToolCall;

    // Simulate a multi-turn transcript: system + user + assistant(tool_call) + tool result.
    // This is the exact shape that gets reconstructed on a follow-up turn when
    // the runtime has a `previous_response_id` from the prior assistant turn.
    let messages = vec![
        LlmMessage::text(LlmMessageRole::System, "You are helpful"),
        LlmMessage::text(LlmMessageRole::User, "What time is it?"),
        LlmMessage {
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text("Let me check.".to_string()),
            tool_calls: Some(vec![ToolCall {
                id: "call_xyz789".to_string(),
                name: "get_current_time".to_string(),
                arguments: json!({"timezone": "UTC"}),
            }]),
            tool_call_id: None,
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
            native_tool_calls: Vec::new(),
        },
        LlmMessage {
            role: LlmMessageRole::Tool,
            content: LlmMessageContent::Text("2025-01-19T10:30:00Z".to_string()),
            tool_calls: None,
            tool_call_id: Some("call_xyz789".to_string()),
            phase: None,
            reasoning: Vec::new(),
            configuration_update: None,
            native_tool_calls: Vec::new(),
        },
    ];

    // Build the full transcript the same way the driver does.
    let (instructions, full_input) = OpenResponsesProtocolChatDriver::build_input(&messages, false);

    // Without trimming the full transcript leaks user + assistant + function_call
    // + function_call_output — exactly the bug.
    assert!(
        full_input.len() > 1,
        "sanity: full transcript has multi items"
    );

    // The trim performed when `previous_response_id` is present in the request
    // path must drop everything up to and including the last prior-assistant item.
    let delta = compute_delta_input_items(full_input);

    // Only the tool result (function_call_output) should remain.
    assert_eq!(
        delta.len(),
        1,
        "stateful continuation must only send delta items; got {} items",
        delta.len()
    );
    let json = serde_json::to_value(&delta[0]).unwrap();
    assert_eq!(json["type"], "function_call_output");
    assert_eq!(json["call_id"], "call_xyz789");
    assert_eq!(json["output"], "2025-01-19T10:30:00Z");

    // Instructions (system message) are NOT part of `input`; they're still sent
    // separately and that is correct — they don't count toward the invariant.
    assert_eq!(instructions, Some("You are helpful".to_string()));
}

/// Stateless mode (no previous_response_id): all input items are kept.
/// The trim helper is only invoked by the call path when previous_response_id
/// is set; this test pins that the helper produces correct delta output
/// regardless, leaving the fresh user message that follows the assistant turn.
#[test]
fn compute_delta_keeps_tail_after_assistant_message() {
    let items = vec![
        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("hi".to_string()),
            phase: None,
        },
        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "assistant".to_string(),
            content: ResponsesContent::Text("hello".to_string()),
            phase: None,
        },
        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("follow up".to_string()),
            phase: None,
        },
    ];
    let trimmed = compute_delta_input_items(items);
    assert_eq!(trimmed.len(), 1);
    let json = serde_json::to_value(&trimmed[0]).unwrap();
    assert_eq!(json["role"], "user");
    assert_eq!(
        json["content"], "follow up",
        "trim keeps the fresh user message that arrived after the assistant turn"
    );
}

/// Stateful continuation with parallel tool calls: every tool output that
/// follows the prior assistant's function_call items is kept. The function_call
/// items themselves belong to server-side state and are dropped.
#[test]
fn compute_delta_keeps_tool_results_after_last_assistant_turn() {
    let items = vec![
        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("do two things".to_string()),
            phase: None,
        },
        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "assistant".to_string(),
            content: ResponsesContent::Text("ok".to_string()),
            phase: None,
        },
        ResponsesInputItem::FunctionCall {
            r#type: "function_call".to_string(),
            call_id: "call_a".to_string(),
            name: "tool_a".to_string(),
            arguments: "{}".to_string(),
        },
        ResponsesInputItem::FunctionCall {
            r#type: "function_call".to_string(),
            call_id: "call_b".to_string(),
            name: "tool_b".to_string(),
            arguments: "{}".to_string(),
        },
        ResponsesInputItem::FunctionCallOutput {
            r#type: "function_call_output".to_string(),
            call_id: "call_a".to_string(),
            output: "a result".to_string(),
        },
        ResponsesInputItem::FunctionCallOutput {
            r#type: "function_call_output".to_string(),
            call_id: "call_b".to_string(),
            output: "b result".to_string(),
        },
    ];

    let trimmed = compute_delta_input_items(items);

    // The function_call items live in server-side state. The delta carries
    // only the tool outputs the client produced for them.
    assert_eq!(trimmed.len(), 2);
    for item in &trimmed {
        let json = serde_json::to_value(item).unwrap();
        assert_eq!(json["type"], "function_call_output");
    }
}

/// Empty input with previous_response_id is valid: the provider can resume
/// purely from the continuation handle, no input needed.
#[test]
fn compute_delta_allows_empty_input_for_stateful_continuation() {
    let trimmed = compute_delta_input_items(vec![]);
    assert!(trimmed.is_empty());
}

/// Defensive: if no prior-assistant item is present (caller passed only fresh
/// user input), all items are kept as delta.
#[test]
fn compute_delta_keeps_all_items_when_no_assistant_turn_present() {
    let items = vec![
        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("one".to_string()),
            phase: None,
        },
        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("two".to_string()),
            phase: None,
        },
    ];
    let trimmed = compute_delta_input_items(items);
    assert_eq!(trimmed.len(), 2);
}

/// Reasoning items from a prior assistant turn are also dropped by the trim.
#[test]
fn compute_delta_drops_prior_reasoning_items() {
    let items = vec![
        ResponsesInputItem::Reasoning {
            r#type: "reasoning".to_string(),
            id: "rs_00000001".to_string(),
            encrypted_content: "encrypted-blob".to_string(),
            summary: Vec::new(),
        },
        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "assistant".to_string(),
            content: ResponsesContent::Text("prior".to_string()),
            phase: None,
        },
        ResponsesInputItem::FunctionCallOutput {
            r#type: "function_call_output".to_string(),
            call_id: "call_z".to_string(),
            output: "result".to_string(),
        },
    ];
    let trimmed = compute_delta_input_items(items);
    assert_eq!(trimmed.len(), 1);
    let json = serde_json::to_value(&trimmed[0]).unwrap();
    assert_eq!(json["type"], "function_call_output");
}

#[test]
fn finalize_input_skips_trim_when_previous_response_id_is_none() {
    let items = sample_full_transcript_items();
    let original_len = items.len();
    let out = finalize_input_for_request(items, &None);
    assert_eq!(
        out.len(),
        original_len,
        "stateless mode keeps the full transcript so the model has context"
    );
}

#[test]
fn finalize_input_drops_locally_orphaned_tool_output_without_previous_response_id() {
    let items = vec![
        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("fresh".to_string()),
            phase: None,
        },
        ResponsesInputItem::FunctionCallOutput {
            r#type: "function_call_output".to_string(),
            call_id: "call_trimmed".to_string(),
            output: "result".to_string(),
        },
    ];

    let out = finalize_input_for_request(items, &None);

    assert_eq!(out.len(), 1);
    let json = serde_json::to_value(&out[0]).unwrap();
    assert_eq!(json["type"], "message");
}

#[test]
fn finalize_input_keeps_tool_output_with_previous_response_id_even_without_local_call() {
    let items = vec![
        ResponsesInputItem::FunctionCallOutput {
            r#type: "function_call_output".to_string(),
            call_id: "call_server_side".to_string(),
            output: "stateful result".to_string(),
        },
        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("follow-up".to_string()),
            phase: None,
        },
    ];

    let out = finalize_input_for_request(items, &Some("resp_prev_42".to_string()));

    assert_eq!(out.len(), 2);
    let json = serde_json::to_value(&out[0]).unwrap();
    assert_eq!(json["type"], "function_call_output");
    assert_eq!(json["call_id"], "call_server_side");
}

#[test]
fn finalize_input_trims_when_previous_response_id_is_set() {
    let items = sample_full_transcript_items();
    let out = finalize_input_for_request(items, &Some("resp_prev_42".to_string()));
    assert_eq!(
        out.len(),
        1,
        "stateful continuation must drop everything up to and including the prior assistant message"
    );
    let json = serde_json::to_value(&out[0]).unwrap();
    assert_eq!(json["type"], "message");
    assert_eq!(json["role"], "user");
    // Only the post-assistant follow-up survives.
    let txt = json["content"].as_str().unwrap_or("");
    assert_eq!(txt, "follow-up");
}

#[test]
fn finalize_input_allows_empty_input_with_previous_response_id() {
    let out = finalize_input_for_request(vec![], &Some("resp_anything".to_string()));
    assert!(
        out.is_empty(),
        "empty delta is valid — the provider can resume purely from the response id"
    );
}

#[test]
fn finalize_input_drops_dangling_function_call_without_previous_response_id() {
    // The exact incident: an early `read_file` call survived compaction but
    // its tool output was evicted (keep_recent_tool_outputs), leaving a
    // dangling `function_call`.
    let items = vec![
        user_message("fresh"),
        function_call("call_pHJNxIuwzLppFsQK5nJrDOpZ", "read_file"),
    ];

    let out = finalize_input_for_request(items, &None);

    assert_eq!(out.len(), 1);
    assert!(
        unpaired_function_call_ids(&out).is_empty(),
        "the dangling function_call must be dropped"
    );
    let json = serde_json::to_value(&out[0]).unwrap();
    assert_eq!(json["type"], "message");
}

#[test]
fn finalize_input_preserves_paired_function_call_and_output() {
    let items = vec![
        user_message("what time is it?"),
        function_call("call_ok", "get_current_time"),
        function_call_output("call_ok"),
    ];

    let out = finalize_input_for_request(items, &None);

    assert_eq!(out.len(), 3, "an intact call/output pair must survive");
    assert!(unpaired_function_call_ids(&out).is_empty());
}

#[test]
fn finalize_input_compaction_drops_only_the_dangling_old_call() {
    // Post-compaction model view equivalent to keep_recent_tool_outputs = 3:
    // one old call whose output was masked away, followed by three intact
    // recent pairs. Only the dangling old call is dropped; the recent pairs
    // and the surrounding messages are preserved.
    let mut items = vec![
        user_message("long session"),
        function_call("call_old", "read_file"),
    ];
    for i in 0..3 {
        let id = format!("call_recent_{i}");
        items.push(function_call(&id, "tool"));
        items.push(function_call_output(&id));
    }

    let out = finalize_input_for_request(items, &None);

    assert!(
        unpaired_function_call_ids(&out).is_empty(),
        "no dangling function_call may remain after repair"
    );
    assert!(
        !out.iter().any(|item| matches!(
            item,
            ResponsesInputItem::FunctionCall { call_id, .. } if call_id == "call_old"
        )),
        "the old dangling call must be removed"
    );
    // 1 user message + 3 intact recent pairs (6 items) = 7.
    assert_eq!(out.len(), 7);
}

#[test]
fn unpaired_function_call_ids_reports_both_directions() {
    let items = vec![
        function_call("call_no_output", "read_file"), // EVE-597: dangling call
        function_call_output("out_no_call"),          // EVE-519: orphan output
        function_call("paired", "tool"),
        function_call_output("paired"),
    ];

    let mut ids = unpaired_function_call_ids(&items);
    ids.sort();
    assert_eq!(
        ids,
        vec!["call_no_output".to_string(), "out_no_call".to_string()]
    );
}

// ========================================================================
// Provider-declared statefulness (EVE-523)
// ========================================================================

#[test]
fn provider_can_enable_stateful_responses() {
    assert!(
        OpenResponsesProtocolChatDriver::new()
            .with_stateful_responses(true)
            .supports_stateful_responses()
    );
}

#[test]
fn wire_protocol_defaults_to_stateless() {
    assert!(!OpenResponsesProtocolChatDriver::new().supports_stateful_responses());
}

/// End-to-end shape of the call path: against a stateless gateway, a request
/// that carries a `previous_response_id` in config must still send the FULL
/// transcript in `input` (no trim) because the gateway will not have stored
/// the prior response. This is the core EVE-523 regression guard.
#[test]
fn stateless_gateway_replays_full_transcript_despite_previous_response_id() {
    let prev_id: Option<String> = Some("gen-turn-1".to_string());

    let driver = OpenResponsesProtocolChatDriver::new();
    let effective_prev_id = if driver.supports_stateful_responses() {
        prev_id.clone()
    } else {
        None
    };
    assert!(
        effective_prev_id.is_none(),
        "stateless gateway must not chain via previous_response_id"
    );

    let items = sample_full_transcript_items();
    let original_len = items.len();
    let out = finalize_input_for_request(items, &effective_prev_id);
    assert_eq!(
        out.len(),
        original_len,
        "stateless gateway must replay the full transcript so the model keeps context"
    );
}

/// The same transcript against OpenAI's hosted API trims to the delta window
/// and keeps the continuation handle — confirming the optimization is intact
/// for genuinely stateful endpoints.
#[test]
fn stateful_endpoint_still_trims_and_chains() {
    let prev_id: Option<String> = Some("resp_turn_1".to_string());

    let driver = OpenResponsesProtocolChatDriver::new().with_stateful_responses(true);
    let effective_prev_id = if driver.supports_stateful_responses() {
        prev_id.clone()
    } else {
        None
    };
    assert_eq!(
        effective_prev_id, prev_id,
        "stateful endpoint keeps the continuation handle"
    );

    let out = finalize_input_for_request(sample_full_transcript_items(), &effective_prev_id);
    assert_eq!(out.len(), 1, "stateful endpoint trims to the delta window");
}

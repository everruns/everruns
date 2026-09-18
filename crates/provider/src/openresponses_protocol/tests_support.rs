//! Fixtures shared by the protocol test modules.

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
use reqwest::header::HeaderMap;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

use crate::driver_registry::{LlmCallConfig, LlmResponseStream, LlmStreamEvent};
use crate::error::{AgentLoopError, LlmErrorKind, Result};
use crate::llm_retry::LlmRetryConfig;
use crate::tool_types::ToolDefinition;

use super::*;

// ------------------------------------------------------------------------
// Request-builder integration: `finalize_input_for_request` is the single
// gate that chooses whether the request `input` is trimmed. These tests
// pin the exact decision the call path makes — they catch regressions
// where the `previous_response_id`-presence check is accidentally dropped
// or inverted, which is what would re-introduce the bug even if the trim
// helper itself stays correct.
// ------------------------------------------------------------------------

pub(crate) fn sample_full_transcript_items() -> Vec<ResponsesInputItem> {
    vec![
        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("first request".to_string()),
            phase: None,
        },
        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "assistant".to_string(),
            content: ResponsesContent::Text("first reply".to_string()),
            phase: None,
        },
        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: "user".to_string(),
            content: ResponsesContent::Text("follow-up".to_string()),
            phase: None,
        },
    ]
}

// ------------------------------------------------------------------------
// EVE-597: stateless full-replay must not serialize a `function_call` whose
// `function_call_output` was evicted by compaction / model-view masking.
// OpenAI/Codex Responses 400 with "No tool output found for function call …"
// and the session wedges permanently. This is the sibling of EVE-519 (orphan
// output, covered above); the repair drops both sides of a broken pair.
// ------------------------------------------------------------------------

pub(crate) fn function_call(call_id: &str, name: &str) -> ResponsesInputItem {
    ResponsesInputItem::FunctionCall {
        r#type: "function_call".to_string(),
        call_id: call_id.to_string(),
        name: name.to_string(),
        arguments: "{}".to_string(),
    }
}

pub(crate) fn function_call_output(call_id: &str) -> ResponsesInputItem {
    ResponsesInputItem::FunctionCallOutput {
        r#type: "function_call_output".to_string(),
        call_id: call_id.to_string(),
        output: "result".to_string(),
    }
}

pub(crate) fn user_message(text: &str) -> ResponsesInputItem {
    ResponsesInputItem::Message {
        r#type: "message".to_string(),
        role: "user".to_string(),
        content: ResponsesContent::Text(text.to_string()),
        phase: None,
    }
}

/// Helper: create a ToolDefinition with optional category and deferrable policy
pub(crate) fn make_tool(
    name: &str,
    category: Option<&str>,
    deferrable: crate::tool_types::DeferrablePolicy,
) -> ToolDefinition {
    ToolDefinition::Builtin(crate::tool_types::BuiltinTool {
        name: name.to_string(),
        display_name: None,
        description: format!("{} description", name),
        parameters: json!({"type": "object", "properties": {}}),
        policy: crate::tool_types::ToolPolicy::Auto,
        category: category.map(|s| s.to_string()),
        deferrable,
        hints: crate::tool_types::ToolHints::default(),
        full_parameters: None,
    })
}

/// Minimal `LlmCallConfig` for wire tests.
pub(crate) fn auth_test_config() -> LlmCallConfig {
    LlmCallConfig {
        speed: None,
        verbosity: None,
        model: "gpt-5.4".to_string(),
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
        capture_request: false,
        limits: Default::default(),
        reasoning_state: None,
    }
}

pub(crate) struct SignedRequest {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) body: Vec<u8>,
}

pub(crate) struct RecordingAuth {
    pub(crate) requests: Arc<Mutex<Vec<SignedRequest>>>,
    pub(crate) fail_on: Option<usize>,
}

#[async_trait::async_trait]
impl crate::runtime_provider::ProviderAuth for RecordingAuth {
    async fn headers(
        &self,
        request: crate::runtime_provider::ProviderAuthRequest<'_>,
    ) -> Result<Vec<(String, String)>> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(SignedRequest {
            method: request.method.into(),
            url: request.url.into(),
            body: request.body.to_vec(),
        });
        let attempt = requests.len();
        if self.fail_on == Some(attempt) {
            return Err(AgentLoopError::llm_kind(
                LlmErrorKind::Authentication,
                "token refresh refused",
            ));
        }
        Ok(vec![(
            "Authorization".into(),
            format!("Bearer token-{attempt}"),
        )])
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub(crate) struct HeaderInjectingExtension;

impl OpenResponsesRequestExtension for HeaderInjectingExtension {
    fn decorate(&self, body: &mut Value, _config: &LlmCallConfig) -> Result<()> {
        body["routing_marker"] = json!("decorated");
        Ok(())
    }

    fn decorate_headers(&self, headers: &mut HeaderMap, _config: &LlmCallConfig) -> Result<()> {
        headers.insert(
            "x-route",
            reqwest::header::HeaderValue::from_static("fallback"),
        );
        headers.insert(
            "authorization",
            reqwest::header::HeaderValue::from_static("Bearer decoration"),
        );
        Ok(())
    }
}

pub(crate) fn successful_auth_stream() -> wiremock::ResponseTemplate {
    wiremock::ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"authenticated\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-auth\",\"status\":\"completed\",\"output\":[]}}\n\n"
        ))
}

pub(crate) async fn assert_authenticated_stream(mut stream: LlmResponseStream) {
    let mut text = String::new();
    let mut finishes = Vec::new();
    while let Some(event) = stream.next().await {
        match event.expect("stream transport succeeds") {
            LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
            LlmStreamEvent::Done(metadata) => finishes.push(metadata.finish_reason),
            other => panic!("unexpected auth response event: {other:?}"),
        }
    }
    assert_eq!(text, "authenticated");
    assert_eq!(finishes, vec![Some("stop".into())]);
}

pub(crate) fn auth_retry_config() -> LlmRetryConfig {
    LlmRetryConfig {
        max_retries: 1,
        initial_backoff: std::time::Duration::from_millis(1),
        max_backoff: std::time::Duration::from_millis(1),
        backoff_multiplier: 1.0,
        jitter_factor: 0.0,
        ..Default::default()
    }
}

pub(crate) fn cache_config() -> LlmCallConfig {
    let mut config = auth_test_config();
    config
        .metadata
        .insert("session_id".into(), "session-one".into());
    config.prompt_cache = Some(crate::driver_registry::PromptCacheConfig {
        enabled: true,
        strategy: crate::driver_registry::PromptCacheStrategy::Auto,
        gemini_cached_content: None,
    });
    config
}

pub(crate) fn search_tools() -> Vec<ToolDefinition> {
    use crate::tool_types::DeferrablePolicy::{Always, Automatic, Never};
    vec![
        make_tool("z", Some("Zeta"), Automatic),
        make_tool("first", Some("HiddenCategory"), Never),
        make_tool("a", Some("Alpha"), Always),
        make_tool("loose", None, Automatic),
        make_tool("second", None, Never),
        make_tool("b", Some("Alpha"), Automatic),
    ]
}

pub(crate) fn expected_search_tools() -> Value {
    // Independent literal wire contract; only repeated fixture names are parameterized.
    let function = |name: &str, deferred: bool| {
        let mut value = json!({"type":"function","name":name,"description":format!("{name} description"),"parameters":{"type":"object","properties":{},"required":[],"additionalProperties":false},"strict":true});
        if deferred {
            value["defer_loading"] = json!(true);
        }
        value
    };
    json!([
        function("first", false), function("second", false),
        {"type":"namespace","name":"Alpha","description":"Tools for Alpha","tools":[function("a", true),function("b", true)]},
        {"type":"namespace","name":"Zeta","description":"Tools for Zeta","tools":[function("z", true)]},
        function("loose", true), {"type":"tool_search"}
    ])
}

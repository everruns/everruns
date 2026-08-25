//! Live tests for the request-level `parallel_tool_calls` preference against the
//! real OpenAI API (Responses).
//!
//! Proves the wire mapping is accepted by the provider and has effect:
//! - `Some(false)` (avoid) is accepted and the provider returns at most one tool
//!   call in a single completion.
//! - `Some(true)` (prefer) is accepted and the provider returns tool calls.
//!
//! Ignored by default (requires network + `OPENAI_API_KEY`); run manually:
//!   `doppler run -- cargo test -p everruns-openai --test parallel_tool_calls_live -- --ignored --nocapture`

use everruns_openai::provider;
use everruns_provider::driver_registry::{LlmCallConfig, LlmMessage, LlmMessageRole};
use everruns_provider::model::ReasoningEffort;
use everruns_provider::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolDefinition, ToolHints, ToolPolicy,
};

const LIVE_MODEL: &str = "gpt-5.6-luna";

fn api_key() -> String {
    std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set for the live test")
}

fn tool(name: &str, description: &str) -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: name.to_string(),
        display_name: None,
        description: description.to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"]
        }),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints: ToolHints::default(),
        full_parameters: None,
    })
}

fn config_with(parallel: Option<bool>) -> LlmCallConfig {
    LlmCallConfig {
        speed: None,
        verbosity: None,
        model: LIVE_MODEL.to_string(),
        temperature: None,
        max_tokens: Some(512),
        tools: vec![
            tool("get_weather", "Get the current weather for a city."),
            tool("get_local_time", "Get the current local time for a city."),
        ],
        reasoning_effort: Some(ReasoningEffort::Low),
        metadata: std::collections::HashMap::new(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: None,
        openrouter_routing: None,
        parallel_tool_calls: parallel,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
    }
}

async fn tool_call_count(parallel: Option<bool>) -> usize {
    let provider = provider("openai", api_key());
    let messages = vec![
        LlmMessage::text(
            LlmMessageRole::System,
            "Use the tools to answer. Always call the tools rather than guessing.",
        ),
        LlmMessage::text(
            LlmMessageRole::User,
            "Get both the current weather and the current local time in Paris.",
        ),
    ];
    let response = provider
        .chat_completion(messages, &config_with(parallel))
        .await
        .expect("OpenAI should accept the request");
    response.tool_calls.map(|calls| calls.len()).unwrap_or(0)
}

#[tokio::test]
#[ignore = "live network + OPENAI_API_KEY"]
async fn openai_avoid_caps_tool_calls_at_one() {
    let count = tool_call_count(Some(false)).await;
    eprintln!("openai avoid: {count} tool call(s)");
    assert!(
        count <= 1,
        "avoid (disable parallel) should yield at most one tool call, got {count}"
    );
}

#[tokio::test]
#[ignore = "live network + OPENAI_API_KEY"]
async fn openai_prefer_is_accepted_and_calls_tools() {
    let count = tool_call_count(Some(true)).await;
    eprintln!("openai prefer: {count} tool call(s)");
    assert!(
        count >= 1,
        "prefer should be accepted and produce at least one tool call, got {count}"
    );
}

//! Live compatibility tests against Meta Model API.
//!
//! These tests intentionally use the lower-cost Contributor tier. They are
//! ignored by default and require `MODEL_API_KEY`:
//!   `doppler run -- cargo test -p everruns-meta --test model_api_live -- --ignored --nocapture`

use everruns_meta::provider;
use everruns_provider::driver_registry::{LlmCallConfig, LlmMessage, LlmMessageRole};
use everruns_provider::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolDefinition, ToolHints, ToolPolicy,
};

const LIVE_MODEL: &str = "muse-spark-1.2-contributor";

fn api_key() -> String {
    std::env::var("MODEL_API_KEY")
        .ok()
        .filter(|key| !key.is_empty())
        .expect("MODEL_API_KEY must be set and non-empty for the Meta live tests")
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

fn config(tools: Vec<ToolDefinition>, parallel_tool_calls: Option<bool>) -> LlmCallConfig {
    LlmCallConfig {
        model: LIVE_MODEL.to_string(),
        temperature: Some(0.0),
        max_tokens: Some(512),
        tools,
        reasoning_effort: Some("low".to_string()),
        speed: None,
        verbosity: None,
        metadata: std::collections::HashMap::new(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: None,
        openrouter_routing: None,
        parallel_tool_calls,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
    }
}

#[tokio::test]
#[ignore = "live network + MODEL_API_KEY"]
async fn contributor_completes_text_response() {
    let provider = provider("meta", api_key());
    let messages = vec![LlmMessage::text(
        LlmMessageRole::User,
        "Reply with exactly: muse-live-ok",
    )];

    let response = provider
        .chat_completion(messages, &config(vec![], None))
        .await
        .expect("Meta Model API should complete a text response");

    eprintln!("meta text response: {}", response.text);
    assert!(
        response.text.to_ascii_lowercase().contains("muse-live-ok"),
        "unexpected Meta response: {}",
        response.text
    );
}

#[tokio::test]
#[ignore = "live network + MODEL_API_KEY"]
async fn contributor_accepts_parallel_tool_calls() {
    let provider = provider("meta", api_key());
    let messages = vec![
        LlmMessage::text(
            LlmMessageRole::System,
            "Always call the supplied tools rather than guessing.",
        ),
        LlmMessage::text(
            LlmMessageRole::User,
            "Get both the current weather and the current local time in Paris.",
        ),
    ];
    let tools = vec![
        tool("get_weather", "Get the current weather for a city."),
        tool("get_local_time", "Get the current local time for a city."),
    ];

    let response = provider
        .chat_completion(messages, &config(tools, Some(true)))
        .await
        .expect("Meta Model API should accept parallel tool calls");
    let count = response.tool_calls.map(|calls| calls.len()).unwrap_or(0);

    eprintln!("meta parallel preference: {count} tool call(s)");
    assert!(
        count >= 1,
        "expected at least one Meta tool call, got {count}"
    );
}

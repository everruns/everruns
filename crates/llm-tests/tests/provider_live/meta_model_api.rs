//! Live compatibility tests against Meta Model API.
//!
//! These tests intentionally use the lower-cost Contributor tier, plus Muse
//! Ember (Standard only, priced near Contributor), and need `MODEL_API_KEY`.

use everruns_contracts::driver_registry::{LlmCallConfig, Message, MessageRole};
use everruns_contracts::model::ReasoningEffort;
use everruns_contracts::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolDefinition, ToolHints, ToolPolicy,
};
use everruns_drivers::meta::provider;

const LIVE_MODEL: &str = "muse-spark-1.3-contributor";
const EMBER_MODEL: &str = "muse-ember-1.0";

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
    config_for(LIVE_MODEL, tools, parallel_tool_calls)
}

fn config_for(
    model: &str,
    tools: Vec<ToolDefinition>,
    parallel_tool_calls: Option<bool>,
) -> LlmCallConfig {
    let mut config = LlmCallConfig::new(model);
    config.temperature = Some(0.0);
    config.max_tokens = Some(512);
    config.tools = tools;
    config.reasoning_effort = Some(ReasoningEffort::Low);
    config.parallel_tool_calls = parallel_tool_calls;
    config
}

#[tokio::test]
async fn contributor_completes_text_response() {
    let provider = provider("meta", api_key());
    let messages = vec![Message::text(
        MessageRole::User,
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
async fn contributor_accepts_parallel_tool_calls() {
    let provider = provider("meta", api_key());
    let messages = vec![
        Message::text(
            MessageRole::System,
            "Always call the supplied tools rather than guessing.",
        ),
        Message::text(
            MessageRole::User,
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

/// Ember is the Meta driver's small model: it must answer and call tools
/// through the same Responses path as Spark.
#[tokio::test]
async fn ember_completes_text_and_calls_a_tool() {
    let provider = provider("meta", api_key());

    let text = provider
        .chat_completion(
            vec![Message::text(
                MessageRole::User,
                "Reply with exactly: ember-live-ok",
            )],
            &config_for(EMBER_MODEL, vec![], None),
        )
        .await
        .expect("Muse Ember should complete a text response");
    eprintln!("ember text response: {}", text.text);
    assert!(
        text.text.to_ascii_lowercase().contains("ember-live-ok"),
        "unexpected Ember response: {}",
        text.text
    );

    let messages = vec![
        Message::text(
            MessageRole::System,
            "Always call the supplied tools rather than guessing.",
        ),
        Message::text(MessageRole::User, "What is the weather in Paris?"),
    ];
    let tools = vec![tool("get_weather", "Get the current weather for a city.")];
    let response = provider
        .chat_completion(messages, &config_for(EMBER_MODEL, tools, None))
        .await
        .expect("Muse Ember should call a tool");
    let calls = response.tool_calls.unwrap_or_default();
    eprintln!("ember tool calls: {calls:?}");
    assert!(
        calls.iter().any(|call| call.name == "get_weather"),
        "expected Ember to call get_weather, got {calls:?}"
    );
}

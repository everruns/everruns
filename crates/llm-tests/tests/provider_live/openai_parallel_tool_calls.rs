//! Live tests for the request-level `parallel_tool_calls` preference against the
//! real OpenAI API (Responses).
//!
//! Proves the wire mapping is accepted by the provider and has effect:
//! - `Some(false)` (avoid) is accepted and the provider returns at most one tool
//!   call in a single completion.
//! - `Some(true)` (prefer) is accepted and the provider returns tool calls.

use everruns_contracts::driver_registry::{LlmCallConfig, Message, MessageRole};
use everruns_contracts::model::ReasoningEffort;
use everruns_contracts::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolDefinition, ToolHints, ToolPolicy,
};
use everruns_drivers::openai::provider;

use crate::{TRANSPORT_ATTEMPTS, is_transient_transport};

const LIVE_MODEL: &str = "gpt-6-luna";

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
    let mut config = LlmCallConfig::new(LIVE_MODEL);
    config.max_tokens = Some(512);
    config.tools = vec![
        tool("get_weather", "Get the current weather for a city."),
        tool("get_local_time", "Get the current local time for a city."),
    ];
    config.reasoning_effort = Some(ReasoningEffort::Low);
    config.parallel_tool_calls = parallel;
    config
}

async fn tool_call_count(parallel: Option<bool>) -> usize {
    let provider = provider("openai", api_key());
    let messages = vec![
        Message::text(
            MessageRole::System,
            "Use the tools to answer. Always call the tools rather than guessing.",
        ),
        Message::text(
            MessageRole::User,
            "Get both the current weather and the current local time in Paris.",
        ),
    ];
    let config = config_with(parallel);
    let response = with_transport_retries(|| provider.chat_completion(messages.clone(), &config))
        .await
        .expect("OpenAI should accept the request");
    response.tool_calls.map(|calls| calls.len()).unwrap_or(0)
}

#[tokio::test]
async fn openai_avoid_caps_tool_calls_at_one() {
    let count = tool_call_count(Some(false)).await;
    eprintln!("openai avoid: {count} tool call(s)");
    assert!(
        count <= 1,
        "avoid (disable parallel) should yield at most one tool call, got {count}"
    );
}

#[tokio::test]
async fn openai_prefer_is_accepted_and_calls_tools() {
    let count = tool_call_count(Some(true)).await;
    eprintln!("openai prefer: {count} tool call(s)");
    assert!(
        count >= 1,
        "prefer should be accepted and produce at least one tool call, got {count}"
    );
}

/// A dropped stream says nothing about parallel-tool semantics. Use the same
/// bounded transport budget as the other live tests, never retry assertions or
/// provider/contract errors, and fail if every attempt loses its transport.
async fn with_transport_retries<T, E, F, Fut>(mut call: F) -> Result<T, E>
where
    E: std::fmt::Display,
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    for attempt in 1..=TRANSPORT_ATTEMPTS {
        match call().await {
            Err(error)
                if attempt < TRANSPORT_ATTEMPTS && is_transient_transport(&error.to_string()) =>
            {
                eprintln!(
                    "parallel tools attempt {attempt}/{TRANSPORT_ATTEMPTS} lost to transport; retrying"
                );
            }
            result => return result,
        }
    }
    unreachable!("the final attempt returns its result")
}

#[tokio::test]
async fn parallel_transport_retry_recovers_a_dropped_stream() {
    let mut attempts = 0;
    let result = with_transport_retries(|| {
        attempts += 1;
        std::future::ready(if attempts == 1 {
            Err("error decoding response body")
        } else {
            Ok(2)
        })
    })
    .await;
    assert_eq!(result, Ok(2));
    assert_eq!(attempts, 2);
}

#[tokio::test]
async fn parallel_transport_retry_still_fails_at_the_budget() {
    let mut attempts = 0;
    let result = with_transport_retries(|| {
        attempts += 1;
        std::future::ready(Err::<(), _>("error decoding response body"))
    })
    .await;
    assert!(result.is_err());
    assert_eq!(attempts, TRANSPORT_ATTEMPTS);
}

#[tokio::test]
async fn parallel_transport_retry_does_not_hide_provider_errors() {
    let mut attempts = 0;
    let result = with_transport_retries(|| {
        attempts += 1;
        std::future::ready(Err::<(), _>("400 invalid parallel_tool_calls"))
    })
    .await;
    assert!(result.is_err());
    assert_eq!(attempts, 1);
}

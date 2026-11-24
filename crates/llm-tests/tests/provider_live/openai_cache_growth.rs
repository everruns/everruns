//! Real-provider qualification of stable history caching beside volatile facts.
use everruns_contracts::driver_registry::{LlmCallConfig, Message, MessageRole};
use everruns_contracts::model::ReasoningEffort;
use everruns_contracts::tool_types::{ToolCall, ToolDefinition};
use everruns_drivers::openai::provider;
use serde_json::json;

#[tokio::test]
async fn openai_cache_reuses_history_when_runtime_facts_change() {
    let provider = provider(
        "openai",
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY required"),
    );
    let mut config = LlmCallConfig::new("gpt-6.2-sol");
    config.reasoning_effort = Some(ReasoningEffort::Low);
    config.max_tokens = Some(128);
    config.volatile_suffix_len = 1;
    config.capture_request = true;
    config.tools = vec![ToolDefinition::function(
        "read_catalog",
        "Read catalog records",
        json!({"type":"object","properties":{},"additionalProperties":false}),
    )];
    let prefix = format!("Qualification {}. Reply only OK.\n{}", uuid::Uuid::new_v4(),
        (0..220).map(|i| format!("Record {i}: immutable catalog entry with verified blue widgets and fixed inventory.\n")).collect::<String>());
    let messages = |facts: &str| {
        let mut call = Message::text(MessageRole::Assistant, "");
        call.tool_calls = Some(vec![ToolCall {
            id: "read_catalog_1".into(),
            name: "read_catalog".into(),
            arguments: json!({}),
        }]);
        let mut result = Message::text(MessageRole::Tool, prefix.clone());
        result.tool_call_id = Some("read_catalog_1".into());
        vec![
            Message::text(MessageRole::System, "Read the records. Reply only OK."),
            Message::text(MessageRole::User, "Read the catalog, then reply OK."),
            call,
            result,
            Message::text(MessageRole::User, facts),
        ]
    };
    let first = provider
        .chat_completion(messages("Runtime clock: 01:00"), &config)
        .await
        .expect("cold request");
    let second = provider
        .chat_completion(messages("Runtime clock: 01:01"), &config)
        .await
        .expect("warm request");
    let read = second.metadata.cache_read_tokens.unwrap_or(0);
    eprintln!(
        "cache growth: first write={:?}, second read={read}, second write={:?}",
        first.metadata.cache_creation_tokens, second.metadata.cache_creation_tokens
    );
    assert!(read > 1024, "stable history must hit despite changed facts");
    let body = second.metadata.request_body.expect("captured request");
    assert_eq!(body["prompt_cache_options"]["mode"], "implicit");
    assert_eq!(body["prompt_cache_options"]["ttl"], "30m");
    assert!(body.get("instructions").is_none());
    let result = body["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output")
        .unwrap();
    assert!(result["output"][0].get("prompt_cache_breakpoint").is_some());
}

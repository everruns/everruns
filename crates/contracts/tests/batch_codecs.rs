//! Offline responses follow the interactive codec's output and schema contracts.
#![cfg(feature = "http")]
#![allow(clippy::unwrap_used)]
use everruns_contracts::driver_registry::{Message, MessageRole};
use everruns_contracts::{
    LlmCallConfig, OpenAIProtocolChatDriver, OpenResponsesProtocolChatDriver, ToolDefinition,
};
use serde_json::{Value, json};
fn config() -> LlmCallConfig {
    let mut config = LlmCallConfig::new("gpt-6.1-sol");
    config.max_tokens = Some(64);
    config.tools = vec![ToolDefinition::function(
        "lookup",
        "Look up a value",
        json!({"type":"object","properties":{"value":{"type":"integer"},"optional":{"type":"string"}},"required":["value"],"additionalProperties":false}),
    )];
    config
}
fn resource() -> Value {
    json!({"id":"resp_batch","object":"response","created_at":0,"status":"completed","model":"gpt-6.1-sol","output":[
        {"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"Safe summary"}],"content":[{"type":"reasoning_text","text":"private raw reasoning"}],"encrypted_content":"opaque-encrypted"},
        {"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Answer"}]},
        {"type":"function_call","id":"fc_1","call_id":"call_1","name":"lookup","arguments":"{\"value\":1}","status":"completed"},
        {"type":"function_call","id":"fc_bad","call_id":"call_bad","name":"lookup","arguments":"{","status":"incomplete"}
    ],"usage":{"input_tokens":10,"output_tokens":4,"total_tokens":14,"input_tokens_details":{"cached_tokens":3},"output_tokens_details":{"reasoning_tokens":2}}})
}
#[test]
fn batch_encoding_preserves_optional_arguments_for_schema_free_resumption() {
    let config = config();
    let messages = vec![Message::text(MessageRole::User, "Look it up.")];
    let responses = OpenResponsesProtocolChatDriver::new()
        .encode_batch_request(&messages, &config)
        .unwrap();
    let completions = OpenAIProtocolChatDriver::new()
        .encode_batch_request(&messages, &config)
        .unwrap();
    for (wire, tool_path) in [
        (&responses, "/tools/0"),
        (&completions, "/tools/0/function"),
    ] {
        assert_eq!(wire["stream"], false);
        let tool = wire.pointer(tool_path).unwrap();
        assert_eq!(tool["strict"], false);
        assert_eq!(tool["parameters"]["required"], json!(["value"]));
        assert!(wire.get("background").is_none());
        assert!(wire.get("previous_response_id").is_none());
    }
    assert_eq!(responses["max_output_tokens"], 64);
    assert_eq!(completions["max_completion_tokens"], 64);
}
#[tokio::test]
async fn responses_reuse_canonical_tools_reasoning_usage_and_terminal_decoding() {
    let response = OpenResponsesProtocolChatDriver::new()
        .decode_batch_response(resource())
        .await
        .unwrap();
    assert_eq!(response.text, "Answer");
    let calls = response.tool_calls.unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].arguments, json!({"value":1}));
    assert_eq!(response.metadata.total_tokens, Some(14));
    assert_eq!(response.metadata.prompt_tokens, Some(7));
    assert_eq!(response.metadata.cache_read_tokens, Some(3));
    assert_eq!(response.metadata.reasoning_tokens, Some(2));
    assert_eq!(response.metadata.response_id.as_deref(), Some("resp_batch"));
    assert_eq!(response.reasoning.len(), 1);
    let reasoning = serde_json::to_string(&response.reasoning).unwrap();
    assert!(reasoning.contains("Safe summary"));
    assert!(reasoning.contains("opaque-encrypted"));
    assert!(!reasoning.contains("private raw reasoning"));
    let mut incomplete = resource();
    incomplete["status"] = json!("incomplete");
    incomplete["incomplete_details"] = json!({"reason":"max_output_tokens"});
    let response = OpenResponsesProtocolChatDriver::new()
        .decode_batch_response(incomplete)
        .await
        .unwrap();
    assert_eq!(response.text, "Answer");
    assert_eq!(
        response.metadata.provider_finish_reason.as_deref(),
        Some("max_output_tokens")
    );
}
#[test]
fn completions_reuse_canonical_tool_and_disjoint_usage_decoding() {
    let response=OpenAIProtocolChatDriver::new().decode_batch_response(json!({"id":"chatcmpl_batch","model":"gpt-6.1-sol","choices":[{"message":{"role":"assistant","content":"Answer","tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{\"value\":1}"}},{"id":"bad","type":"function","function":{"name":"lookup","arguments":"{"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":4,"prompt_tokens_details":{"cached_tokens":3},"completion_tokens_details":{"reasoning_tokens":2}}})).unwrap();
    assert_eq!(response.text, "Answer");
    assert_eq!(response.tool_calls.unwrap().len(), 1);
    assert_eq!(response.metadata.prompt_tokens, Some(7));
    assert_eq!(response.metadata.cache_read_tokens, Some(3));
    assert_eq!(response.metadata.total_tokens, Some(14));
    assert_eq!(response.metadata.reasoning_tokens, Some(2));
    assert_eq!(
        response.metadata.response_id.as_deref(),
        Some("chatcmpl_batch")
    );
}
#[tokio::test]
async fn malformed_successes_are_rejected_without_exposing_the_payload() {
    let responses = OpenResponsesProtocolChatDriver::new();
    let completions = OpenAIProtocolChatDriver::new();
    assert!(
        responses
            .decode_batch_response(json!({"secret":"private"}))
            .await
            .is_err()
    );
    assert!(
        completions
            .decode_batch_response(json!({"secret":"private"}))
            .is_err()
    );
    let mut failed = resource();
    failed["status"] = json!("failed");
    assert!(responses.decode_batch_response(failed).await.is_err());
}

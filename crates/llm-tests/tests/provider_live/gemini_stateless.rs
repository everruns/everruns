//! Google's Interactions API accepts stateless replay through the real driver.
use everruns_contracts::driver_registry::{LlmCallConfig, Message, MessageRole};
use everruns_contracts::model::ReasoningEffort;
use everruns_drivers::gemini::provider;
use serde_json::json;

#[tokio::test]
async fn gemini_stateless_replays_without_a_stored_interaction() {
    let key = std::env::var("GEMINI_API_KEY")
        .or_else(|_| std::env::var("GOOGLE_API_KEY"))
        .expect("Gemini API key required");
    let provider = provider("gemini", key);
    let mut config = LlmCallConfig::new("gemini-4-hydrogen");
    config.max_tokens = Some(128);
    config.reasoning_effort = Some(ReasoningEffort::Low);
    config.prefer_non_streaming = true;
    config.capture_request = true;
    config
        .driver_options
        .insert("gemini/interactions".into(), json!({"store":false}));
    // An invalid old chain must be ignored rather than sent or retried.
    config.previous_response_id = Some("must-not-use-this-chain".into());
    let mut messages = vec![Message::text(
        MessageRole::User,
        "Remember the word cobalt. Reply only OK.",
    )];
    let first = provider
        .chat_completion(messages.clone(), &config)
        .await
        .expect("stateless first call");
    messages.push(Message::text(MessageRole::Assistant, first.text));
    messages.push(Message::text(
        MessageRole::User,
        "What word did I ask you to remember? Reply with that word only.",
    ));
    let second = provider
        .chat_completion(messages, &config)
        .await
        .expect("stateless replay");
    assert!(second.text.to_lowercase().contains("cobalt"));
    let body = second.metadata.request_body.expect("captured request");
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], false);
    assert!(body.get("previous_interaction_id").is_none());
    assert_eq!(body["input"].as_array().unwrap().len(), 3);
}

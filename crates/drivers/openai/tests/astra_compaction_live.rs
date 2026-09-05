//! Live acceptance gate for Astra configuration updates and explicit compaction.
//! Run only after the account has credits:
//! doppler run --project everruns-dev --config dev -- cargo test -p everruns-openai --test astra_compaction_live -- --ignored --nocapture

use everruns_provider::compact::{CompactRequest, messages_to_compact_input};
use everruns_provider::driver_registry::{
    LlmCallConfig, LlmMessage, LlmMessageRole, ProviderOpaqueContext,
};
use everruns_provider::reasoning_updates::ReasoningState;
use everruns_provider::{ProviderEndpoint, ReasoningEffort};
use serde_json::json;

#[tokio::test]
#[ignore = "requires funded OpenAI API credentials; blocked by credit_balance_exhausted"]
async fn astra_compaction_preserves_facts_constraints_and_continuation() {
    let key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY is required");
    let provider = everruns_openai::provider("openai", key);
    let nonce = format!(
        "gate-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let mut config = LlmCallConfig {
        model: "gpt-6-astra".into(),
        temperature: None,
        max_tokens: Some(2048),
        tools: vec![],
        reasoning_effort: Some(ReasoningEffort::Low),
        speed: None,
        verbosity: None,
        metadata: Default::default(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: None,
        openrouter_routing: None,
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: vec![],
        cache_diagnostics: None,
        reasoning_state: Some(ReasoningState {
            epoch: "live-gate".into(),
            baseline: Some(ReasoningEffort::Low),
            effective: Some(ReasoningEffort::Low),
            pending: None,
        }),
    };
    let mut history = vec![LlmMessage::text(
        LlmMessageRole::User,
        format!(
            "Remember code={nonce}, timezone=Pacific/Auckland, guard=read-only. Preserve these exact facts and constraints. Reply only ACK for now."
        ),
    )];
    let first = provider
        .chat_completion(history.clone(), &config)
        .await
        .expect("initial live response");
    assert!(first.tool_calls.as_ref().is_none_or(Vec::is_empty));
    let mut first_message = LlmMessage::text(LlmMessageRole::Assistant, first.text);
    first_message.reasoning = first.reasoning;
    history.push(first_message);
    history.push(LlmMessage::text(
        LlmMessageRole::User,
        "Continue to preserve the original facts and constraints. Reply only ACK again.",
    ));
    config.previous_response_id = Some(first.metadata.response_id.expect("stateful response ID"));
    config.reasoning_state.as_mut().unwrap().effective = Some(ReasoningEffort::High);
    config.reasoning_state.as_mut().unwrap().pending = Some(ReasoningEffort::High);
    let second = provider
        .chat_completion(history.clone(), &config)
        .await
        .expect("live reasoning update");
    assert!(second.tool_calls.as_ref().is_none_or(Vec::is_empty));
    history[2].configuration_update = Some(ReasoningEffort::High);
    let mut second_message = LlmMessage::text(LlmMessageRole::Assistant, second.text);
    second_message.reasoning = second.reasoning;
    history.push(second_message);
    config.reasoning_state.as_mut().unwrap().pending = None;
    let input = messages_to_compact_input(&history);
    let compacted = provider
        .clone()
        .into_boxed_driver()
        .compact(
            &ProviderEndpoint::default(),
            CompactRequest {
                model: config.model.clone(),
                input,
                previous_response_id: None,
                instructions: None,
                reasoning_state: config.reasoning_state.clone(),
            },
        )
        .await
        .expect("successful live explicit compaction")
        .expect("native compaction supported");
    let output = json!(compacted.output);
    let types: Vec<_> = output
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["type"].as_str().unwrap_or("unknown"))
        .collect();
    eprintln!("explicit compaction output types: {types:?}");
    assert_eq!(types.first(), Some(&"compaction"));
    assert!(
        !types
            .iter()
            .any(|kind| matches!(*kind, "function_call" | "custom_tool_call")),
        "compaction must not create unexecuted tool calls"
    );
    let context = ProviderOpaqueContext::OpenResponsesCompact {
        output: compacted.output,
        reasoning_state: config.reasoning_state.clone(),
    };
    // Cross the same JSON persistence boundary used by native checkpoints.
    config.provider_opaque_context = Some(serde_json::from_value(json!(context)).unwrap());
    config.previous_response_id = None;
    let continuation = provider.chat_completion(vec![LlmMessage::text(LlmMessageRole::User,
        "Return only a JSON object with the original code, timezone, and guard. Use keys code, timezone, guard. Do not invent or reset any value.")], &config)
        .await.expect("fresh post-compaction continuation");
    let facts: serde_json::Value =
        serde_json::from_str(continuation.text.trim()).expect("JSON continuation");
    assert_eq!(
        facts,
        json!({"code":nonce,"timezone":"Pacific/Auckland","guard":"read-only"})
    );
    assert!(continuation.tool_calls.as_ref().is_none_or(Vec::is_empty));
    eprintln!(
        "PASS: exact facts and constraints survived explicit compaction, JSON checkpoint reload, and fresh continuation with no response ID"
    );
}

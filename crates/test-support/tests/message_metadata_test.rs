// End-to-end test for the message_metadata capability.
//
// Uses the in-memory agentic loop with the echo sim driver: the assistant
// reply mirrors exactly what the LLM received for the last user message, so
// the timestamp annotation must be visible in the reply while the stored
// user message stays unannotated.
//
// Run with: cargo test -p everruns-core --test message_metadata_test

use everruns_builtins::MessageMetadataCapability;
use everruns_core::MessageRetriever;
use everruns_core::message::MessageRole;
use everruns_llmsim::LlmSimConfig;
use everruns_test_support::InMemoryAgenticLoop;

#[tokio::test]
async fn message_metadata_annotates_llm_view_not_storage() {
    let agent_loop = InMemoryAgenticLoop::builder()
        .with_llm_sim(LlmSimConfig::echo())
        .capability(MessageMetadataCapability)
        .build()
        .await
        .expect("loop builds");

    let result = agent_loop.run_turn("hello there").await.expect("turn runs");

    assert!(
        result.response.contains("[time 2") && result.response.contains("Z] hello there"),
        "LLM should see the timestamp annotation, got: {}",
        result.response
    );

    // The annotation is model-view only: the stored user message is unchanged.
    let messages = agent_loop
        .message_retriever()
        .load(agent_loop.session_id())
        .await
        .expect("messages load");
    let stored_user = messages
        .iter()
        .find(|m| m.role == MessageRole::User)
        .expect("user message stored");
    assert_eq!(stored_user.text(), Some("hello there"));
}

#[tokio::test]
async fn without_capability_no_annotation() {
    let agent_loop = InMemoryAgenticLoop::builder()
        .with_llm_sim(LlmSimConfig::echo())
        .build()
        .await
        .expect("loop builds");

    let result = agent_loop.run_turn("hello there").await.expect("turn runs");

    assert!(
        !result.response.contains("[time "),
        "annotation must be opt-in, got: {}",
        result.response
    );
}

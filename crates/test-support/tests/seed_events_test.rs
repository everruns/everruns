// Seeding a fixture session with a prior conversation.
//
// Conversation history is event-sourced: `EventHistory` is a read model over
// the session's event log, so a fixture that needs a prior conversation must
// replay envelopes rather than write messages. This test pins that contract —
// seeded envelopes project into history in order, and a subsequent turn
// appends on top of them instead of replacing them.
//
// Run with: cargo test -p everruns-test-support --test seed_events_test

use everruns_core::MessageRetriever;
use everruns_core::events::{EventData, InputMessageData, OutputMessageCompletedData};
use everruns_core::message::{Message, MessageRole};
use everruns_test_support::{InMemoryAgenticLoop, LlmSimConfig};

#[tokio::test]
async fn seeded_events_project_into_history_before_the_next_turn() {
    let agent_loop = InMemoryAgenticLoop::builder()
        .with_llm_sim(LlmSimConfig::echo())
        .build()
        .await
        .expect("loop builds");

    agent_loop
        .seed_events([
            EventData::InputMessage(InputMessageData::new(Message::user("first question"))),
            EventData::OutputMessageCompleted(OutputMessageCompletedData::new(Message::assistant(
                "first answer",
            ))),
        ])
        .await
        .expect("seed events append");

    let seeded = agent_loop
        .message_retriever()
        .load(agent_loop.session_id())
        .await
        .expect("messages load");
    let seeded_text: Vec<_> = seeded.iter().filter_map(|m| m.text()).collect();
    assert_eq!(seeded_text, vec!["first question", "first answer"]);

    agent_loop
        .run_turn("second question")
        .await
        .expect("turn runs");

    let after = agent_loop
        .message_retriever()
        .load(agent_loop.session_id())
        .await
        .expect("messages load");
    let after_text: Vec<_> = after.iter().filter_map(|m| m.text()).collect();
    assert_eq!(
        &after_text[..2],
        &["first question", "first answer"],
        "the turn appends to seeded history instead of replacing it"
    );
    assert!(
        after_text.contains(&"second question"),
        "the new input is appended, got: {after_text:?}"
    );
    assert_eq!(
        after.first().map(|m| &m.role),
        Some(&MessageRole::User),
        "seeded roles survive projection"
    );
}

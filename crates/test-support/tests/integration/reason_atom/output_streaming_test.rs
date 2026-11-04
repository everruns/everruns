//! Output demand regressions; the paired event-cost workload is a bench target.
use super::setup;
#[path = "output_streaming_fixture.rs"]
mod fixture;
use everruns_contracts::runtime::conversation::Communication;
use everruns_core::EventData;
use fixture::{MeasuredEmitter, run};
use std::time::Duration;

#[tokio::test]
async fn explicit_notes_complete_without_text_or_thinking_deltas() {
    let (result, events, _) = run(
        setup::setup_test_environment().await,
        Communication::Explicit,
        None,
        3,
        Duration::from_millis(55),
        MeasuredEmitter::default(),
    )
    .await;
    assert!(result.success && result.commentary);
    assert_eq!(result.text, "x".repeat(3 * 4096));
    assert!(events.iter().all(|event| !matches!(
        event.data,
        EventData::OutputMessageDelta(_) | EventData::ReasonThinkingDelta(_)
    )));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.data, EventData::OutputMessageCompleted(_)))
            .count(),
        1
    );
    assert!(events.iter().any(|event| matches!(&event.data, EventData::ReasonThinkingCompleted(data) if data.thinking == "r".repeat(3 * 4096))));
    assert!(
        events
            .iter()
            .any(|event| matches!(event.data, EventData::LlmGeneration(_)))
    );
}

#[tokio::test]
async fn direct_chat_keeps_live_deltas_and_explicit_debug_can_opt_in() {
    for communication in [Communication::Direct, Communication::Explicit] {
        let hint = communication.is_explicit().then_some(true);
        let (_, events, _) = run(
            setup::setup_test_environment().await,
            communication,
            hint,
            2,
            Duration::ZERO,
            MeasuredEmitter::default(),
        )
        .await;
        assert!(
            events
                .iter()
                .any(|event| matches!(event.data, EventData::OutputMessageDelta(_)))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event.data, EventData::ReasonThinkingDelta(_)))
        );
    }
}

#[tokio::test]
async fn completion_only_direct_consumer_can_suppress_deltas() {
    let (result, events, _) = run(
        setup::setup_test_environment().await,
        Communication::Direct,
        Some(false),
        2,
        Duration::ZERO,
        MeasuredEmitter::default(),
    )
    .await;
    assert!(result.success && !result.commentary);
    assert!(events.iter().all(|event| !matches!(
        event.data,
        EventData::OutputMessageDelta(_) | EventData::ReasonThinkingDelta(_)
    )));
}

//! A schedule fire's reason step sends the model a short view: the fire, the
//! last runs of the same schedule, and none of the unrelated history.

use super::*;

fn fire(text: &str) -> RuntimeMessage {
    use everruns_core::message::{MessageOrigin, MessageOriginKind, MessageOriginRun};
    let mut message = RuntimeMessage::user(text);
    let origin = MessageOrigin::new(MessageOriginKind::Schedule)
        .with_source_id("schedule_poll")
        .with_run(MessageOriginRun {
            number: Some(1),
            ..Default::default()
        });
    message.metadata = Some(origin.into_metadata());
    message
}

#[tokio::test]
async fn a_schedule_fire_reaches_the_model_without_the_unrelated_history() {
    let rig = ProactiveTestRig::new(
        DriverId::external("openai-codex"),
        10_000_000,
        (100, 10),
        false,
        false,
    )
    .await;
    rig.message_retriever
        .seed(
            rig.session_id.into(),
            vec![
                RuntimeMessage::user(format!("long chat {}", "x".repeat(50_000))),
                RuntimeMessage::assistant("I scheduled the poll."),
                fire("poll run 1"),
                RuntimeMessage::assistant("still pending after run 1"),
                RuntimeMessage::user("an unrelated question"),
                RuntimeMessage::assistant("an unrelated answer"),
                fire("poll run 2"),
            ],
        )
        .await;

    rig.execute(None).await.unwrap();

    let calls = rig.calls.lock().await;
    let (messages, _) = calls.last().expect("the model was called");
    let sent = format!("{messages:?}");
    assert!(sent.contains("poll run 2"), "{sent}");
    assert!(sent.contains("still pending after run 1"), "{sent}");
    assert!(sent.contains("This is a scheduled run."), "{sent}");
    assert!(!sent.contains("long chat"), "{sent}");
    assert!(!sent.contains("an unrelated question"), "{sent}");
    assert!(!sent.contains("an unrelated answer"), "{sent}");
}

#[tokio::test]
async fn a_chat_turn_after_a_fire_still_sees_the_whole_history() {
    let rig = ProactiveTestRig::new(
        DriverId::external("openai-codex"),
        10_000_000,
        (100, 10),
        false,
        false,
    )
    .await;
    rig.message_retriever
        .seed(
            rig.session_id.into(),
            vec![
                RuntimeMessage::user("long chat"),
                fire("poll run 1"),
                RuntimeMessage::assistant("still pending after run 1"),
                RuntimeMessage::user("what did the poll find?"),
            ],
        )
        .await;

    rig.execute(None).await.unwrap();

    let calls = rig.calls.lock().await;
    let (messages, _) = calls.last().expect("the model was called");
    let sent = format!("{messages:?}");
    assert!(sent.contains("long chat"), "{sent}");
    assert!(!sent.contains("This is a scheduled run."), "{sent}");
}

//! Durable event replay on `Session`: `events_after` and `events_from`.

use everruns::prelude::*;

fn agent() -> Agent {
    Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("Hello!"))
        .build()
        .expect("valid agent")
}

fn sequences(events: &[SessionEvent]) -> Vec<i32> {
    events.iter().filter_map(SessionEvent::sequence).collect()
}

#[tokio::test]
async fn events_after_returns_dense_durable_sequences() {
    let session = InMemoryEngine::new().create(agent());
    assert!(session.events_after(0).await.unwrap().is_empty());

    session.run("hi").await.unwrap();
    let all = session.events_after(0).await.unwrap();
    assert!(!all.is_empty());
    let seqs = sequences(&all);
    assert_eq!(seqs.len(), all.len(), "replay carries durable events only");
    assert_eq!(seqs, (1..=all.len() as i32).collect::<Vec<_>>());
    assert!(
        all.iter()
            .any(|event| matches!(event.kind, SessionEventKind::TurnCompleted))
    );

    let tail = session.events_after(3).await.unwrap();
    assert_eq!(sequences(&tail), seqs[3..].to_vec());
    assert!(
        session
            .events_after(*seqs.last().unwrap())
            .await
            .unwrap()
            .is_empty()
    );
    // Negative positions read from the start rather than failing.
    assert_eq!(session.events_after(-5).await.unwrap().len(), all.len());
}

#[tokio::test]
async fn events_from_replays_then_continues_live_without_duplicates() {
    let session = InMemoryEngine::new().create(agent());
    session.run("first").await.unwrap();
    let first_turn = session.events_after(0).await.unwrap();
    let last_first = *sequences(&first_turn).last().unwrap();

    let mut stream = session.events_from(2).await.unwrap();
    session.run("second").await.unwrap();
    let total = session.events_after(0).await.unwrap().len() as i32;
    drop(session);

    let mut received = Vec::new();
    while let Some(event) = stream.recv().await.expect("lossless") {
        received.push(event);
    }
    let durable = sequences(&received);
    assert_eq!(
        durable,
        (3..=total).collect::<Vec<_>>(),
        "backlog then live, in order, no gap and no duplicate"
    );
    assert!(total > last_first, "the second turn appended events");
    // Ephemeral deltas from the live turn still pass through.
    assert!(received.iter().any(|event| event.sequence().is_none()
        && matches!(event.kind, SessionEventKind::TextDelta { .. })));
}

#[cfg(feature = "local")]
#[tokio::test]
async fn events_after_survives_a_process_restart() {
    let root = tempfile::tempdir().unwrap();
    let local_agent = || {
        Agent::builder()
            .instructions("Keep durable history.")
            .model(Model::simulated("durable reply"))
            .local(LocalConfig::new(root.path()))
            .build()
            .unwrap()
    };
    let (session_id, before) = {
        let engine = Engine::new();
        let session = engine.create(local_agent());
        session.run("persisted input").await.unwrap();
        (session.session_id(), session.events_after(0).await.unwrap())
    };
    assert!(!before.is_empty());

    let engine = Engine::new();
    engine.attach(session_id, local_agent()).await.unwrap();
    let resumed = engine.resume(session_id).await.unwrap();
    let after_restart = resumed.events_after(0).await.unwrap();
    assert_eq!(sequences(&after_restart), sequences(&before));
    assert_eq!(
        after_restart
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<Vec<_>>(),
        before
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<Vec<_>>()
    );

    let n = sequences(&before)[1];
    let tail = resumed.events_after(n).await.unwrap();
    assert_eq!(sequences(&tail), sequences(&before)[2..].to_vec());

    resumed.run("after restart").await.unwrap();
    let grown = resumed
        .events_after(*sequences(&before).last().unwrap())
        .await
        .unwrap();
    assert!(!grown.is_empty(), "new turn appends after the old tail");
    assert_eq!(
        sequences(&grown)[0],
        sequences(&before).last().unwrap() + 1,
        "sequences stay dense across the restart"
    );
}

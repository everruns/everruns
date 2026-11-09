use everruns::{Agent, Engine, InMemoryEngine, Model, ResumeError};

fn simulated(response: &str) -> Agent {
    Agent::builder()
        .instructions("Reply deterministically.")
        .model(Model::simulated(response))
        .build()
        .expect("valid simulated agent")
}

#[tokio::test]
async fn session_survives_agent_drop_and_resumes_from_engine() {
    let engine = Engine::new();
    let session = {
        let agent = simulated("retained");
        engine.create(agent)
    };
    let session_id = session.session_id();

    assert_eq!(
        session.send_and_wait("first").await.unwrap().response,
        "retained"
    );
    drop(session);

    let resumed = engine.resume(session_id).await.unwrap();
    assert_eq!(resumed.session_id(), session_id);
    assert_eq!(
        resumed.send_and_wait("second").await.unwrap().response,
        "retained"
    );
    assert_eq!(resumed.history().page().await.unwrap().messages.len(), 4);
}

#[tokio::test]
async fn one_engine_isolates_two_agent_snapshots() {
    let engine = InMemoryEngine::new();
    let alpha = engine.create(simulated("alpha"));
    let beta = engine.create(simulated("beta"));

    let (alpha_turn, beta_turn) =
        tokio::join!(alpha.send_and_wait("hello"), beta.send_and_wait("hello"),);
    assert_eq!(alpha_turn.unwrap().response, "alpha");
    assert_eq!(beta_turn.unwrap().response, "beta");
    assert_ne!(alpha.session_id(), beta.session_id());
    assert_eq!(alpha.history().page().await.unwrap().messages.len(), 2);
    assert_eq!(beta.history().page().await.unwrap().messages.len(), 2);
}

#[tokio::test]
async fn another_engine_cannot_resume_process_local_session() {
    let owner = InMemoryEngine::new();
    let session_id = owner.create(simulated("owned")).session_id();

    let error = match InMemoryEngine::new().resume(session_id).await {
        Ok(_) => panic!("another engine must not resume this session"),
        Err(error) => error,
    };
    assert_eq!(error, ResumeError::SessionNotFound { session_id });
}

#[tokio::test]
async fn dropping_session_closes_live_events_while_engine_retains_resume_state() {
    let engine = InMemoryEngine::new();
    let session = engine.create(simulated("retained"));
    let session_id = session.session_id();
    let mut events = session.events();

    drop(session);

    assert!(events.recv().await.unwrap().is_none());
    assert_eq!(
        engine.resume(session_id).await.unwrap().session_id(),
        session_id
    );
}

/// EVE-1344: cancelling the running turn ends only that turn. A message
/// admitted behind it is a later admission, not part of the cancelled turn:
/// it runs as the next turn and its waiter resolves.
#[tokio::test]
async fn cancelling_a_running_turn_keeps_a_queued_message_for_the_next_turn() {
    use everruns::{LlmSimConfig, TurnStopReason};
    use std::time::Duration;
    let config = LlmSimConfig::fixed("done").with_response_delay(Duration::from_millis(500));
    let agent = Agent::builder()
        .instructions("Reply deterministically.")
        .model(Model::simulated_with_config(config))
        .build()
        .expect("valid simulated agent");
    let session = Engine::new().create(agent);

    let first = session.send("first").await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while first.assignment().await.unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("first message starts a turn");
    let queued = session.send("second").await.unwrap();
    first.turn().cancel().await.unwrap();

    let cancelled = tokio::time::timeout(Duration::from_secs(10), first.wait())
        .await
        .expect("cancelled turn resolves")
        .unwrap();
    assert_eq!(cancelled.stop_reason, TurnStopReason::Cancelled);
    let next = tokio::time::timeout(Duration::from_secs(10), queued.wait())
        .await
        .expect("the queued message's waiter resolves")
        .unwrap();
    assert!(next.success, "{:?}", next.error);
    assert_eq!(next.response, "done");
    assert_ne!(next.turn_id, cancelled.turn_id);
}

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

use everruns::{Agent, Engine, Model};
use everruns_core::host::{ActorRunner, InProcessRuntime, SessionLeases, TurnBackend};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn an_engine_uses_its_backend_factory_once_per_live_session() {
    let created = Arc::new(AtomicUsize::new(0));
    let seen = created.clone();
    let engine = Engine::builder()
        .execution_backend(Arc::new(
            move |runtime: InProcessRuntime, leases: Arc<dyn SessionLeases>| {
                seen.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(ActorRunner::new(runtime, leases)) as Arc<dyn TurnBackend>)
            },
        ))
        .build();
    let agent = Agent::builder()
        .instructions("Test the shared durable runtime.")
        .model(Model::simulated("shared runtime"))
        .build()
        .unwrap();
    let first = engine.create(agent.clone());
    assert_eq!(
        first.send_and_wait("hello").await.unwrap().response,
        "shared runtime"
    );
    first.send_and_wait("again").await.unwrap();
    let resumed = engine.resume(first.session_id()).await.unwrap();
    resumed.send_and_wait("same session").await.unwrap();
    assert_eq!(created.load(Ordering::SeqCst), 1);
    engine
        .create(agent)
        .send_and_wait("another session")
        .await
        .unwrap();
    assert_eq!(created.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn backend_construction_errors_reach_the_turn_without_running_the_model() {
    let engine = Engine::builder()
        .execution_backend(Arc::new(
            |_: InProcessRuntime, _: Arc<dyn SessionLeases>| {
                Err(everruns_contracts::error::AgentLoopError::config(
                    "backend unavailable",
                ))
            },
        ))
        .build();
    let session = engine.create(
        Agent::builder()
            .instructions("Test the shared durable runtime.")
            .model(Model::simulated("must not run"))
            .build()
            .unwrap(),
    );
    let error = session.send_and_wait("hello").await.unwrap_err();
    assert!(error.to_string().contains("backend unavailable"));
}

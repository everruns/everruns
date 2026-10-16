use crate::app::Mode;
use crate::hosting::Server;
use everruns_core::host::{
    ActorRunner, HostBackends, InProcessRuntime, SessionLeases, TurnBackend,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn a_serve_host_can_supply_an_engine_with_a_custom_backend() {
    let called = Arc::new(AtomicUsize::new(0));
    let seen = called.clone();
    let engine = everruns::Engine::builder()
        .execution_backend(Arc::new(
            move |runtime: InProcessRuntime, leases: Arc<dyn SessionLeases>| {
                seen.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(ActorRunner::new(runtime, leases)) as Arc<dyn TurnBackend>)
            },
        ))
        .build();
    let server = Server::builder(crate::wire_tests::app(), Mode::Eval)
        .engine(engine)
        .build()
        .unwrap();
    let id = server
        .host
        .create_session(crate::host::NewSession::default())
        .await
        .unwrap();
    let session = server.host.session(&id).await.unwrap();
    let sent = session.send_and_wait("hello").await.unwrap();
    assert!(sent.success);
    assert_eq!(called.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_registered_serve_agent_reconstructs_for_an_external_actor_host() {
    let server = Server::builder(crate::wire_tests::app(), Mode::Eval)
        .build()
        .unwrap();
    let id = everruns::SessionId::new();
    let runtime = server
        .execution_host("tester", id, HostBackends::in_memory())
        .await
        .unwrap();
    let result = runtime.run_turn(id, "hello").await.unwrap();
    assert!(result.success);
    assert_eq!(result.response, "shouted");
    assert!(
        server
            .execution_host("unknown", id, HostBackends::in_memory())
            .await
            .is_err()
    );
}

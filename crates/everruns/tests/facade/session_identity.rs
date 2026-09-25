//! Public Framework acceptance tests for typed session identity.

use everruns::{Agent, InMemoryEngine, Model, SessionId};

fn simulated_agent() -> Agent {
    Agent::builder()
        .instructions("Reply briefly.")
        .model(Model::simulated("ack"))
        .build()
        .expect("valid agent")
}

#[test]
fn session_exposes_a_typed_round_trippable_identity() {
    let session = InMemoryEngine::new().create(simulated_agent());

    let typed = session.session_id();
    let parsed: SessionId = session.id().parse().expect("valid session id");

    assert_eq!(typed, parsed);
    assert_eq!(typed.to_string(), session.id());
}

#[test]
fn independent_sessions_have_distinct_typed_identities() {
    let agent = simulated_agent();

    assert_ne!(
        InMemoryEngine::new().create(agent.clone()).session_id(),
        InMemoryEngine::new().create(agent.clone()).session_id()
    );
}

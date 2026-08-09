//! Public Framework acceptance tests for typed session identity.

use everruns::{Agent, Model, SessionId};

fn simulated_agent() -> Agent {
    Agent::builder()
        .instructions("Reply briefly.")
        .model(Model::simulated("ack"))
        .build()
        .expect("valid agent")
}

#[test]
fn session_exposes_a_typed_round_trippable_identity() {
    let session = simulated_agent().session();

    let typed = session.session_id();
    let parsed: SessionId = session.id().parse().expect("valid session id");

    assert_eq!(typed, parsed);
    assert_eq!(typed.to_string(), session.id());
}

#[test]
fn independent_sessions_have_distinct_typed_identities() {
    let agent = simulated_agent();

    assert_ne!(agent.session().session_id(), agent.session().session_id());
}

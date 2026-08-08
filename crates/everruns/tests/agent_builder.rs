//! Compile/build test for the value-first agent API (EVE-832): a clean program
//! describes an agent importing only `everruns::{Agent, Model}` — no `Harness`,
//! hosted `Agent` record, ids, timestamps, registries, or `PlatformDefinition`.

use everruns::{Agent, Model};

#[test]
fn describes_an_agent_with_only_the_facade() {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("Sure."))
        .name("assistant")
        .build()
        .expect("valid agent");

    // The built value is usable and independent of any backend record.
    let _clone = agent.clone();
}

#[test]
fn missing_model_is_a_typed_error() {
    use everruns::BuildError;
    let err = Agent::builder()
        .instructions("You are concise.")
        .build()
        .unwrap_err();
    assert_eq!(err, BuildError::MissingModel);
}

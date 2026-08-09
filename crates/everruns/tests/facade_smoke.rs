//! Facade smoke test (EVE-830): a clean program that depends only on `everruns`
//! must be able to build one session and run one turn without importing
//! `everruns-core` or `everruns-runtime`.

use everruns::{Agent, Model};

#[tokio::test]
async fn facade_runs_one_simulated_turn() {
    let agent = Agent::builder()
        .instructions("You are a helpful assistant.")
        .model(Model::simulated("4"))
        .build()
        .expect("agent builds");

    let result = agent
        .session()
        .run("What is 2 + 2?")
        .await
        .expect("turn runs");

    assert!(result.success, "turn should succeed: {:?}", result.error);
    assert_eq!(result.response, "4");
}

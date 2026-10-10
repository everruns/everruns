//! A session runs in one process at a time: each turn holds the session's
//! lease in the local database, which every process on the data directory
//! shares.

use std::time::Duration;

use everruns::local::{LocalSessionLeases, SqliteDb};
use everruns::{Agent, Engine, LocalConfig, Model};
use everruns_core::host::SessionLeases;

#[tokio::test]
async fn a_session_another_process_runs_does_not_start_a_turn_here() {
    let data = tempfile::tempdir().unwrap();
    let agent = Agent::builder()
        .instructions("Be concise.")
        .model(Model::simulated("ok"))
        .local(LocalConfig::new(data.path()))
        .build()
        .unwrap();
    let engine = Engine::new();
    let session = engine.create(agent);
    assert_eq!(session.send_and_wait("hi").await.unwrap().response, "ok");

    // Another process on the same data directory takes the lease between
    // turns, as it would to run the session itself.
    let elsewhere =
        LocalSessionLeases::new(SqliteDb::open(data.path().join("local.db")).unwrap()).unwrap();
    let held = elsewhere
        .acquire(session.session_id(), Duration::from_secs(30))
        .await
        .unwrap()
        .expect("the turn gave the lease up when it ended");

    let error = session
        .send_and_wait("again")
        .await
        .expect_err("the session runs elsewhere");
    assert!(
        error.to_string().contains("runs in another process"),
        "{error}"
    );

    elsewhere.release(&held).await.unwrap();
    assert_eq!(session.send_and_wait("again").await.unwrap().response, "ok");
}

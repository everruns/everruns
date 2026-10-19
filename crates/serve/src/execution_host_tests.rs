fn profile_app() -> crate::App {
    let mut app = crate::wire_tests::app();
    let inner = std::sync::Arc::get_mut(&mut app.inner).unwrap();
    inner.channels.clear();
    inner.schedules.clear();
    inner.agents.retain(|agent| !agent.sub);
    app
}

use crate::{app::Mode, host::NewSession, hosting::Server};
use everruns::hosting::{MemoryHosting, SqliteHosting};

#[tokio::test]
async fn serve_registers_ordinary_agents_on_the_same_engine_used_for_admission() {
    let server = Server::builder(profile_app(), Mode::Eval)
        .hosting(MemoryHosting::default())
        .build()
        .unwrap();
    let id = server
        .host
        .create_session(NewSession::default())
        .await
        .unwrap();
    let session = server.host.session(&id).await.unwrap();
    let binding = server
        .engine()
        .session_binding(session.session_id())
        .await
        .unwrap();
    assert_eq!(binding.behavior.agent, "tester");
    assert_eq!(binding.behavior.build_id, server.build_id());
    assert!(server.engine().has_agent_build(&binding.behavior));
    assert!(session.send_and_wait("hello").await.unwrap().success);
}

#[tokio::test]
async fn lost_serve_catalog_is_rebuilt_from_the_session_authority_without_running_a_turn() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authority.db");
    let first = Server::builder(profile_app(), Mode::Eval)
        .hosting(SqliteHosting::open(&path).unwrap())
        .build()
        .unwrap();
    let id = first
        .host
        .create_session(NewSession::default())
        .await
        .unwrap();
    let original = crate::server::session_json(&first.host, &id).await.unwrap();
    drop(first);
    let replacement = Server::builder(profile_app(), Mode::Eval)
        .hosting(SqliteHosting::open(&path).unwrap())
        .build()
        .unwrap();
    assert!(replacement.host.session_row(&id).is_err());
    let reopened = crate::server::session_json(&replacement.host, &id)
        .await
        .unwrap();
    assert_eq!(reopened["id"], original["id"]);
    assert_eq!(reopened["created_at"], original["created_at"]);
    assert_eq!(reopened["status"], "idle");
    assert!(
        replacement
            .host
            .events_after(&id, 0)
            .await
            .unwrap()
            .is_empty()
    );
}

#[test]
fn durable_hosting_refuses_unconfigured_channel_delivery_before_listening() {
    let error = Server::builder(crate::wire_tests::app(), Mode::Eval)
        .hosting(MemoryHosting::default())
        .build()
        .err()
        .unwrap();
    assert!(error.to_string().contains("channel inbox/outbox"));
}

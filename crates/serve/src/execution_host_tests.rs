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

/// EVE-1298: a session a pre-authority release kept under `data_dir/everruns`
/// moves to the native authority on first access, history included, and keeps
/// running there.
#[tokio::test]
async fn pre_authority_sessions_move_to_the_native_authority_on_first_access() {
    let dir = tempfile::tempdir().unwrap();
    // What a release before the native authority left behind: the session in
    // the legacy local store and its row in the serve catalog.
    let (id, before) = {
        let first = Server::builder(profile_app(), Mode::Eval)
            .data_dir(dir.path())
            .build()
            .unwrap();
        let entry = first.app().agent("tester").unwrap().clone();
        let agent = first
            .host
            .build_agent_with(
                &entry,
                None,
                Some(everruns::LocalConfig::new(dir.path().join("everruns"))),
                None,
            )
            .unwrap();
        let session = everruns::Engine::new().create(agent);
        assert!(session.send_and_wait("hello").await.unwrap().success);
        let before = session.events_after(0).await.unwrap().len();
        first
            .host
            .store
            .insert_session(&crate::store::SessionRow {
                id: session.id(),
                agent: "tester".into(),
                build_id: "pre-authority".into(),
                title: None,
                tags: Vec::new(),
                hints: None,
                metadata: None,
                created_at: "2026-10-01T00:00:00.000Z".into(),
                updated_at: "2026-10-01T00:00:00.000Z".into(),
            })
            .unwrap();
        (session.id(), before)
    };
    assert!(before > 0);

    let native = Server::builder(profile_app(), Mode::Eval)
        .data_dir(dir.path())
        .build()
        .unwrap();
    let json = crate::server::session_json(&native.host, &id)
        .await
        .unwrap();
    assert_eq!(json["id"], id);
    let session = native.host.session(&id).await.unwrap();
    let binding = native
        .engine()
        .session_binding(session.session_id())
        .await
        .unwrap();
    assert_eq!(
        binding.behavior.build_id,
        format!("{}:pre-authority", native.build_id())
    );
    assert_eq!(
        native.host.events_after(&id, 0).await.unwrap().len(),
        before
    );
    let turn = native.host.send(&id, "again".into()).await.unwrap();
    turn.wait().await.unwrap();
    let after = native.host.events_after(&id, 0).await.unwrap().len();
    assert!(after > before);
    drop(native);

    // A restarted host reopens the moved session from the authority alone.
    let restarted = Server::builder(profile_app(), Mode::Eval)
        .data_dir(dir.path())
        .build()
        .unwrap();
    restarted.host.session(&id).await.unwrap();
    assert_eq!(
        restarted.host.events_after(&id, 0).await.unwrap().len(),
        after
    );
}

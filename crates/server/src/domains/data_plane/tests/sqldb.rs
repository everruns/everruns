//! A v2 session's SQL databases through the control plane: its API answers
//! from the session's shard, or 503 when the shard is out of reach, never
//! from the v1 store (`session_databases/plane.rs`).

use everruns_contracts::session_sqldb::SessionSqlDbStore as _;
use everruns_contracts::typed_id::SessionId;

use super::*;
use crate::domains::common::{Command as _, Ctx};
use crate::domains::session_databases::{
    CreateSessionDatabaseCmd, DeleteSessionDatabase, GetSessionDatabase, GetSessionDatabaseSchema,
    ListSessionDatabases,
};

/// A session of `ORG` on the data plane, with a database `notes` (two rows)
/// left in the v1 store.
async fn v2_session_with_a_v1_database(cp: &ControlPlane) -> SessionId {
    let session = cp
        .db
        .create_session(crate::storage::test_database::test_session_row(ORG))
        .await
        .unwrap()
        .id;
    let now = chrono::Utc::now();
    cp.db
        .dp_upsert_session_summary(crate::storage::DpSessionSummary {
            session_id: &session.to_string(),
            org_id: ORG,
            data_plane_id: "dp-1",
            agent_id: "agent",
            session_version: 0,
            title: "",
            status: "started",
            turns: 0,
            cost_micros: 0,
            created_at: now,
            last_activity_at: now,
        })
        .await
        .unwrap();
    v1_store(cp)
        .sql_execute(
            session,
            "notes",
            "CREATE TABLE notes (body TEXT); INSERT INTO notes VALUES ('one'), ('two');",
        )
        .await
        .unwrap();
    session
}

fn v1_store(cp: &ControlPlane) -> crate::session_sqldb::PostgresSqlDbStore {
    crate::session_sqldb::PostgresSqlDbStore::new(cp.db.database().clone())
}

fn ctx(cp: &ControlPlane) -> Ctx {
    Ctx::minimal_for_test(everruns_core::Caller::internal(ORG), cp.db.clone(), None)
        .with_sqldb_store(Arc::new(v1_store(cp)))
}

async fn v1_names(cp: &ControlPlane, session: SessionId) -> Vec<String> {
    cp.db
        .list_session_databases(session)
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.name)
        .collect()
}

#[tokio::test]
async fn a_v2_sessions_databases_come_from_its_shard_never_the_v1_store() {
    let (cp, seen, _) = control_plane().await;
    let session = v2_session_with_a_v1_database(&cp).await;
    let id = session.to_string();
    let ctx = ctx(&cp);

    // The v1 store's `notes` is not the session's: its shard holds none yet.
    let listed = ListSessionDatabases {
        session_id: id.clone(),
    }
    .execute(&ctx)
    .await
    .unwrap();
    assert!(listed.is_empty(), "{listed:?}");

    let created = CreateSessionDatabaseCmd {
        session_id: id.clone(),
        name: "fresh".into(),
    }
    .execute(&ctx)
    .await
    .unwrap();
    assert_eq!(created.name, "fresh");
    assert_eq!(seen.lock().databases[&id].len(), 1, "created on the shard");
    let again = CreateSessionDatabaseCmd {
        session_id: id.clone(),
        name: "fresh".into(),
    }
    .execute(&ctx)
    .await
    .expect_err("a taken name");
    assert_eq!(again.status(), axum::http::StatusCode::CONFLICT);

    let got = GetSessionDatabase {
        session_id: id.clone(),
        name: "fresh".into(),
    }
    .execute(&ctx)
    .await
    .unwrap();
    assert_eq!(got.size_bytes, 0);
    let schema = GetSessionDatabaseSchema {
        session_id: id.clone(),
        name: "fresh".into(),
    }
    .execute(&ctx)
    .await
    .unwrap();
    assert!(schema.tables.is_empty());
    let missing = GetSessionDatabase {
        session_id: id.clone(),
        name: "notes".into(),
    }
    .execute(&ctx)
    .await
    .expect_err("notes is only in the v1 store");
    assert_eq!(missing.status(), axum::http::StatusCode::NOT_FOUND);

    DeleteSessionDatabase {
        session_id: id.clone(),
        name: "fresh".into(),
    }
    .execute(&ctx)
    .await
    .unwrap();
    assert!(seen.lock().databases[&id].is_empty());

    // The v1 store was neither read nor written.
    assert_eq!(v1_names(&cp, session).await, ["notes"]);
}

#[tokio::test]
async fn a_v2_session_whose_shard_is_out_of_reach_answers_503_not_the_v1_store() {
    let (cp, _, _) = control_plane().await;
    let session = v2_session_with_a_v1_database(&cp).await;
    // The data plane's gateway is gone (the edge down).
    let dead = "http://127.0.0.1:9";
    cp.db
        .dp_upsert_data_plane("dp-1", dead, dead)
        .await
        .unwrap();
    let ctx = ctx(&cp);

    let listed = ListSessionDatabases {
        session_id: session.to_string(),
    }
    .execute(&ctx)
    .await
    .expect_err("no answer from the v1 store");
    assert_eq!(listed.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    let created = CreateSessionDatabaseCmd {
        session_id: session.to_string(),
        name: "fresh".into(),
    }
    .execute(&ctx)
    .await
    .expect_err("nothing written to the v1 store");
    assert_eq!(
        created.status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(v1_names(&cp, session).await, ["notes"]);
}

//! Claiming a leased session's Engine binding, on a real database.

use std::time::Duration;

use everruns_contracts::typed_id::SessionId;

use super::*;
use crate::storage::test_database::test_session_row;
use crate::storage::{CreateEventRow, ENGINE_HOSTING_FLAG};

const ORG: i64 = 2;
const HOLDER: &str = "worker-a";

/// A session on a fresh harness, leased to [`HOLDER`]; its fence.
async fn leased(
    db: &StorageBackend,
    edit: impl FnOnce(&mut crate::storage::CreateSessionRow),
) -> (SessionId, i64) {
    let harness = db
        .create_harness(
            ORG,
            crate::storage::CreateHarnessRow {
                localizations: Default::default(),
                name: format!("Test harness {}", uuid::Uuid::new_v4().simple()),
                display_name: None,
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: Some("You are helpful.".to_string()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                is_built_in: false,
                network_access: None,
                embedder_metadata: serde_json::json!({}),
            },
        )
        .await
        .expect("create harness");
    let mut row = test_session_row(ORG);
    row.harness_id = Some(harness.id);
    edit(&mut row);
    let id = db.create_session(row).await.expect("create session").id;
    db.mark_session_actor_runnable(id, ORG, 1).await.unwrap();
    let lease = db
        .lease_session_actors(HOLDER, 10, Duration::from_secs(10), 100)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.session_id == id)
        .expect("leased");
    (id, lease.fence)
}

async fn enrol(db: &StorageBackend) {
    db.replace_org_feature_flags(
        ORG,
        &std::collections::HashMap::from([(ENGINE_HOSTING_FLAG.to_string(), true)]),
    )
    .await
    .unwrap();
}

/// Without enrolment a session stays on the worker's host; with it, the
/// first claim binds it to a key with a published revision, and the next
/// claim keeps that binding.
#[tokio::test]
async fn enrolment_binds_a_session_at_its_first_claim() {
    let db = StorageBackend::test_database();
    let (id, fence) = leased(&db, |_| {}).await;
    assert_eq!(
        claim(&db, id, HOLDER, fence).await.unwrap(),
        EngineClaim::Legacy
    );

    enrol(&db).await;
    let claimed = claim(&db, id, HOLDER, fence).await.unwrap();
    let EngineClaim::Bound(first) = claimed else {
        let session = db.get_session(ORG, id).await.unwrap().unwrap();
        panic!("bound: {claimed:?}, overrides {:?}", overrides(&session));
    };
    assert!(first.binding.agent_key.starts_with("ak_"));
    assert_eq!(first.binding.revision, "r1");
    let definition = db
        .engine_agent_definition(&first.binding.agent_key, "r1")
        .await
        .unwrap();
    assert!(definition.is_some(), "the bound revision is published");
    let EngineClaim::Bound(second) = claim(&db, id, HOLDER, fence).await.unwrap() else {
        panic!("still bound");
    };
    assert_eq!(second.binding, first.binding);
    assert_eq!(
        claim(&db, id, "worker-b", fence).await.unwrap(),
        EngineClaim::Lost
    );
}

/// A session with an override the Engine cannot express stays on the
/// worker's host, and so does one whose turn is open.
#[tokio::test]
async fn overrides_and_open_turns_stay_on_the_worker_host() {
    let db = StorageBackend::test_database();
    enrol(&db).await;
    let (prompted, fence) = leased(&db, |row| row.system_prompt = Some("Be brief.".into())).await;
    assert_eq!(
        claim(&db, prompted, HOLDER, fence).await.unwrap(),
        EngineClaim::Legacy
    );

    let (open, fence) = leased(&db, |_| {}).await;
    db.create_event(CreateEventRow {
        session_id: open,
        event_type: "turn.started".into(),
        ts: chrono::Utc::now(),
        context: serde_json::json!({}),
        data: serde_json::json!({}),
        metadata: None,
        tags: None,
    })
    .await
    .unwrap();
    assert_eq!(
        claim(&db, open, HOLDER, fence).await.unwrap(),
        EngineClaim::Legacy
    );
}

/// A harness with an empty prompt does not compile: its sessions stay on the
/// worker's host rather than failing.
#[tokio::test]
async fn an_agent_that_does_not_compile_stays_on_the_worker_host() {
    let db = StorageBackend::test_database();
    enrol(&db).await;
    let harness = uuid::Uuid::new_v4();
    db.create_test_harness(ORG, harness).await;
    let (id, fence) = leased(&db, |row| {
        row.harness_id = Some(everruns_contracts::typed_id::HarnessId::from_uuid(harness))
    })
    .await;
    assert_eq!(
        claim(&db, id, HOLDER, fence).await.unwrap(),
        EngineClaim::Legacy
    );
}

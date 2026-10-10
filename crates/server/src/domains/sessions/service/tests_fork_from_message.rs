//! Tests: branching a session from a message (`up_to_message_id`).

use super::*;
use crate::domains::common::Command;
use crate::domains::harnesses::types::CreateHarnessRequest;
use crate::kernel_imports::Caller;
use crate::storage::StorageBackend;

use super::tests_support::*;

async fn parent_with_two_turns() -> (Arc<StorageBackend>, SessionService, Caller, Session) {
    let db = Arc::new(StorageBackend::test_database());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(1);
    let ctx = test_ctx(caller.clone(), db.clone()).await;
    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "branch-harness".to_string(),
        display_name: Some("Branch Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: None,
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();
    let parent = session_service
        .create(
            &caller,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap();

    // Two turns: user msg_a1 -> agent msg_a2, then user msg_b1 -> agent msg_b2.
    for (event_type, message_id) in [
        ("turn.started", None),
        ("input.message", Some("msg_a1")),
        ("output.message.completed", Some("msg_a2")),
        ("turn.completed", None),
        ("turn.started", None),
        ("input.message", Some("msg_b1")),
        ("output.message.completed", Some("msg_b2")),
        ("turn.completed", None),
    ] {
        let data = match message_id {
            Some(id) => serde_json::json!({ "message": { "id": id } }),
            None => serde_json::json!({}),
        };
        db.create_event(CreateEventRow {
            session_id: parent.id,
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context: serde_json::json!({}),
            data,
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }
    (db, session_service, caller, parent)
}

async fn event_types(db: &StorageBackend, session_id: SessionId) -> Vec<String> {
    let mut events = db
        .list_events(session_id, None, None, &[], &[], None, None)
        .await
        .unwrap();
    events.sort_by_key(|event| event.sequence);
    events.into_iter().map(|event| event.event_type).collect()
}

#[tokio::test]
async fn branching_from_a_message_keeps_history_through_its_turn() {
    let (db, service, caller, parent) = parent_with_two_turns().await;
    let parent_types = event_types(&db, parent.id).await;

    // An agent message and the user message of the same turn cut at the same
    // place: the end of the first turn.
    for message_id in ["msg_a2", "msg_a1"] {
        let child = service
            .fork(
                &caller,
                parent.id,
                ForkOverrides {
                    up_to_message_id: Some(message_id.to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(event_types(&db, child.id).await, parent_types[..4].to_vec());
        let cut = db
            .fork_cut_sequence(parent.id, message_id)
            .await
            .unwrap();
        assert_eq!(child.forked_from_sequence, cut);
    }

    // The last message keeps everything.
    let child = service
        .fork(
            &caller,
            parent.id,
            ForkOverrides {
                up_to_message_id: Some("msg_b2".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(event_types(&db, child.id).await, parent_types);
}

#[tokio::test]
async fn branching_from_an_unknown_message_creates_nothing() {
    let (db, service, caller, parent) = parent_with_two_turns().await;
    let before = db.count_sessions_for_org(1).await.unwrap();

    let error = service
        .fork(
            &caller,
            parent.id,
            ForkOverrides {
                up_to_message_id: Some("msg_missing".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();

    assert!(error.downcast_ref::<ResourceNotFoundError>().is_some());
    assert_eq!(db.count_sessions_for_org(1).await.unwrap(), before);
}

//! A subagent a shard starts (`subagents.rs`), on a real database: its
//! feed record gives it a record under its parent with the shard's id,
//! marked as the data plane's, and only for the org's own v2 sessions.

use super::*;
use crate::domains::data_plane::subagents;
use everruns_contracts::dataplane::SubagentOrigin;
use everruns_contracts::typed_id::SessionId;

#[tokio::test]
async fn a_shards_subagent_gets_a_record_under_its_parent() {
    let (cp, _, _) = control_plane().await;
    let harness = harness(&cp.db, ORG).await;
    let mut row = crate::storage::test_database::test_session_row(ORG);
    row.harness_id = Some(harness);
    let parent = cp.db.create_session(row).await.unwrap();
    let origin = SubagentOrigin {
        parent_session_id: parent.id.to_string(),
        harness_id: harness.to_string(),
        agent_id: None,
        title: Some("Researcher".into()),
        locale: None,
    };
    let child = SessionId::new();

    let now = chrono::Utc::now().timestamp_millis();
    // Not yet the data plane's: held back while fresh, so the feed sends it
    // again; skipped once its parent has had long enough to arrive.
    assert!(
        subagents::record(&cp, ORG, &child.to_string(), &origin, now)
            .await
            .is_err()
    );
    subagents::record(
        &cp,
        ORG,
        &child.to_string(),
        &origin,
        now - subagents::PARENT_WAIT_MS - 1,
    )
    .await
    .unwrap();
    assert!(cp.db.get_session(ORG, child).await.unwrap().is_none());

    sessions::created(&cp, ORG, parent.id, harness, None, &[], None)
        .await
        .unwrap();
    // Another org cannot record children of this org's session.
    subagents::record(&cp, 3, &child.to_string(), &origin, now)
        .await
        .unwrap();
    assert!(cp.db.get_session(3, child).await.unwrap().is_none());

    subagents::record(&cp, ORG, &child.to_string(), &origin, now)
        .await
        .unwrap();
    let record = cp.db.get_session(ORG, child).await.unwrap().unwrap();
    assert_eq!(record.id, child, "the child keeps the id it runs under");
    assert_eq!(record.parent_session_id, Some(parent.id));
    assert_eq!(record.title.as_deref(), Some("Researcher"));
    assert_eq!(record.harness_id, Some(harness));
    assert!(
        sessions::runs_on_data_plane(&cp, ORG, child).await.unwrap(),
        "the child's messages never reach the platform's workers"
    );
    // The Memory its own record resolves to (here the org's team Memory).
    let mounts = crate::domains::data_plane::memory::mounts(&cp, child)
        .await
        .unwrap();
    assert_eq!(mounts, vec!["/shared".to_owned()]);

    // A replayed record changes nothing.
    subagents::record(&cp, ORG, &child.to_string(), &origin, now)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_grandchild_that_arrives_before_its_parent_waits_for_it() {
    // A grandchild started on one shard reaches the feed before its parent,
    // a child another shard started (codex review, 2026-11-24).
    let (cp, _, _) = control_plane().await;
    let harness = harness(&cp.db, ORG).await;
    let mut row = crate::storage::test_database::test_session_row(ORG);
    row.harness_id = Some(harness);
    let root = cp.db.create_session(row).await.unwrap();
    sessions::created(&cp, ORG, root.id, harness, None, &[], None)
        .await
        .unwrap();
    let origin = |parent: SessionId| SubagentOrigin {
        parent_session_id: parent.to_string(),
        harness_id: harness.to_string(),
        agent_id: None,
        title: None,
        locale: None,
    };
    let (child, grandchild) = (SessionId::new(), SessionId::new());
    let now = chrono::Utc::now().timestamp_millis();

    let early = subagents::record(&cp, ORG, &grandchild.to_string(), &origin(child), now).await;
    assert!(early.is_err(), "held back, so the feed sends it again");
    assert!(cp.db.get_session(ORG, grandchild).await.unwrap().is_none());

    subagents::record(&cp, ORG, &child.to_string(), &origin(root.id), now)
        .await
        .unwrap();
    subagents::record(&cp, ORG, &grandchild.to_string(), &origin(child), now)
        .await
        .unwrap();
    let record = cp.db.get_session(ORG, grandchild).await.unwrap().unwrap();
    assert_eq!(record.parent_session_id, Some(child));
}

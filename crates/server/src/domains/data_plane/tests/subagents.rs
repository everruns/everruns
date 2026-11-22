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

    // Not yet the data plane's: skipped.
    subagents::record(&cp, ORG, &child.to_string(), &origin)
        .await
        .unwrap();
    assert!(cp.db.get_session(ORG, child).await.unwrap().is_none());

    sessions::created(&cp, ORG, parent.id, harness, None, &[], None)
        .await
        .unwrap();
    // Another org cannot record children of this org's session.
    subagents::record(&cp, 3, &child.to_string(), &origin)
        .await
        .unwrap();
    assert!(cp.db.get_session(3, child).await.unwrap().is_none());

    subagents::record(&cp, ORG, &child.to_string(), &origin)
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
    assert_eq!(mounts, vec!["/team".to_owned()]);

    // A replayed record changes nothing.
    subagents::record(&cp, ORG, &child.to_string(), &origin)
        .await
        .unwrap();
}

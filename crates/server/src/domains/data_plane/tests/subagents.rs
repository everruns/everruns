//! A subagent a cell starts (`subagents.rs`), on a real database: the
//! `subagent_record` host call gives it a record under its parent, marked
//! as the data plane's, and only for the org's own v2 sessions.

use super::*;
use crate::domains::data_plane::subagents;

fn internal_ctx(cp: &ControlPlane, org_id: i64) -> crate::domains::common::Ctx {
    crate::domains::common::Ctx::new(
        everruns_core::Caller::internal(org_id),
        cp.db.clone(),
        Arc::new(crate::services::CapabilityService::new(cp.db.clone(), None)),
        cp.encryption.clone(),
        Arc::new(everruns_core::DefaultPermissionResolver),
    )
}

#[tokio::test]
async fn a_cells_subagent_gets_a_record_under_its_parent() {
    let (cp, _, _) = control_plane().await;
    let harness = harness(&cp.db, ORG).await;
    let mut row = crate::storage::test_database::test_session_row(ORG);
    row.harness_id = Some(harness);
    let parent = cp.db.create_session(row).await.unwrap();
    let params = json!({
        "parent_session_id": parent.id.to_string(),
        "harness_id": harness.to_string(),
        "title": "Researcher",
    });

    // Not yet the data plane's: refused.
    let ctx = internal_ctx(&cp, ORG);
    assert!(subagents::record(&cp, &ctx, &params).await.is_err());

    sessions::created(&cp, ORG, parent.id, harness, None, &[], None)
        .await
        .unwrap();
    let answer = subagents::record(&cp, &ctx, &params).await.unwrap();
    let child: everruns_contracts::typed_id::SessionId =
        answer["session_id"].as_str().unwrap().parse().unwrap();
    // The Memory its own record resolves to (here the org's team Memory).
    let mounts = crate::domains::data_plane::memory::mounts(&cp, child)
        .await
        .unwrap();
    assert_eq!(mounts, vec!["/team".to_owned()]);
    assert_eq!(answer["memory"], json!(mounts));
    let record = cp.db.get_session(ORG, child).await.unwrap().unwrap();
    assert_eq!(record.parent_session_id, Some(parent.id));
    assert_eq!(record.title.as_deref(), Some("Researcher"));
    assert_eq!(record.harness_id, Some(harness));
    assert!(
        sessions::runs_on_data_plane(&cp, ORG, child).await.unwrap(),
        "the child's messages never reach the platform's workers"
    );

    // Another org cannot record children of this org's session.
    assert!(
        subagents::record(&cp, &internal_ctx(&cp, 3), &params)
            .await
            .is_err()
    );
}

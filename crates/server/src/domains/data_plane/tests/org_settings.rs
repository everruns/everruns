//! An org's settings pushed to its data plane (`org_settings.rs`): each
//! setting an `org_setting` connection, each search account's key a sealed
//! secret, synced whole and only when something changed.

use everruns_contracts::dataplane::{
    EGRESS_SETTING, ORG_SETTING, TOOL_APPROVAL_SETTING, WEB_SEARCH_SETTING, web_search_key_secret,
};

use super::*;
use crate::domains::data_plane::org_settings;

async fn setting(cp: &ControlPlane, id: &str) -> Option<(i64, dp::Connection)> {
    let item = cp.db.dp_item(ORG, "connection", id).await.unwrap()?;
    let Some(dp::ConfigChange::Connection(connection)) = dp::ConfigChange::parse(&item.body?)
    else {
        panic!("{id} is a connection");
    };
    Some((item.revision, connection))
}

async fn key(cp: &ControlPlane, account: uuid::Uuid) -> Option<Vec<u8>> {
    crate::domains::data_plane::providers::sealed_key(cp, ORG, &web_search_key_secret(account))
        .await
        .unwrap()
}

#[tokio::test]
async fn an_orgs_settings_and_search_keys_travel_with_its_config() {
    let (cp, _, _) = control_plane().await;
    cp.db
        .set_org_egress_grants(ORG, Some(true), Some(false))
        .await
        .unwrap();
    cp.db
        .set_org_egress_allowlist_patterns(ORG, &["api.example.com".to_owned()])
        .await
        .unwrap();
    let encrypted = cp.encryption().unwrap().encrypt_string("bs-key-1").unwrap();
    let account = cp
        .db
        .create_organization_connection(crate::storage::CreateOrganizationConnectionRow {
            org_id: ORG,
            name: "Search".into(),
            provider: "brave_search".into(),
            access_token_encrypted: Some(encrypted),
            ..Default::default()
        })
        .await
        .unwrap()
        .id;
    // Not a search account: its key never travels.
    let sandbox = cp
        .db
        .create_organization_connection(crate::storage::CreateOrganizationConnectionRow {
            org_id: ORG,
            name: "Sandbox".into(),
            provider: "daytona".into(),
            access_token_encrypted: Some(cp.encryption().unwrap().encrypt_string("dtn").unwrap()),
            ..Default::default()
        })
        .await
        .unwrap()
        .id;

    org_settings::sync(&cp, ORG).await.unwrap();

    let (_, web_search) = setting(&cp, WEB_SEARCH_SETTING).await.expect("pushed");
    assert_eq!(web_search.kind, ORG_SETTING);
    assert_eq!(web_search.token_secret_id, "");
    assert_eq!(
        web_search.settings["accounts"][0]["id"],
        json!(account.to_string())
    );
    let (_, egress) = setting(&cp, EGRESS_SETTING).await.expect("pushed");
    assert_eq!(
        egress.settings,
        json!({ "patterns": ["api.example.com"], "open": false })
    );
    let (approval_revision, _) = setting(&cp, TOOL_APPROVAL_SETTING).await.expect("pushed");
    assert_eq!(key(&cp, account).await.as_deref(), Some(&b"bs-key-1"[..]));
    assert_eq!(key(&cp, sandbox).await, None);

    // Nothing changed: nothing is written again.
    org_settings::sync(&cp, ORG).await.unwrap();
    assert_eq!(
        setting(&cp, TOOL_APPROVAL_SETTING).await.unwrap().0,
        approval_revision
    );

    // A revoked grant pushes no patterns; a removed account loses its key.
    cp.db
        .set_org_egress_grants(ORG, Some(false), None)
        .await
        .unwrap();
    assert!(
        cp.db
            .delete_organization_connection(ORG, account)
            .await
            .unwrap()
    );
    org_settings::sync(&cp, ORG).await.unwrap();
    assert_eq!(
        setting(&cp, EGRESS_SETTING).await.unwrap().1.settings,
        json!({ "patterns": [], "open": false })
    );
    assert_eq!(key(&cp, account).await, None);
    assert_eq!(
        setting(&cp, WEB_SEARCH_SETTING).await.unwrap().1.settings["accounts"],
        json!([])
    );
}

#[tokio::test]
async fn a_v1_org_gets_no_settings() {
    let (cp, _, _) = control_plane().await;
    org_settings::sync(&cp, 3).await.unwrap();
    assert!(cp.db.dp_items_live(3).await.unwrap().is_empty());
}

/// Review fix: a settings change counts in the same transaction, so a sync
/// that failed after it is caught up by the dispatcher instead of leaving
/// the old policy (open egress) on the cells.
#[tokio::test]
async fn a_settings_change_is_owed_a_sync_until_one_covers_it() {
    let (cp, _, _) = control_plane().await;
    org_settings::sync(&cp, ORG).await.unwrap();
    assert!(cp.db.dp_orgs_settings_behind(64).await.unwrap().is_empty());

    // Open egress, and its sync "fails": nothing pushed, but it is owed.
    cp.db
        .set_org_egress_grants(ORG, None, Some(true))
        .await
        .unwrap();
    let behind = cp.db.dp_orgs_settings_behind(64).await.unwrap();
    assert_eq!(behind.iter().map(|b| b.0).collect::<Vec<_>>(), vec![ORG]);
    let (_, egress) = setting(&cp, EGRESS_SETTING).await.unwrap();
    assert_eq!(egress.settings["open"], json!(false), "not pushed yet");

    // The dispatcher's pass syncs it, then nothing is owed.
    assert_eq!(org_settings::sync_owed(&cp).await.unwrap(), 1);
    let (_, egress) = setting(&cp, EGRESS_SETTING).await.unwrap();
    assert_eq!(egress.settings["open"], json!(true));
    assert!(cp.db.dp_orgs_settings_behind(64).await.unwrap().is_empty());
    assert_eq!(org_settings::sync_owed(&cp).await.unwrap(), 0);

    // A change to one of the org's connections is owed a sync too.
    cp.db
        .create_organization_connection(crate::storage::CreateOrganizationConnectionRow {
            org_id: ORG,
            name: "Search".into(),
            provider: "brave_search".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(cp.db.dp_orgs_settings_behind(64).await.unwrap().len(), 1);
}

/// Review fix: a write host call runs once per call id; a duplicate gets
/// the first run's answer, or "unknown" while that run has not finished,
/// and another org never sees it.
#[tokio::test]
async fn a_write_host_calls_receipt_answers_its_duplicates() {
    use crate::storage::HostCallReceipt;
    let (cp, _, _) = control_plane().await;
    let take = |org| {
        let db = cp.db.clone();
        async move {
            db.dp_take_host_call_receipt("host-call:c1", org, "memory_file")
                .await
                .unwrap()
        }
    };
    assert_eq!(take(ORG).await, HostCallReceipt::New);
    assert_eq!(take(ORG).await, HostCallReceipt::Unknown);
    cp.db
        .dp_answer_host_call_receipt("host-call:c1", &json!({ "sealed": 1 }))
        .await
        .unwrap();
    assert_eq!(
        take(ORG).await,
        HostCallReceipt::Answered(json!({ "sealed": 1 }))
    );
    assert_eq!(take(3).await, HostCallReceipt::Unknown);
    assert_eq!(cp.db.dp_expire_host_call_receipts(1).await.unwrap(), 0);
}

/// Review fix: two syncs of one org never overlap, so one that read the
/// tables before a revocation cannot publish after the revocation's own
/// sync. A sync waits for the one holding the org's lock, then reads the
/// tables as they are by then.
#[tokio::test]
async fn an_orgs_syncs_run_one_at_a_time_and_read_after_the_one_before() {
    let (cp, _, _) = control_plane().await;
    cp.db
        .set_org_egress_grants(ORG, None, Some(true))
        .await
        .unwrap();
    // Another server's sync holds the org.
    let held = cp
        .db
        .advisory_xact_lock_polling(
            "dp-org-settings-sync",
            &ORG.to_string(),
            std::time::Duration::from_secs(5),
        )
        .await
        .unwrap();
    let waiting = tokio::spawn({
        let cp = cp.clone();
        async move { org_settings::sync(&cp, ORG).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(!waiting.is_finished(), "waits for the sync before it");
    // Revoked while it waits: it reads the tables after, not before.
    cp.db
        .set_org_egress_grants(ORG, None, Some(false))
        .await
        .unwrap();
    drop(held);
    waiting.await.unwrap().unwrap();
    assert_eq!(
        setting(&cp, EGRESS_SETTING).await.unwrap().1.settings["open"],
        json!(false)
    );
    assert!(cp.db.dp_orgs_settings_behind(64).await.unwrap().is_empty());
}

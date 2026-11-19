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

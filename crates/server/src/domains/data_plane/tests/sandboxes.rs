//! A v2 session's managed sandbox (`sandboxes.rs`), on a real database:
//! the data plane's create carries the pinned sandbox, and its provider key
//! travels as a sealed org secret, sealed again only when it changed.

use everruns_contracts::dataplane::sandbox_key_secret;
use everruns_contracts::session_sandbox::{
    SessionSandboxCredential, SessionSandboxCredentialSource,
};
use everruns_contracts::typed_id::VirtualUserId;

use super::*;
use crate::domains::data_plane::providers::sealed_key;
use crate::domains::data_plane::sandboxes;
use crate::storage::CreateVirtualUserConnectionRow;

/// A person of `ORG` with a Daytona key.
async fn person(cp: &ControlPlane, key: &str) -> VirtualUserId {
    let id = VirtualUserId::from_uuid(uuid::Uuid::now_v7());
    cp.db
        .create_virtual_user(crate::storage::CreateVirtualUserRow {
            org_id: ORG,
            id,
            usage: "end_user".into(),
            name: "Sandbox owner".into(),
            description: None,
            avatar_url: None,
            locale: None,
            timezone: None,
        })
        .await
        .unwrap();
    set_key(cp, id, key).await;
    id
}

async fn set_key(cp: &ControlPlane, owner: VirtualUserId, key: &str) {
    cp.db
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: owner,
            provider: "daytona".into(),
            connection_type: "api_key".into(),
            provider_user_id: None,
            provider_username: None,
            access_token_encrypted: Some(test_encryption().encrypt_string(key).unwrap()),
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
}

/// A session of `ORG` with a Daytona sandbox pinned, its key `owner`'s.
async fn pinned_session(
    cp: &ControlPlane,
    harness: everruns_contracts::typed_id::HarnessId,
    owner: VirtualUserId,
) -> everruns_contracts::typed_id::SessionId {
    let mut row = crate::storage::test_database::test_session_row(ORG);
    row.harness_id = Some(harness);
    let id = cp.db.create_session(row).await.unwrap().id;
    let template: crate::domains::sandbox_templates::record::SandboxTemplateSpec =
        serde_json::from_value(json!({"target": {"kind": "managed", "provider": "daytona"}}))
            .unwrap();
    let mut spec = crate::domains::sandbox_templates::resolution::resolve_spec(&template).unwrap();
    spec.target.credential = SessionSandboxCredential {
        source: SessionSandboxCredentialSource::SessionUser,
        virtual_user_id: Some(owner.uuid()),
        connection_id: None,
    };
    cp.db
        .pin_primary_sandbox(id, "managed", &spec)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn a_v2_sessions_sandbox_goes_with_the_create_and_its_key_sealed() {
    let (cp, seen, _) = control_plane().await;
    let harness = harness(&cp.db, ORG).await;
    let owner = person(&cp, "dtn-key-1").await;
    let secret = sandbox_key_secret("daytona", Some(owner.uuid()), None);

    let first = pinned_session(&cp, harness, owner).await;
    sessions::created(&cp, ORG, first, harness, None, &[], None)
        .await
        .unwrap();
    {
        let seen = seen.lock();
        let (_, body, applied) = seen.sessions.last().expect("created on the data plane");
        let sandbox = &body["sandbox"];
        assert_eq!(sandbox["ref"], "session_sandbox", "{body}");
        assert_eq!(sandbox["config"]["provider"], "daytona");
        assert_eq!(
            sandbox["config"]["credential"]["virtual_user_id"],
            json!(owner.uuid())
        );
        assert!(
            !body.to_string().contains("dtn-key-1"),
            "the key is not in the create: {body}"
        );
        // The sealed key reached the data plane before the create.
        let delivered = seen
            .updates
            .iter()
            .filter(|(_, u)| u.version <= *applied)
            .flat_map(|(_, u)| u.changes.iter())
            .any(|c| c["kind"] == "Secret" && c.to_string().contains(&secret));
        assert!(delivered, "the key's secret was delivered first");
    }
    assert_eq!(
        sealed_key(&cp, ORG, &secret).await.unwrap().as_deref(),
        Some(b"dtn-key-1".as_slice())
    );

    // Another session with the same key: the org's config does not move.
    let revision = |cp: &ControlPlane| {
        let db = cp.db.clone();
        let secret = secret.clone();
        async move {
            db.dp_item(ORG, "secret", &secret)
                .await
                .unwrap()
                .unwrap()
                .revision
        }
    };
    let before = revision(&cp).await;
    let second = pinned_session(&cp, harness, owner).await;
    sessions::created(&cp, ORG, second, harness, None, &[], None)
        .await
        .unwrap();
    assert_eq!(revision(&cp).await, before);

    // A rotated key is sealed again with the next session.
    set_key(&cp, owner, "dtn-key-2").await;
    let third = pinned_session(&cp, harness, owner).await;
    sessions::created(&cp, ORG, third, harness, None, &[], None)
        .await
        .unwrap();
    assert_eq!(revision(&cp).await, before + 1);
    assert_eq!(
        sealed_key(&cp, ORG, &secret).await.unwrap().as_deref(),
        Some(b"dtn-key-2".as_slice())
    );

    // A session with no sandbox sends none.
    let mut row = crate::storage::test_database::test_session_row(ORG);
    row.harness_id = Some(harness);
    let plain = cp.db.create_session(row).await.unwrap().id;
    assert_eq!(sandboxes::pinned(&cp, ORG, plain).await.unwrap(), None);
}

#[test]
fn a_published_package_carries_no_sandbox_of_its_own() {
    let mut package = json!({
        "name": "builder",
        "sandbox_policy": { "templates": ["build"] },
        "capabilities": [
            { "ref": "session_sandbox", "config": { "provider": "daytona" } },
            "sandbox_desktop",
            { "ref": "session_file_system" },
        ],
    });
    sandboxes::strip(&mut package);
    assert_eq!(package.get("sandbox_policy"), None);
    assert_eq!(
        package["capabilities"],
        json!([{ "ref": "session_file_system" }])
    );
    // A package with no capabilities keeps none: a null would not parse.
    let mut bare = json!({ "name": "plain" });
    sandboxes::strip(&mut bare);
    assert_eq!(bare, json!({ "name": "plain" }));
}

#[tokio::test]
async fn pausing_a_v2_sandbox_uses_the_pinned_one() {
    let (cp, _, _) = control_plane().await;
    let harness = harness(&cp.db, ORG).await;
    let owner = person(&cp, "dtn-key-1").await;
    let session = pinned_session(&cp, harness, owner).await;
    let config = crate::domains::session_sandbox::plane::config(&cp.db, session)
        .await
        .unwrap()
        .expect("a managed sandbox is pinned");
    assert_eq!(config.provider, "daytona");

    // A session with no sandbox has nothing to pause.
    let mut row = crate::storage::test_database::test_session_row(ORG);
    row.harness_id = Some(harness);
    let plain = cp.db.create_session(row).await.unwrap().id;
    assert!(
        crate::domains::session_sandbox::plane::config(&cp.db, plain)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn an_idle_v2_sessions_sandbox_is_swept_once_per_idle_spell() {
    use crate::domains::session_sandbox::plane::{idle_handled, idle_on_data_planes};
    let (cp, _, _) = control_plane().await;
    let harness = harness(&cp.db, ORG).await;
    let owner = person(&cp, "dtn-key-1").await;
    let session = pinned_session(&cp, harness, owner).await;
    let summary = |at: chrono::DateTime<chrono::Utc>, version: i64| {
        let db = cp.db.clone();
        let id = session.to_string();
        async move {
            db.dp_upsert_session_summary(crate::storage::DpSessionSummary {
                session_id: &id,
                org_id: ORG,
                data_plane_id: "dp-1",
                agent_id: "ak_x",
                session_version: version,
                title: "",
                status: "idle",
                turns: version,
                cost_micros: 0,
                created_at: at,
                last_activity_at: at,
            })
            .await
            .unwrap();
        }
    };
    let pool = cp.db.pool().clone();
    let idle = || idle_on_data_planes(&pool, std::time::Duration::ZERO, 50);

    // A turn that just ended is not idle yet.
    summary(chrono::Utc::now(), 1).await;
    assert!(idle().await.unwrap().is_empty());

    // An hour later it is, once.
    let ended = chrono::Utc::now() - chrono::Duration::hours(1);
    summary(ended, 2).await;
    let swept = idle().await.unwrap();
    assert_eq!(swept.len(), 1);
    assert_eq!((swept[0].0, swept[0].1), (session.uuid(), ORG));
    idle_handled(&pool, session.uuid(), swept[0].2)
        .await
        .unwrap();
    assert!(idle().await.unwrap().is_empty());

    // Another turn, another idle spell.
    summary(ended + chrono::Duration::minutes(10), 3).await;
    assert_eq!(idle().await.unwrap().len(), 1);
}

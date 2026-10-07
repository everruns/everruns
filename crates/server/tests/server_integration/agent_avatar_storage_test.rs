//! Dual-backend conformance for agent avatar storage.
//!
//! Run with: cargo test -p everruns-server --test server_integration agent_avatar_storage_test:: -- --test-threads=1

use crate::repository_conformance_test::agent_input;
use crate::test_harness::get_database_url;

use everruns_contracts::typed_id::HarnessId;
use everruns_core::DEFAULT_ORG_ID;
use everruns_server::org_init;
use everruns_server::storage::{AgentAvatarVariantInput, Database, SetAgentAvatar, StorageBackend};
use sqlx::PgPool;
use uuid::Uuid;

fn avatar(agent_id: Uuid, org_id: i64, marker: u8) -> SetAgentAvatar {
    SetAgentAvatar {
        org_id,
        agent_id,
        source: "upload".to_string(),
        variants: ["source.png", "square-64.png"]
            .into_iter()
            .map(|variant| AgentAvatarVariantInput {
                variant: variant.to_string(),
                content_type: "image/png".to_string(),
                data: vec![marker; 4],
            })
            .collect(),
    }
}

async fn run(backend: &StorageBackend, label: &str, harness_id: HarnessId) {
    // Unique per run: the PostgreSQL database outlives a test run.
    let name = format!("avatar-owner-{label}-{}", Uuid::now_v7().simple());
    let (agent, _) = backend
        .upsert_agent_by_name(DEFAULT_ORG_ID, agent_input(name.clone(), harness_id))
        .await
        .expect("create agent");
    let agent_uuid = agent.id.uuid();
    assert_eq!(agent.avatar_id, None);

    // Another org cannot attach an avatar to this agent.
    assert_eq!(
        backend
            .set_agent_avatar(avatar(agent_uuid, DEFAULT_ORG_ID + 999, 1))
            .await
            .unwrap(),
        None
    );

    let mut selected = avatar(agent_uuid, DEFAULT_ORG_ID, 1);
    selected.source = "preset:watchers-bracket".to_string();
    let first = backend
        .set_agent_avatar(selected)
        .await
        .unwrap()
        .expect("first avatar");
    assert_eq!(
        backend
            .get_agent_avatar_source(DEFAULT_ORG_ID, agent_uuid)
            .await
            .unwrap()
            .as_deref(),
        Some("preset:watchers-bracket")
    );
    assert!(
        backend
            .get_agent_avatar_source(DEFAULT_ORG_ID + 999, agent_uuid)
            .await
            .unwrap()
            .is_none()
    );
    let row = backend
        .get_agent(DEFAULT_ORG_ID, agent.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.avatar_id,
        Some(first),
        "{label}: agent points at avatar"
    );
    let variant = backend
        .get_agent_avatar_variant(first, "square-64.png")
        .await
        .unwrap()
        .expect("variant stored");
    assert_eq!(variant.content_type, "image/png");
    assert_eq!(variant.data, vec![1; 4]);
    assert!(
        backend
            .get_agent_avatar_variant(first, "circle-64.png")
            .await
            .unwrap()
            .is_none()
    );

    // A replacement gets a new id; the old one stops resolving.
    let second = backend
        .set_agent_avatar(avatar(agent_uuid, DEFAULT_ORG_ID, 2))
        .await
        .unwrap()
        .expect("second avatar");
    assert_ne!(first, second);
    assert!(
        backend
            .get_agent_avatar_variant(first, "square-64.png")
            .await
            .unwrap()
            .is_none(),
        "{label}: replaced avatar is gone"
    );
    assert_eq!(
        backend
            .get_agent_avatar_variant(second, "square-64.png")
            .await
            .unwrap()
            .unwrap()
            .data,
        vec![2; 4]
    );

    // Re-saving the agent definition keeps its avatar.
    backend
        .upsert_agent_by_name(DEFAULT_ORG_ID, agent_input(name, harness_id))
        .await
        .unwrap();
    let row = backend
        .get_agent(DEFAULT_ORG_ID, agent.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.avatar_id, Some(second), "{label}: upsert keeps avatar");

    assert!(
        !backend
            .clear_agent_avatar(DEFAULT_ORG_ID + 999, agent_uuid)
            .await
            .unwrap()
    );
    assert!(
        backend
            .clear_agent_avatar(DEFAULT_ORG_ID, agent_uuid)
            .await
            .unwrap()
    );
    assert!(
        !backend
            .clear_agent_avatar(DEFAULT_ORG_ID, agent_uuid)
            .await
            .unwrap()
    );
    let row = backend
        .get_agent(DEFAULT_ORG_ID, agent.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.avatar_id, None);
    assert!(
        backend
            .get_agent_avatar_variant(second, "square-64.png")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn in_memory_agent_avatar_storage() {
    let backend = StorageBackend::test_database();
    run(&backend, "memory", HarnessId::from_uuid(Uuid::nil())).await;
}

#[tokio::test]
async fn postgres_agent_avatar_storage() {
    let pool = PgPool::connect(&get_database_url())
        .await
        .expect("Failed to connect to PostgreSQL");
    let backend = StorageBackend::from_database(Database::new(pool));
    org_init::initialize_org_harnesses(&backend, DEFAULT_ORG_ID)
        .await
        .expect("initialize built-in harnesses");
    let harness_id = org_init::generic_harness_id(&backend, DEFAULT_ORG_ID)
        .await
        .expect("generic harness id");
    run(&backend, "postgres", harness_id).await;
}

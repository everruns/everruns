//! Platform Chat starter uniqueness and archived identity.

use crate::test_harness;

use everruns_provider::typed_id::PrincipalId;
use everruns_server::org_init;
use everruns_server::storage::{CreatePrincipalRow, CreateSessionRow, Database, StorageBackend};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

const TEST_ORG_ID: i64 = 1;

async fn create_test_pool() -> PgPool {
    PgPool::connect(&test_harness::get_database_url())
        .await
        .expect("connect to test database")
}

async fn create_test_backend() -> StorageBackend {
    StorageBackend::Postgres(Database::new(create_test_pool().await))
}

async fn create_test_principal(backend: &StorageBackend, org_id: i64) -> PrincipalId {
    backend
        .create_principal(CreatePrincipalRow {
            id: PrincipalId::new(),
            org_id,
            kind: "system".to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: None,
            metadata: json!({"source": "platform_chat_starter_test"}),
        })
        .await
        .expect("create principal")
        .id
}

#[tokio::test]
async fn platform_chat_starter_is_unique_per_owner_even_after_archive() {
    let backend = create_test_backend().await;
    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    org_init::initialize_org_harnesses(&backend, TEST_ORG_ID)
        .await
        .expect("initialize built-in harnesses");
    let platform_chat = backend
        .get_harness_by_name(TEST_ORG_ID, "platform-chat")
        .await
        .expect("load Platform Chat harness")
        .expect("Platform Chat harness is seeded");
    let starter = CreateSessionRow {
        source: everruns_platform::SessionSource::Chat,
        workspace_id: None,
        org_id: TEST_ORG_ID,
        app_id: None,
        endpoint_id: None,
        harness_id: Some(platform_chat.id),
        agent_id: None,
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: None,
        owner_principal_id,
        resolved_owner_user_id: None,
        title: Some("Platform Chat".to_string()),
        locale: None,
        tags: vec!["chat".to_string()],
        model_id: None,
        capabilities: json!([]),
        tools: json!([]),
        mcp_servers: json!({}),
        system_prompt: None,
        initial_files: json!([]),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: None,
        budget_root_session_id: None,
    };

    let first = backend
        .create_session(starter.clone())
        .await
        .expect("starter");
    assert!(first.tags.contains(&"platform-chat-starter".to_string()));
    sqlx::query("UPDATE sessions SET archived_at = NOW() WHERE id = $1")
        .bind(first.id.uuid())
        .execute(&create_test_pool().await)
        .await
        .expect("archive starter");
    sqlx::query("UPDATE sessions SET tags = ARRAY['chat']::text[] WHERE id = $1")
        .bind(first.id.uuid())
        .execute(&create_test_pool().await)
        .await
        .expect("update starter tags");
    let kept_marker: bool = sqlx::query_scalar(
        "SELECT 'platform-chat-starter' = ANY(tags) FROM sessions WHERE id = $1",
    )
    .bind(first.id.uuid())
    .fetch_one(&create_test_pool().await)
    .await
    .expect("read starter tags");
    assert!(
        kept_marker,
        "session updates cannot remove starter identity"
    );

    let duplicate = backend.create_session(starter.clone()).await;
    assert!(
        duplicate.is_err(),
        "archiving must not permit a second starter"
    );

    let mut ordinary = starter;
    ordinary.title = None;
    backend
        .create_session(ordinary)
        .await
        .expect("ordinary platform chat threads remain allowed");
}

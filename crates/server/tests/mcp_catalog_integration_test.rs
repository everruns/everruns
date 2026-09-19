//! PostgreSQL integration tests for the MCP catalog and personal connections.

mod test_harness;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use everruns_server::org_init;
use everruns_server::storage::{
    CreateAgentRow, CreateMcpServerRow, CreateOrganizationRow, CreateUserConnectionRow,
    CreateUserRow, Database, StorageBackend, UpdateMcpServer,
};
use test_harness::get_database_url;

const TEST_ORG_ID: i64 = 1;

async fn create_test_pool() -> PgPool {
    PgPool::connect(&get_database_url())
        .await
        .expect("connect to PostgreSQL")
}

async fn create_test_backend() -> StorageBackend {
    StorageBackend::Postgres(Database::new(create_test_pool().await))
}

async fn ensure_test_harness_id(
    backend: &StorageBackend,
) -> everruns_provider::typed_id::HarnessId {
    org_init::initialize_org_harnesses(backend, TEST_ORG_ID)
        .await
        .expect("initialize built-in harnesses");
    org_init::generic_harness_id(backend, TEST_ORG_ID)
        .await
        .expect("generic harness id")
}

async fn create_test_org(backend: &StorageBackend, name: &str) -> i64 {
    backend
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: name.to_string(),
            created_by: None,
        })
        .await
        .expect("create organization")
        .org_id
}

async fn create_test_user(
    backend: &StorageBackend,
    label: &str,
) -> everruns_server::storage::UserRow {
    backend
        .create_user(CreateUserRow {
            email: format!("{label}-{}@example.com", Uuid::now_v7()),
            name: format!("Test {label}"),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .expect("create user")
}

async fn create_catalog_usage_agent(
    backend: &StorageBackend,
    org_id: i64,
    harness_id: everruns_provider::typed_id::HarnessId,
    name: &str,
    display_name: &str,
    mcp_servers: serde_json::Value,
) -> everruns_server::storage::AgentRow {
    backend
        .create_agent(
            org_id,
            CreateAgentRow {
                public_id: everruns_provider::typed_id::AgentId::new().to_string(),
                name: format!("{name}-{}", &Uuid::now_v7().to_string()[..8]),
                display_name: Some(display_name.to_string()),
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: "Catalog usage test".to_string(),
                default_model_id: None,
                harness_id,
                tags: vec![],
                initial_files: json!([]),
                tools: json!([]),
                mcp_servers,
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                is_built_in: false,
            },
        )
        .await
        .expect("create catalog usage agent")
}

#[tokio::test]
async fn mcp_catalog_usage_counts_active_agents_once_and_stays_org_scoped() {
    let backend = create_test_backend().await;
    let other_org = create_test_org(&backend, "MCP catalog isolation").await;
    org_init::initialize_org_harnesses(&backend, other_org)
        .await
        .expect("initialize other org harnesses");
    let org1_harness = ensure_test_harness_id(&backend).await;
    let org2_harness = org_init::generic_harness_id(&backend, other_org)
        .await
        .expect("other org generic harness");

    let server = backend
        .create_mcp_server(
            TEST_ORG_ID,
            CreateMcpServerRow {
                name: format!("catalog-{}", Uuid::now_v7().simple()),
                description: None,
                url: "https://catalog.example/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .expect("create catalog server");
    let unused = backend
        .create_mcp_server(
            TEST_ORG_ID,
            CreateMcpServerRow {
                name: format!("unused-{}", Uuid::now_v7().simple()),
                description: None,
                url: "https://unused.example/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .expect("create unused catalog server");
    let other_server = backend
        .create_mcp_server(
            other_org,
            CreateMcpServerRow {
                name: server.name.clone(),
                description: None,
                url: "https://other.example/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .expect("create other org catalog server");
    let reference = format!("catalog:{}", server.name);

    create_catalog_usage_agent(
        &backend,
        TEST_ORG_ID,
        org1_harness,
        "alpha-agent",
        "Alpha agent",
        json!({
            "primary": { "use": reference },
            "duplicate-alias": { "use": reference },
        }),
    )
    .await;
    create_catalog_usage_agent(
        &backend,
        TEST_ORG_ID,
        org1_harness,
        "beta-agent",
        "Beta agent",
        json!({ "primary": { "use": reference } }),
    )
    .await;
    let archived = create_catalog_usage_agent(
        &backend,
        TEST_ORG_ID,
        org1_harness,
        "archived-agent",
        "Archived agent",
        json!({ "primary": { "use": reference } }),
    )
    .await;
    backend
        .delete_agent(TEST_ORG_ID, archived.id)
        .await
        .expect("archive agent");
    let deleted = create_catalog_usage_agent(
        &backend,
        TEST_ORG_ID,
        org1_harness,
        "deleted-agent",
        "Deleted agent",
        json!({ "primary": { "use": reference } }),
    )
    .await;
    sqlx::query(
        "UPDATE agents SET status = 'deleted', deleted_at = NOW() WHERE org_id = $1 AND id = $2",
    )
    .bind(TEST_ORG_ID)
    .bind(deleted.id.uuid())
    .execute(&create_test_pool().await)
    .await
    .expect("mark agent deleted");
    create_catalog_usage_agent(
        &backend,
        other_org,
        org2_harness,
        "other-org-agent",
        "Other org agent",
        json!({ "primary": { "use": reference } }),
    )
    .await;

    let org1_usage = backend
        .list_mcp_server_agent_usage(TEST_ORG_ID)
        .await
        .expect("list org 1 catalog usage");
    assert_eq!(
        org1_usage
            .iter()
            .find(|row| row.mcp_server_id == server.id)
            .map(|row| row.used_by_agents),
        Some(2)
    );
    assert_eq!(
        org1_usage
            .iter()
            .find(|row| row.mcp_server_id == unused.id)
            .map(|row| row.used_by_agents),
        Some(0)
    );

    let bounded = backend
        .get_mcp_server_agent_names(TEST_ORG_ID, server.id, 1)
        .await
        .expect("load bounded agent names");
    assert_eq!(bounded.total_count, 2);
    assert_eq!(bounded.agent_names, vec!["Alpha agent"]);

    let org2_usage = backend
        .list_mcp_server_agent_usage(other_org)
        .await
        .expect("list org 2 catalog usage");
    assert_eq!(
        org2_usage
            .iter()
            .find(|row| row.mcp_server_id == other_server.id)
            .map(|row| row.used_by_agents),
        Some(1)
    );
}

#[tokio::test]
async fn user_mcp_connections_are_user_and_org_scoped_and_include_tombstones() {
    let backend = create_test_backend().await;
    let other_org = create_test_org(&backend, "MCP connection isolation").await;
    let current_user = create_test_user(&backend, "mcp-current").await;
    let other_user = create_test_user(&backend, "mcp-other").await;

    let active = backend
        .create_mcp_server(
            TEST_ORG_ID,
            CreateMcpServerRow {
                name: format!("active-{}", Uuid::now_v7().simple()),
                description: None,
                url: "https://active.example/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .expect("create active server");
    let deleted = backend
        .create_mcp_server(
            TEST_ORG_ID,
            CreateMcpServerRow {
                name: format!("deleted-{}", Uuid::now_v7().simple()),
                description: None,
                url: "https://deleted.example/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .expect("create deleted server");
    backend
        .update_mcp_server(
            TEST_ORG_ID,
            deleted.id.uuid(),
            UpdateMcpServer {
                status: Some("deleted".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("delete server");
    let other_org_server = backend
        .create_mcp_server(
            other_org,
            CreateMcpServerRow {
                name: format!("other-{}", Uuid::now_v7().simple()),
                description: None,
                url: "https://other.example/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .expect("create other org server");

    let insert_connection =
        |user_id: Uuid, server_id: everruns_provider::typed_id::McpServerId, username: &str| {
            let backend = &backend;
            let username = username.to_string();
            async move {
                backend
                    .upsert_user_connection(CreateUserConnectionRow {
                        user_id,
                        provider: everruns_core::mcp_oauth_provider_id_for_uuid(server_id.uuid()),
                        connection_type: "oauth".to_string(),
                        provider_user_id: None,
                        provider_username: Some(username),
                        access_token_encrypted: Some(vec![1, 2, 3]),
                        refresh_token_encrypted: None,
                        scopes: Some("read write".to_string()),
                        expires_at: None,
                        installation_id: None,
                        provider_metadata: None,
                    })
                    .await
                    .expect("insert user MCP connection")
            }
        };
    let active_connection = insert_connection(current_user.id, active.id, "current").await;
    insert_connection(current_user.id, deleted.id, "current-deleted").await;
    insert_connection(current_user.id, other_org_server.id, "current-other-org").await;
    insert_connection(other_user.id, active.id, "other-user").await;

    let rows = backend
        .list_user_mcp_connections(TEST_ORG_ID, current_user.id)
        .await
        .expect("list current user MCP connections");
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| row.server_id == active.id));
    assert!(
        rows.iter()
            .any(|row| row.server_id == deleted.id && row.server_status == "deleted")
    );
    assert!(rows.iter().all(|row| {
        row.server_id != other_org_server.id
            && row.provider_username.as_deref() != Some("other-user")
    }));

    assert!(
        backend
            .delete_user_connection(current_user.id, &active_connection.provider)
            .await
            .expect("delete current user connection")
    );
    assert!(
        !backend
            .delete_user_connection(current_user.id, &active_connection.provider)
            .await
            .expect("repeat current user connection deletion")
    );
}

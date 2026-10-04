use crate::test_harness::get_database_url;
use everruns_durable::UpdateField;
use everruns_server::{
    org_init,
    storage::{
        CreateAgentRow, CreateOrganizationRow, Database, StorageBackend, UpdateAgent,
        UpdateOrganizationSettings,
    },
};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn test_harness_levels_upgrade_pins_legacy_agents_atomically() {
    let backend = StorageBackend::Postgres(Database::new(
        sqlx::PgPool::connect(&get_database_url()).await.unwrap(),
    ));
    let org = backend
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Harness upgrade".into(),
            created_by: None,
        })
        .await
        .unwrap();
    org_init::initialize_org_harnesses(&backend, org.org_id)
        .await
        .unwrap();
    let generic = backend
        .get_harness_by_name(org.org_id, "generic")
        .await
        .unwrap()
        .unwrap();
    let conversation = backend
        .get_harness_by_name(org.org_id, "conversation")
        .await
        .unwrap()
        .unwrap();
    backend
        .update_harness(
            org.org_id,
            generic.id,
            everruns_server::storage::UpdateHarness {
                tags: Some(vec!["generic".into(), "default".into(), "built-in".into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    backend
        .patch_organization_settings(
            org.org_id,
            UpdateOrganizationSettings {
                default_harness_id: UpdateField::Set(generic.id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let agent = backend
        .create_agent(
            org.org_id,
            CreateAgentRow {
                public_id: everruns_contracts::typed_id::AgentId::new().to_string(),
                name: "legacy-inherited".into(),
                display_name: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: "Preserve tools".into(),
                default_model_id: None,
                harness_id: generic.id,
                tags: vec![],
                initial_files: json!([]),
                tools: json!([]),
                mcp_servers: json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    backend
        .update_agent(
            org.org_id,
            agent.id,
            UpdateAgent {
                harness_source: Some("organization_default".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        backend
            .migrate_generic_default(org.org_id, conversation.id)
            .await
            .unwrap()
    );
    let pinned = backend
        .get_agent(org.org_id, agent.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pinned.harness_source, "explicit");
    assert_eq!(pinned.harness_id, generic.id);
    assert_eq!(
        backend
            .get_organization_settings(org.org_id)
            .await
            .unwrap()
            .unwrap()
            .default_harness_id,
        Some(conversation.id)
    );
    assert!(
        !backend
            .migrate_generic_default(org.org_id, conversation.id)
            .await
            .unwrap()
    );
    // A later deliberate legacy default is not another pending upgrade.
    backend
        .patch_organization_settings(
            org.org_id,
            UpdateOrganizationSettings {
                default_harness_id: UpdateField::Set(generic.id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    org_init::initialize_org_harnesses(&backend, org.org_id)
        .await
        .unwrap();
    assert_eq!(
        backend
            .get_organization_settings(org.org_id)
            .await
            .unwrap()
            .unwrap()
            .default_harness_id,
        Some(generic.id)
    );
    let base = backend
        .get_harness_by_name(org.org_id, "base")
        .await
        .unwrap()
        .unwrap();
    backend
        .patch_organization_settings(
            org.org_id,
            UpdateOrganizationSettings {
                default_harness_id: UpdateField::Set(base.id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        !backend
            .migrate_generic_default(org.org_id, conversation.id)
            .await
            .unwrap()
    );
    assert_eq!(
        backend
            .get_organization_settings(org.org_id)
            .await
            .unwrap()
            .unwrap()
            .default_harness_id,
        Some(base.id)
    );
}

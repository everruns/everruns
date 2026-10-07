use super::*;
use crate::storage::UpdateField;
use crate::storage::{
    CreateAgentRow, CreateOrganizationRow, UpdateAgent, UpdateOrganizationSettings,
};
use everruns_contracts::typed_id::AgentId;
use everruns_core::{DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID};
use serde_json::json;

#[tokio::test]
async fn new_built_in_names_preserve_colliding_custom_harnesses() {
    let db = StorageBackend::test_database();
    db.create_organization_with_id(
        DEFAULT_ORG_ID,
        CreateOrganizationRow {
            public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            name: "Custom collision".into(),
            created_by: None,
        },
    )
    .await
    .unwrap();
    let custom_input = |name: &str, parent_harness_id| CreateHarnessRow {
        name: name.into(),
        display_name: None,
        icon: None,
        description: Some("Keep this worker".into()),
        intro_markdown: None,
        short_description: None,
        starters: json!([]),
        system_prompt: Some("Custom instructions".into()),
        parent_harness_id,
        default_model_id: None,
        tags: vec!["custom".into()],
        initial_files: json!([]),
        mcp_servers: json!({}),
        is_built_in: false,
        network_access: None,
        embedder_metadata: json!({}),
    };
    let custom = db
        .create_harness(DEFAULT_ORG_ID, custom_input("worker", None))
        .await
        .unwrap();
    let child = db
        .create_harness(DEFAULT_ORG_ID, custom_input("child", Some(custom.id)))
        .await
        .unwrap();
    db.create_harness(DEFAULT_ORG_ID, custom_input("worker-custom", None))
        .await
        .unwrap();
    db.set_harness_capabilities(
        custom.id.uuid(),
        vec![("current_time".into(), 0, json!({}))],
    )
    .await
    .unwrap();
    db.patch_organization_settings(
        DEFAULT_ORG_ID,
        UpdateOrganizationSettings {
            default_harness_id: UpdateField::Set(custom.id),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();
    initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();
    let preserved = db
        .get_harness(DEFAULT_ORG_ID, custom.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(preserved.name, "worker-custom-2");
    assert_eq!(preserved.display_name.as_deref(), Some("worker"));
    assert_eq!(preserved.system_prompt, custom.system_prompt);
    assert_eq!(preserved.tags, custom.tags);
    assert!(!preserved.is_built_in);
    assert_eq!(
        db.get_harness_capabilities(custom.id.uuid()).await.unwrap()[0].capability_id,
        "current_time"
    );
    assert_eq!(
        db.get_harness(DEFAULT_ORG_ID, child.id)
            .await
            .unwrap()
            .unwrap()
            .parent_harness_id,
        Some(custom.id)
    );
    assert_eq!(
        db.get_organization_settings(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .unwrap()
            .default_harness_id,
        Some(custom.id)
    );
    let built_in = db
        .get_harness_by_name(DEFAULT_ORG_ID, "worker")
        .await
        .unwrap()
        .unwrap();
    assert!(built_in.is_built_in);
    assert_ne!(built_in.id, custom.id);
}

#[tokio::test]
async fn upgrade_pins_inherited_agents_without_changing_legacy_tools() {
    let db = StorageBackend::test_database();
    db.create_organization_with_id(
        DEFAULT_ORG_ID,
        CreateOrganizationRow {
            public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            name: "Upgrade".into(),
            created_by: None,
        },
    )
    .await
    .unwrap();
    let legacy = vec![
        crate::harnesses::built_in_harnesses()
            .into_iter()
            .find(|h| h.name == "base")
            .unwrap(),
    ];
    // Reproduce the old org: Base, Generic default, with already-created agents.
    let mut generic = crate::harnesses::built_in_harnesses()
        .into_iter()
        .find(|h| h.name == "generic")
        .unwrap();
    generic.roles = vec![BuiltInHarnessRole::Default];
    generic.tags.push("default".into());
    let mut legacy = legacy;
    legacy.push(generic);
    initialize_org_harnesses_with_definitions(&db, DEFAULT_ORG_ID, &legacy)
        .await
        .unwrap();
    let generic = db
        .get_harness_by_name(DEFAULT_ORG_ID, "generic")
        .await
        .unwrap()
        .unwrap();
    let base = db
        .get_harness_by_name(DEFAULT_ORG_ID, "base")
        .await
        .unwrap()
        .unwrap();
    let legacy_caps = db
        .get_harness_capabilities(generic.id.uuid())
        .await
        .unwrap();
    let mut ids = vec![];
    for (name, harness, source) in [
        ("inherited", base.id, "organization_default"),
        ("explicit", base.id, "explicit"),
    ] {
        let id = AgentId::new();
        let agent = db
            .create_agent(
                DEFAULT_ORG_ID,
                CreateAgentRow {
                    public_id: id.to_string(),
                    name: name.into(),
                    display_name: None,
                    description: None,
                    intro_markdown: None,
                    short_description: None,
                    starters: json!([]),
                    system_prompt: "Test".into(),
                    default_model_id: None,
                    harness_id: harness,
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
        db.update_agent(
            DEFAULT_ORG_ID,
            agent.id,
            UpdateAgent {
                harness_source: Some(source.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        ids.push(agent.id);
    }
    initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();
    let conversation = db
        .get_harness_by_name(DEFAULT_ORG_ID, "conversation")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        db.get_organization_settings(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .unwrap()
            .default_harness_id,
        Some(conversation.id)
    );
    let inherited = db.get_agent(DEFAULT_ORG_ID, ids[0]).await.unwrap().unwrap();
    assert_eq!(inherited.harness_id, generic.id);
    assert_eq!(inherited.harness_source, "explicit");
    assert_eq!(
        db.get_agent(DEFAULT_ORG_ID, ids[1])
            .await
            .unwrap()
            .unwrap()
            .harness_id,
        base.id
    );
    let after = db
        .get_harness_by_name(DEFAULT_ORG_ID, "generic")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.id, generic.id);
    assert_eq!(after.status, "active");
    assert!(after.tags.iter().any(|tag| tag == "deprecated"));
    let caps = db.get_harness_capabilities(after.id.uuid()).await.unwrap();
    assert_eq!(
        caps.iter()
            .map(|c| (&c.capability_id, &c.config))
            .collect::<Vec<_>>(),
        legacy_caps
            .iter()
            .map(|c| (&c.capability_id, &c.config))
            .collect::<Vec<_>>()
    );
    db.patch_organization_settings(
        DEFAULT_ORG_ID,
        UpdateOrganizationSettings {
            default_harness_id: UpdateField::Set(generic.id),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();
    assert_eq!(
        db.get_organization_settings(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .unwrap()
            .default_harness_id,
        Some(generic.id)
    );
    let again = initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();
    assert_eq!(again.created, 0);
    assert_eq!(again.updated, 0);
    db.patch_organization_settings(
        DEFAULT_ORG_ID,
        UpdateOrganizationSettings {
            default_harness_id: UpdateField::Set(base.id),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();
    assert_eq!(
        db.get_organization_settings(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .unwrap()
            .default_harness_id,
        Some(base.id)
    );
}

#[tokio::test]
async fn worker_assignment_checks_inherited_high_risk_capabilities() {
    use crate::domains::agents::commands::check_harness_assignment;
    use crate::domains::common::Ctx;
    use everruns_core::{Caller, OrgRole};
    use std::sync::Arc;
    let db = Arc::new(StorageBackend::test_database());
    db.create_organization_with_id(
        DEFAULT_ORG_ID,
        CreateOrganizationRow {
            public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            name: "Permissions".into(),
            created_by: None,
        },
    )
    .await
    .unwrap();
    initialize_org_harnesses(&db, DEFAULT_ORG_ID).await.unwrap();
    let worker = db
        .get_harness_by_name(DEFAULT_ORG_ID, "worker")
        .await
        .unwrap()
        .unwrap();
    let base = db
        .get_harness_by_name(DEFAULT_ORG_ID, "conversation")
        .await
        .unwrap()
        .unwrap();
    let mut caller = Caller::internal(DEFAULT_ORG_ID);
    caller.role = OrgRole::Member;
    caller.is_internal = false;
    caller.user_id = Some(Uuid::now_v7());
    let ctx = Ctx::minimal_for_test(caller, db.clone(), None);
    assert!(check_harness_assignment(&ctx, base.id).await.is_ok());
    let worker_base = db
        .get_harness_by_name(DEFAULT_ORG_ID, "worker-base")
        .await
        .unwrap()
        .unwrap();
    let shell_error = check_harness_assignment(&ctx, worker_base.id)
        .await
        .unwrap_err();
    assert!(shell_error.to_string().contains("bashkit_shell"));
    let error = check_harness_assignment(&ctx, worker.id).await.unwrap_err();
    assert!(error.to_string().contains("subagents"));
    let ctx = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db, None);
    assert!(check_harness_assignment(&ctx, worker.id).await.is_ok());
}

#[tokio::test]
async fn startup_prepares_harness_upgrade_before_background_seed() {
    let db = std::sync::Arc::new(StorageBackend::test_database());
    db.create_organization_with_id(
        DEFAULT_ORG_ID,
        CreateOrganizationRow {
            public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            name: "Startup upgrade".into(),
            created_by: None,
        },
    )
    .await
    .unwrap();
    let mut legacy = crate::harnesses::built_in_harnesses();
    legacy.retain(|h| h.name == "base" || h.name == "generic");
    let generic = legacy.iter_mut().find(|h| h.name == "generic").unwrap();
    generic.roles = vec![BuiltInHarnessRole::Default];
    generic.tags.push("default".into());
    initialize_org_harnesses_with_definitions(&db, DEFAULT_ORG_ID, &legacy)
        .await
        .unwrap();
    let task = crate::seed::prepare_seed_task(
        db.clone(),
        &crate::auth::config::AuthConfig::default(),
        crate::platform::oss_host_composition_for_grade(everruns_core::DeploymentGrade::Dev),
        crate::harnesses::built_in_harnesses(),
        None,
    )
    .await
    .unwrap();
    task.abort();
    let conversation = db
        .get_harness_by_name(DEFAULT_ORG_ID, "conversation")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        db.get_organization_settings(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .unwrap()
            .default_harness_id,
        Some(conversation.id)
    );
}

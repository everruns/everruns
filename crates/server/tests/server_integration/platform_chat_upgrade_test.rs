//! Promotion preserves the canonical identity and consolidates preview bindings.

use super::session_row_fixture::base_session_row;
use crate::test_harness;
use everruns_contracts::typed_id::{AgentId, PrincipalId};
use everruns_server::domains::session_files::{CreateFileInput, WorkspaceFileService};
use everruns_server::storage::UpdateField;
use everruns_server::{
    setup::org_init,
    storage::{CreateAgentTriggerRow, CreateMemoryFileRow, Database, StorageBackend, *},
};
use serde_json::json;
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

async fn verify_upgrade(db: Arc<StorageBackend>, opted_in: bool) {
    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Platform Chat upgrade".to_string(),
            created_by: None,
        })
        .await
        .unwrap();
    let org_id = org.org_id;
    org_init::initialize_org_harnesses(&db, org_id)
        .await
        .unwrap();
    let canonical = db
        .create_harness(
            org_id,
            CreateHarnessRow {
                name: "platform-chat".into(),
                display_name: Some("Platform Chat".into()),
                icon: None,
                description: None,
                intro_markdown: Some("Legacy intro".into()),
                short_description: None,
                starters: json!([]),
                system_prompt: Some("Legacy prompt".into()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: json!([]),
                mcp_servers: json!({}),
                network_access: None,
                embedder_metadata: json!({}),
                is_built_in: true,
            },
        )
        .await
        .unwrap();
    db.update_harness(
        org_id,
        canonical.id,
        UpdateHarness {
            system_prompt: Some(Some("Legacy prompt".to_string())),
            intro_markdown: Some(Some("Legacy intro".to_string())),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    db.set_harness_capabilities(
        canonical.id.uuid(),
        vec![("platform".to_string(), 0, json!({}))],
    )
    .await
    .unwrap();
    let preview = db
        .create_harness(
            org_id,
            CreateHarnessRow {
                name: "platform-chat-v2".to_string(),
                display_name: Some("Platform Chat v2".to_string()),
                icon: canonical.icon.clone(),
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: canonical.system_prompt.clone(),
                parent_harness_id: canonical.parent_harness_id,
                default_model_id: None,
                tags: vec!["preview".to_string()],
                initial_files: json!([]),
                mcp_servers: json!({}),
                is_built_in: true,
                network_access: None,
                embedder_metadata: json!({}),
            },
        )
        .await
        .unwrap();
    db.replace_org_feature_flags(
        org_id,
        &HashMap::from([("platform_chat_v2".to_string(), opted_in)]),
    )
    .await
    .unwrap();
    let owner = db
        .create_principal(CreatePrincipalRow {
            id: PrincipalId::new(),
            org_id,
            kind: "system".to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: None,
            metadata: json!({}),
        })
        .await
        .unwrap()
        .id;
    let starter = db
        .create_session(CreateSessionRow {
            harness_id: Some(canonical.id),
            owner_principal_id: owner,
            title: Some("Platform Chat".to_string()),
            tags: vec!["chat".to_string(), "platform-chat-starter".to_string()],
            ..base_session_row(org_id)
        })
        .await
        .unwrap();
    let preview_session = db
        .create_session(CreateSessionRow {
            harness_id: Some(preview.id),
            owner_principal_id: owner,
            title: Some("Existing preview conversation".to_string()),
            ..base_session_row(org_id)
        })
        .await
        .unwrap();
    let agent = db
        .create_agent(
            org_id,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "preview-agent".to_string(),
                display_name: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: "Operator".to_string(),
                default_model_id: None,
                harness_id: preview.id,
                tags: vec![],
                initial_files: json!([]),
                tools: json!([]),
                mcp_servers: json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                communication: Default::default(),
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    let app = db
        .create_app(
            org_id,
            CreateAppRow {
                public_id: format!("app_{}", Uuid::now_v7().simple()),
                name: "Preview app".to_string(),
                description: None,
                harness_id: preview.id.uuid(),
                agent_id: Some(agent.id.uuid()),
                virtual_user_id: None,
                owner_principal_id: owner,
                resolved_owner_user_id: None,
                channel_type: None,
                channel_config: json!({}),
                channel_config_encrypted: None,
            },
        )
        .await
        .unwrap();
    let trigger = db
        .create_agent_trigger(CreateAgentTriggerRow {
            org_id,
            id: everruns_contracts::typed_id::TriggerId::new(),
            agent_id: agent.id,
            trigger_type: "schedule".to_string(),
            ingress_id: None,
            config: json!({}),
            config_encrypted: None,
            enabled: false,
            durable_schedule_id: None,
            execution_harness_id: Some(preview.id),
            execution_owner_principal_id: Some(owner),
            execution_resolved_owner_user_id: None,
            execution_virtual_user_id: None,
            execution_app_id: Some(app.id),
            legacy_alias_id: None,
            legacy_alias_name: None,
        })
        .await
        .unwrap();
    let child = db
        .create_harness(
            org_id,
            CreateHarnessRow {
                name: "child-operator".to_string(),
                display_name: None,
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: None,
                parent_harness_id: Some(preview.id),
                default_model_id: None,
                tags: vec![],
                initial_files: json!([]),
                mcp_servers: json!({}),
                is_built_in: false,
                network_access: None,
                embedder_metadata: json!({}),
            },
        )
        .await
        .unwrap();
    db.patch_organization_settings(
        org_id,
        UpdateOrganizationSettings {
            default_harness_id: UpdateField::Set(preview.id),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let before_files = WorkspaceFileService::new(db.clone());
    before_files
        .create_file(
            preview_session.id.uuid(),
            CreateFileInput {
                path: "/workspace/note.txt".to_string(),
                content: Some("keep me".to_string()),
                encoding: None,
                is_readonly: None,
            },
        )
        .await
        .unwrap();
    if opted_in {
        // Restoring the legacy session mounts already ensures the shared namespace.
        let memory = db
            .list_memories(org_id, None, false)
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.name == "platform-chat-shared")
            .unwrap();
        db.create_memory_file(
            memory.id,
            CreateMemoryFileRow {
                path: "/remember.md".to_string(),
                content: Some(b"kept preview memory".to_vec()),
                is_directory: false,
                content_hash: None,
            },
        )
        .await
        .unwrap();
    }

    org_init::initialize_org_harnesses(&db, org_id)
        .await
        .unwrap();
    let upgraded = db
        .get_harness_by_name(org_id, "platform-chat")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(upgraded.id, canonical.id);
    assert_eq!(upgraded.status, "archived");
    let managed = db
        .get_agent_by_name(org_id, "platform-chat")
        .await
        .unwrap()
        .unwrap();
    let bashkit_worker = db
        .get_harness_by_name(org_id, "bashkit-worker")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(managed.harness_id, bashkit_worker.id);
    assert!(
        managed
            .system_prompt
            .contains("everruns <noun> <verb> --flags")
    );
    assert!(managed.is_built_in);
    let caps = db.get_agent_capabilities(managed.id.uuid()).await.unwrap();
    assert!(
        caps.iter()
            .any(|cap| cap.capability_id == "platform" && cap.config == json!({"surface":"shell"}))
    );
    assert!(
        db.get_harness_by_name(org_id, "platform-chat-v2")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.get_harness(org_id, preview.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        "deleted"
    );
    assert_eq!(
        db.get_session(org_id, starter.id)
            .await
            .unwrap()
            .unwrap()
            .harness_id,
        Some(bashkit_worker.id)
    );
    assert_eq!(
        db.get_session(org_id, preview_session.id)
            .await
            .unwrap()
            .unwrap()
            .harness_id,
        Some(bashkit_worker.id)
    );
    assert_eq!(
        db.get_agent(org_id, agent.id)
            .await
            .unwrap()
            .unwrap()
            .harness_id,
        canonical.id
    );
    assert_eq!(
        db.get_organization_settings(org_id)
            .await
            .unwrap()
            .unwrap()
            .default_harness_id,
        Some(canonical.id)
    );
    assert_eq!(
        db.get_app_by_id(org_id, app.id)
            .await
            .unwrap()
            .unwrap()
            .harness_id,
        canonical.id.uuid()
    );
    assert_eq!(
        db.get_agent_trigger(org_id, trigger.id)
            .await
            .unwrap()
            .unwrap()
            .execution_harness_id,
        Some(canonical.id)
    );
    assert_eq!(
        db.get_harness(org_id, child.id)
            .await
            .unwrap()
            .unwrap()
            .parent_harness_id,
        Some(canonical.id)
    );
    // A new file service models a restart: old sessions did not run mount setup.
    let files = WorkspaceFileService::new(db.clone());
    assert_eq!(
        files
            .read_file(preview_session.id.uuid(), "/workspace/note.txt")
            .await
            .unwrap()
            .unwrap()
            .content
            .as_deref(),
        Some("keep me")
    );
    let remembered = files
        .read_file(starter.id.uuid(), "/memory/shared/remember.md")
        .await
        .unwrap();
    if opted_in {
        assert_eq!(
            remembered.unwrap().content.as_deref(),
            Some("kept preview memory")
        );
    } else {
        assert!(
            remembered.is_none(),
            "another org's preview notes must not leak"
        );
    }
    let docs = files
        .read_file(
            starter.id.uuid(),
            "/workspace/docs/capabilities/platform.md",
        )
        .await
        .unwrap()
        .unwrap();
    assert!(docs.is_readonly);
    assert!(
        files
            .create_file(
                starter.id.uuid(),
                CreateFileInput {
                    path: "/workspace/docs/overwrite.md".to_string(),
                    content: Some("no".to_string()),
                    encoding: None,
                    is_readonly: None,
                }
            )
            .await
            .is_err()
    );
    files
        .create_file(
            starter.id.uuid(),
            CreateFileInput {
                path: "/memory/shared/new.md".to_string(),
                content: Some("new shared note".to_string()),
                encoding: None,
                is_readonly: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        files
            .read_file(preview_session.id.uuid(), "/memory/shared/new.md")
            .await
            .unwrap()
            .unwrap()
            .content
            .as_deref(),
        Some("new shared note")
    );
    assert_eq!(
        db.list_memories(org_id, None, false).await.unwrap().len(),
        1
    );
    assert!(
        !db.list_org_feature_flags(org_id)
            .await
            .unwrap()
            .contains_key("platform_chat_v2")
    );
    assert!(!db.consolidate_platform_chat(org_id).await.unwrap());
    let again = org_init::initialize_org_harnesses(&db, org_id)
        .await
        .unwrap();
    assert_eq!(again.created, 0);
    assert_eq!(again.updated, 0);
}

#[tokio::test]
async fn platform_chat_upgrade_in_memory() {
    let db = Arc::new(StorageBackend::test_database());
    for opted_in in [true, false] {
        verify_upgrade(db.clone(), opted_in).await;
    }
}

#[tokio::test]
async fn platform_chat_upgrade_postgres() {
    let db = Arc::new(StorageBackend::from_database(Database::new(
        test_harness::create_test_pool().await,
    )));
    for opted_in in [true, false] {
        verify_upgrade(db.clone(), opted_in).await;
    }
}

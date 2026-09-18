//! Fixtures shared by the test modules.

use super::*;
use crate::domains::common::Ctx;
use crate::kernel_imports::{Caller, OrgRole};
use crate::services::CapabilityService;
use crate::storage::{
    CreateHarnessRow, CreateModelRow, CreateOrganizationRow, CreateProviderRow, StorageBackend,
};
use everruns_core::capabilities::Capability;

pub(crate) async fn test_ctx(caller: Caller, db: Arc<StorageBackend>) -> Ctx {
    crate::org_init::initialize_org_harnesses(&db, caller.org_id)
        .await
        .expect("initialize built-in harnesses for session service tests");
    let capability_service = Arc::new(CapabilityService::new(db.clone(), None));
    Ctx::new(
        caller,
        db,
        capability_service,
        None,
        Arc::new(everruns_core::DefaultPermissionResolver),
    )
    .with_feature_flags(crate::domains::common::all_feature_flags_for_test())
}

pub(crate) fn external_caller(org_id: i64) -> Caller {
    Caller {
        org_id,
        org_public_id: everruns_core::organization::org_public_id_from_internal(org_id),
        user_id: None,
        role: OrgRole::Owner,
        is_platform_user: false,
        is_internal: false,
    }
}

pub(crate) fn build_create_request(
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
    model_id: Option<ModelId>,
) -> CreateSessionRequest {
    CreateSessionRequest {
        source: None,
        workspace_id: None,
        harness_id: Some(harness_id),
        harness_name: None,
        agent_id,
        agent_name: None,
        agent_identity_id: None,
        title: Some("Test Session".to_string()),
        goal: None,
        locale: None,
        tags: vec![],
        model_id,
        capabilities: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        system_prompt: None,
        initial_files: vec![],
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        parent_session_id: None,
        forked_from_session_id: None,
        budget_root_session_id: None,
        seed: SessionSeedMode::Fresh,
    }
}

pub(crate) async fn create_second_org(db: &StorageBackend) -> i64 {
    db.create_organization_with_id(
        2,
        CreateOrganizationRow {
            public_id: "org_2".to_string(),
            name: "Org 2".to_string(),
            created_by: None,
        },
    )
    .await
    .unwrap()
    .unwrap()
    .org_id
}

pub(crate) async fn create_model(db: &StorageBackend, org_id: i64, model_id: &str) -> ModelId {
    let provider = db
        .create_provider(
            org_id,
            CreateProviderRow {
                name: format!("Provider {org_id}"),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    db.create_model(
        org_id,
        CreateModelRow {
            provider_id: provider.id,
            model_id: model_id.to_string(),
            display_name: model_id.to_string(),
            capabilities: vec![],
            enabled: true,
            is_favorite: false,
            source: "manual".to_string(),
            provider_metadata: None,
        },
    )
    .await
    .unwrap()
    .id
}

pub(crate) struct TestHighRiskCapability;

impl Capability for TestHighRiskCapability {
    fn id(&self) -> &str {
        "test_high_risk"
    }

    fn name(&self) -> &str {
        "Test High Risk"
    }

    fn description(&self) -> &str {
        "Test-only high risk capability"
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }
}

// Build a harness carrying a `user_hooks` capability with a single hook of
// the given event that writes a sentinel into the session VFS. Exercises the
// real server resolve -> finalize -> bashkit_shell dispatch path end-to-end.
pub(crate) async fn harness_with_user_hook(
    db: &Arc<StorageBackend>,
    org_id: i64,
    name: &str,
    event: &str,
    command: &str,
) -> HarnessId {
    let harness = db
        .create_harness(
            org_id,
            CreateHarnessRow {
                name: name.to_string(),
                display_name: Some(name.to_string()),
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: Some("hooked".to_string()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                embedder_metadata: serde_json::json!({}),
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    db.set_harness_capabilities(
        harness.id.uuid(),
        vec![(
            "user_hooks".to_string(),
            0,
            serde_json::json!({
                "hooks": [{
                    "event": event,
                    "executor": { "type": "bash", "command": command },
                }]
            }),
        )],
    )
    .await
    .unwrap();
    harness.id
}

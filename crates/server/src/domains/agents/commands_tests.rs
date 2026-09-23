use super::*;
use crate::kernel_imports::{
    AgentLoopError, Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, DefaultPermissionResolver,
    LlmResponse, LlmResponseStream, OrgRole, Result as CoreResult, UtilityLlmRequest,
    UtilityLlmService,
};
use crate::services::CapabilityService;
use crate::storage::StorageBackend;
use crate::storage::models::{CreateAgentIdentityConnectionRow, CreateHarnessRow};
use async_trait::async_trait;
use everruns_platform::FeatureFlags;
use std::sync::Arc;
use uuid::Uuid;

#[test]
fn first_agent_version_is_initial_minor_even_for_major_change() {
    let (major, minor, patch, version) =
        bump_published_version(None, &AgentVersionChangeKind::Major);

    assert_eq!((major, minor, patch), (0, 1, 0));
    assert_eq!(version, "0.1.0");
}

fn ctx_with_role_and_flags(
    db: Arc<StorageBackend>,
    role: OrgRole,
    feature_flags: FeatureFlags,
) -> Ctx {
    futures::executor::block_on(crate::org_init::initialize_org_harnesses(
        &db,
        DEFAULT_ORG_ID,
    ))
    .expect("initialize built-in harnesses for agent command tests");
    let capability_service = Arc::new(CapabilityService::new(db.clone(), None));
    Ctx::new(
        Caller {
            org_id: DEFAULT_ORG_ID,
            project_id: everruns_core::DEFAULT_PROJECT_ID,
            org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            user_id: Some(Uuid::nil()),
            role,
            is_platform_user: false,
            is_internal: false,
        },
        db,
        capability_service,
        None,
        Arc::new(DefaultPermissionResolver),
    )
    .with_feature_flags(feature_flags)
}

fn ctx_with_role(db: Arc<StorageBackend>, role: OrgRole) -> Ctx {
    ctx_with_role_and_flags(
        db,
        role,
        FeatureFlags {
            agent_versions: true,
            ..FeatureFlags::default()
        },
    )
}

struct ExhaustedUtilityLlm;

#[async_trait]
impl UtilityLlmService for ExhaustedUtilityLlm {
    fn is_configured(&self) -> bool {
        true
    }

    async fn chat_completion(&self, _request: UtilityLlmRequest) -> CoreResult<LlmResponse> {
        Err(AgentLoopError::llm(
            "credit_balance_exhausted: credits depleted; api_key=secret",
        ))
    }

    async fn chat_completion_stream(
        &self,
        _request: UtilityLlmRequest,
    ) -> CoreResult<LlmResponseStream> {
        unreachable!("agent analysis does not stream")
    }
}

#[tokio::test]
async fn analyze_maps_provider_quota_failure_to_safe_actionable_error() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx =
        ctx_with_role(db, OrgRole::Owner).with_utility_llm_service(Arc::new(ExhaustedUtilityLlm));

    let error = AnalyzeAgent {
        system_prompt: Some("Be helpful.".to_string()),
        capabilities: Vec::new(),
        tools: Vec::new(),
        mcp_servers: Default::default(),
    }
    .execute(&ctx)
    .await
    .expect_err("exhausted provider must fail analysis");

    let (status, axum::Json(body)): (
        axum::http::StatusCode,
        axum::Json<crate::api::common::ErrorResponse>,
    ) = error.into();
    assert_eq!(status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body.code.as_deref(), Some("provider_quota_exhausted"));
    let detail = body.detail.expect("safe actionable detail");
    assert!(detail.contains("out of credits or quota"));
    assert!(!detail.contains("api_key"));
    assert!(!detail.contains("secret"));
}

fn high_risk_agent_request(name: String) -> CreateAgentRequest {
    CreateAgentRequest {
        id: None,
        name,
        display_name: None,
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "test".to_string(),
        default_model_id: None,
        harness_id: None,
        harness_name: None,
        tags: Vec::new(),
        capabilities: vec![AgentCapabilityConfig::new("bashkit_shell")],
        initial_files: Vec::new(),
        tools: Vec::new(),
        mcp_servers: Default::default(),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
    }
}

fn basic_agent_request(name: &str) -> CreateAgentRequest {
    CreateAgentRequest {
        id: None,
        name: name.to_string(),
        display_name: None,
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "initial prompt".to_string(),
        default_model_id: None,
        harness_id: None,
        harness_name: None,
        tags: Vec::new(),
        capabilities: Vec::new(),
        initial_files: Vec::new(),
        tools: Vec::new(),
        mcp_servers: Default::default(),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
    }
}

fn update_prompt_request(system_prompt: &str) -> UpdateAgentRequest {
    UpdateAgentRequest {
        name: None,
        display_name: None,
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: None,
        system_prompt: Some(system_prompt.to_string()),
        default_model_id: None,
        harness_id: None,
        harness_name: None,
        tags: None,
        capabilities: None,
        initial_files: None,
        status: None,
        tools: None,
        mcp_servers: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
    }
}

async fn create_test_harness(db: &StorageBackend, name: &str) -> HarnessId {
    db.create_harness(
        DEFAULT_ORG_ID,
        CreateHarnessRow {
            name: name.to_string(),
            display_name: Some(name.to_string()),
            icon: None,
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: serde_json::json!([]),
            system_prompt: Some("test harness".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: Vec::new(),
            initial_files: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            network_access: None,
            embedder_metadata: serde_json::json!({}),
            is_built_in: false,
        },
    )
    .await
    .expect("create test harness")
    .id
}

#[tokio::test]
async fn create_agent_defaults_to_generic_harness() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
    let generic_id = crate::org_init::generic_harness_id(&db, DEFAULT_ORG_ID)
        .await
        .expect("generic harness id");

    let created = CreateAgent(basic_agent_request("default-harness-agent"))
        .run(&ctx)
        .await
        .expect("agent is created");

    assert_eq!(created.harness_id, generic_id);
}

#[tokio::test]
async fn create_and_update_agent_resolve_harness_name_and_id() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
    let first_harness_id = create_test_harness(&db, "agent-harness-one").await;
    let second_harness_id = create_test_harness(&db, "agent-harness-two").await;

    let mut req = basic_agent_request("named-harness-agent");
    req.harness_name = Some("agent-harness-one".to_string());
    let created = CreateAgent(req).run(&ctx).await.expect("agent is created");
    assert_eq!(created.harness_id, first_harness_id);

    let renamed = UpdateAgentCmd {
        id: created.public_id.to_string(),
        req: UpdateAgentRequest {
            name: None,
            display_name: Some("renamed".to_string()),
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: None,
            system_prompt: None,
            default_model_id: None,
            harness_id: None,
            harness_name: None,
            tags: None,
            capabilities: None,
            initial_files: None,
            status: None,
            tools: None,
            mcp_servers: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
        },
    }
    .run(&ctx)
    .await
    .expect("agent is updated without harness change");
    assert_eq!(renamed.harness_id, first_harness_id);

    let changed = UpdateAgentCmd {
        id: created.public_id.to_string(),
        req: UpdateAgentRequest {
            harness_id: Some(second_harness_id),
            ..update_prompt_request("changed harness")
        },
    }
    .run(&ctx)
    .await
    .expect("agent harness changes by id");
    assert_eq!(changed.harness_id, second_harness_id);
}

#[tokio::test]
async fn create_agent_rejects_unknown_archived_or_ambiguous_harness() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner);

    let mut unknown = basic_agent_request("unknown-harness-agent");
    unknown.harness_id = Some(HarnessId::new());
    let err = CreateAgent(unknown)
        .run(&ctx)
        .await
        .expect_err("unknown harness is rejected");
    assert_eq!(err.status(), axum::http::StatusCode::NOT_FOUND);

    let archived_id = create_test_harness(&db, "archived-agent-harness").await;
    db.delete_harness(DEFAULT_ORG_ID, archived_id)
        .await
        .expect("archive harness");
    let mut archived = basic_agent_request("archived-harness-agent");
    archived.harness_id = Some(archived_id);
    let err = CreateAgent(archived)
        .run(&ctx)
        .await
        .expect_err("archived harness is rejected");
    assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);

    let mut ambiguous = basic_agent_request("ambiguous-harness-agent");
    ambiguous.harness_id = Some(archived_id);
    ambiguous.harness_name = Some("generic".to_string());
    let err = CreateAgent(ambiguous)
        .run(&ctx)
        .await
        .expect_err("ambiguous harness selection is rejected");
    assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
}

async fn create_high_risk_agent_version(ctx: &Ctx, name: &str) -> (Agent, AgentVersion) {
    let agent = CreateAgent(high_risk_agent_request(name.to_string()))
        .run(ctx)
        .await
        .expect("admin can create high-risk agent");
    let version = CreateAgentVersionCmd {
        agent_id: agent.public_id.to_string(),
        req: CreateAgentVersionRequest {
            summary: None,
            change_kind: None,
        },
    }
    .run(ctx)
    .await
    .expect("admin can create high-risk version");
    (agent, version)
}

#[tokio::test]
async fn update_agent_creates_unpublished_auto_snapshot() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
    let agent = CreateAgent(basic_agent_request("auto-snapshot-agent"))
        .run(&ctx)
        .await
        .expect("agent is created");

    UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: update_prompt_request("updated prompt"),
    }
    .run(&ctx)
    .await
    .expect("agent is updated");

    let snapshots = ListAgentVersions {
        agent_id: agent.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .expect("versions are listed");
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].change_kind, AgentVersionChangeKind::Auto);
    assert!(!snapshots[0].is_published);
    assert_eq!(snapshots[0].version, "draft.1");
    assert_eq!(
        snapshots[0].authored_config["system_prompt"],
        "updated prompt"
    );

    let published = CreateAgentVersionCmd {
        agent_id: agent.public_id.to_string(),
        req: CreateAgentVersionRequest {
            summary: Some("publish current draft".to_string()),
            change_kind: None,
        },
    }
    .run(&ctx)
    .await
    .expect("published version is created");
    assert!(published.is_published);
    assert_eq!(published.version, "0.1.0");
    assert_eq!(published.version_number, 2);

    let latest_published = db
        .get_latest_agent_version(DEFAULT_ORG_ID, AgentId::from_uuid(agent.internal_id))
        .await
        .expect("latest published lookup succeeds")
        .expect("published version exists");
    assert_eq!(latest_published.id, published.public_id);
}

#[tokio::test]
async fn update_agent_updates_parallel_tool_calls() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db, OrgRole::Owner);
    let agent = CreateAgent(basic_agent_request("parallel-tool-calls-agent"))
        .run(&ctx)
        .await
        .expect("agent is created");

    let mut req = update_prompt_request("enable parallel tool calls");
    req.parallel_tool_calls = Some(true);
    let updated = UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req,
    }
    .run(&ctx)
    .await
    .expect("agent enables parallel tool calls");
    assert_eq!(updated.parallel_tool_calls, Some(true));

    let mut req = update_prompt_request("disable parallel tool calls");
    req.parallel_tool_calls = Some(false);
    let updated = UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req,
    }
    .run(&ctx)
    .await
    .expect("agent disables parallel tool calls");
    assert_eq!(updated.parallel_tool_calls, Some(false));
}

#[tokio::test]
async fn update_agent_skips_auto_snapshot_when_versions_disabled() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role_and_flags(db.clone(), OrgRole::Owner, FeatureFlags::default());
    let agent = CreateAgent(basic_agent_request("auto-snapshot-disabled-agent"))
        .run(&ctx)
        .await
        .expect("agent is created");

    UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: update_prompt_request("updated prompt"),
    }
    .run(&ctx)
    .await
    .expect("agent is updated");

    let snapshots = db
        .list_agent_versions(DEFAULT_ORG_ID, AgentId::from_uuid(agent.internal_id))
        .await
        .expect("stored versions are listed");
    assert!(snapshots.is_empty());
}

#[tokio::test]
async fn update_agent_skips_auto_snapshot_for_unchanged_config() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db, OrgRole::Owner);
    let agent = CreateAgent(basic_agent_request("auto-snapshot-noop-agent"))
        .run(&ctx)
        .await
        .expect("agent is created");

    UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: UpdateAgentRequest {
            name: None,
            display_name: None,
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: None,
            system_prompt: None,
            default_model_id: None,
            harness_id: None,
            harness_name: None,
            tags: None,
            capabilities: None,
            initial_files: None,
            status: None,
            tools: None,
            mcp_servers: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
        },
    }
    .run(&ctx)
    .await
    .expect("no-op update succeeds");

    let snapshots = ListAgentVersions {
        agent_id: agent.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .expect("versions are listed");
    assert!(snapshots.is_empty());
}

#[tokio::test]
async fn update_agent_prunes_old_auto_snapshots() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db, OrgRole::Owner);
    let agent = CreateAgent(basic_agent_request("auto-snapshot-pruned-agent"))
        .run(&ctx)
        .await
        .expect("agent is created");

    for i in 0..(MAX_AUTO_SNAPSHOTS_PER_AGENT + 2) {
        UpdateAgentCmd {
            id: agent.public_id.to_string(),
            req: update_prompt_request(&format!("updated prompt {i}")),
        }
        .run(&ctx)
        .await
        .expect("agent is updated");
    }

    let snapshots = ListAgentVersions {
        agent_id: agent.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .expect("versions are listed");
    assert_eq!(snapshots.len(), MAX_AUTO_SNAPSHOTS_PER_AGENT as usize);
    assert_eq!(snapshots[0].version, "draft.52");
    assert_eq!(snapshots.last().unwrap().version, "draft.3");
}

#[tokio::test]
async fn upsert_agent_update_creates_unpublished_auto_snapshot() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db, OrgRole::Owner);
    let agent_id = AgentId::new();
    let mut req = basic_agent_request("upsert-auto-snapshot-agent");
    req.system_prompt = "initial upsert prompt".to_string();

    let created = UpsertAgent {
        id: agent_id.to_string(),
        req: req.clone(),
    }
    .run(&ctx)
    .await
    .expect("agent is created by upsert");
    assert!(created.was_created);

    req.system_prompt = "updated upsert prompt".to_string();
    let updated = UpsertAgent {
        id: agent_id.to_string(),
        req,
    }
    .run(&ctx)
    .await
    .expect("agent is updated by upsert");
    assert!(!updated.was_created);

    let snapshots = ListAgentVersions {
        agent_id: agent_id.to_string(),
    }
    .run(&ctx)
    .await
    .expect("versions are listed");
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].change_kind, AgentVersionChangeKind::Auto);
    assert!(!snapshots[0].is_published);
    assert_eq!(
        snapshots[0].authored_config["system_prompt"],
        "updated upsert prompt"
    );
}

#[tokio::test]
async fn upsert_agent_skips_auto_snapshot_for_unchanged_config() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db, OrgRole::Owner);
    let agent_id = AgentId::new();
    let req = basic_agent_request("upsert-auto-snapshot-noop-agent");

    UpsertAgent {
        id: agent_id.to_string(),
        req: req.clone(),
    }
    .run(&ctx)
    .await
    .expect("agent is created by upsert");
    UpsertAgent {
        id: agent_id.to_string(),
        req,
    }
    .run(&ctx)
    .await
    .expect("agent is upserted without changes");

    let snapshots = ListAgentVersions {
        agent_id: agent_id.to_string(),
    }
    .run(&ctx)
    .await
    .expect("versions are listed");
    assert!(snapshots.is_empty());
}

#[tokio::test]
async fn create_agent_version_rejects_auto_change_kind() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db, OrgRole::Owner);
    let agent = CreateAgent(basic_agent_request("manual-auto-kind-agent"))
        .run(&ctx)
        .await
        .expect("agent is created");

    let err = CreateAgentVersionCmd {
        agent_id: agent.public_id.to_string(),
        req: CreateAgentVersionRequest {
            summary: None,
            change_kind: Some(AgentVersionChangeKind::Auto),
        },
    }
    .run(&ctx)
    .await
    .expect_err("manual publish cannot use automatic change kind");

    assert!(
        matches!(
            err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ),
        "expected BadRequest, got {err:?}"
    );
}

#[tokio::test]
async fn set_default_version_rejects_unpublished_snapshot() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db, OrgRole::Owner);
    let agent = CreateAgent(basic_agent_request("default-draft-agent"))
        .run(&ctx)
        .await
        .expect("agent is created");
    UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: update_prompt_request("draft-only prompt"),
    }
    .run(&ctx)
    .await
    .expect("agent is updated");
    let snapshots = ListAgentVersions {
        agent_id: agent.public_id.to_string(),
    }
    .run(&ctx)
    .await
    .expect("versions are listed");

    let err = SetDefaultAgentVersion {
        agent_id: agent.public_id.to_string(),
        req: SetDefaultAgentVersionRequest {
            version_id: snapshots[0].public_id,
        },
    }
    .run(&ctx)
    .await
    .expect_err("draft snapshot cannot become default");

    assert!(
        matches!(
            err,
            CommandError {
                kind: CommandErrorKind::BadRequest(_),
                ..
            }
        ),
        "expected BadRequest, got {err:?}"
    );
}

#[tokio::test]
async fn set_default_version_blocks_member_for_high_risk_capabilities() {
    let db = Arc::new(StorageBackend::in_memory());
    let admin_ctx = ctx_with_role(db.clone(), OrgRole::Admin);
    let member_ctx = ctx_with_role(db, OrgRole::Member);
    let (agent, version) =
        create_high_risk_agent_version(&admin_ctx, "member-set-default-denied").await;

    let err = SetDefaultAgentVersion {
        agent_id: agent.public_id.to_string(),
        req: SetDefaultAgentVersionRequest {
            version_id: version.public_id,
        },
    }
    .run(&member_ctx)
    .await
    .expect_err("member must not activate high-risk version");

    assert!(
        matches!(
            err,
            CommandError {
                kind: CommandErrorKind::Forbidden(_),
                ..
            }
        ),
        "expected Forbidden, got {err:?}"
    );

    SetDefaultAgentVersion {
        agent_id: agent.public_id.to_string(),
        req: SetDefaultAgentVersionRequest {
            version_id: version.public_id,
        },
    }
    .run(&admin_ctx)
    .await
    .expect("admin can activate high-risk version");
}

#[tokio::test]
async fn rollback_version_restores_versioned_harness() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
    let first_harness_id = create_test_harness(&db, "rollback-harness-one").await;
    let second_harness_id = create_test_harness(&db, "rollback-harness-two").await;

    let mut req = basic_agent_request("rollback-harness-agent");
    req.harness_id = Some(first_harness_id);
    let agent = CreateAgent(req).run(&ctx).await.expect("agent is created");
    let version = CreateAgentVersionCmd {
        agent_id: agent.public_id.to_string(),
        req: CreateAgentVersionRequest {
            summary: Some("save first harness".to_string()),
            change_kind: None,
        },
    }
    .run(&ctx)
    .await
    .expect("version is created");

    let changed = UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: UpdateAgentRequest {
            harness_id: Some(second_harness_id),
            ..update_prompt_request("switch to second harness")
        },
    }
    .run(&ctx)
    .await
    .expect("agent harness changes");
    assert_eq!(changed.harness_id, second_harness_id);

    let restored = RollbackAgentVersion {
        agent_id: agent.public_id.to_string(),
        version_id: version.public_id,
        req: RollbackAgentVersionRequest {
            save_version: false,
            summary: None,
        },
    }
    .run(&ctx)
    .await
    .expect("rollback succeeds");

    assert_eq!(restored.harness_id, first_harness_id);
}

#[tokio::test]
async fn rollback_version_blocks_member_for_high_risk_capabilities() {
    let db = Arc::new(StorageBackend::in_memory());
    let owner_ctx = ctx_with_role(db.clone(), OrgRole::Owner);
    let member_ctx = ctx_with_role(db, OrgRole::Member);
    let (agent, version) =
        create_high_risk_agent_version(&owner_ctx, "member-rollback-denied").await;

    let err = RollbackAgentVersion {
        agent_id: agent.public_id.to_string(),
        version_id: version.public_id,
        req: RollbackAgentVersionRequest {
            save_version: false,
            summary: None,
        },
    }
    .run(&member_ctx)
    .await
    .expect_err("member must not roll back to high-risk version");

    assert!(
        matches!(
            err,
            CommandError {
                kind: CommandErrorKind::Forbidden(_),
                ..
            }
        ),
        "expected Forbidden, got {err:?}"
    );

    RollbackAgentVersion {
        agent_id: agent.public_id.to_string(),
        version_id: version.public_id,
        req: RollbackAgentVersionRequest {
            save_version: false,
            summary: None,
        },
    }
    .run(&owner_ctx)
    .await
    .expect("owner can roll back to high-risk version");
}

#[tokio::test]
async fn agent_creation_rejected_at_limit_and_allowed_below() {
    let db = Arc::new(StorageBackend::in_memory());
    let mut ctx = ctx_with_role(db, OrgRole::Owner);
    ctx.resource_limits.max_agents_per_org = 2;

    CreateAgent(basic_agent_request("a1"))
        .execute(&ctx)
        .await
        .expect("first agent below limit");
    CreateAgent(basic_agent_request("a2"))
        .execute(&ctx)
        .await
        .expect("second agent at limit boundary");

    let err = CreateAgent(basic_agent_request("a3"))
        .execute(&ctx)
        .await
        .expect_err("third agent exceeds the cap");
    assert_eq!(err.status().as_u16(), 409);
    assert!(err.message().contains("Agent limit reached"));
}

#[tokio::test]
async fn soft_deleted_agents_do_not_count_toward_limit() {
    let db = Arc::new(StorageBackend::in_memory());
    let mut ctx = ctx_with_role(db, OrgRole::Owner);
    ctx.resource_limits.max_agents_per_org = 1;

    let a1 = CreateAgent(basic_agent_request("a1"))
        .execute(&ctx)
        .await
        .expect("first agent below limit");

    let err = CreateAgent(basic_agent_request("a2"))
        .execute(&ctx)
        .await
        .expect_err("second agent exceeds the cap");
    assert_eq!(err.status().as_u16(), 409);

    // Archive then mark deleted; a deleted row must not count toward the cap.
    DeleteAgent {
        id: a1.public_id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect("archive a1");
    DestroyAgent {
        id: a1.public_id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect("mark a1 deleted");

    CreateAgent(basic_agent_request("a2"))
        .execute(&ctx)
        .await
        .expect("creation allowed once the deleted agent is excluded");
}

#[tokio::test]
async fn archiving_agent_revokes_all_identity_connections() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
    let agent = CreateAgent(basic_agent_request("grant-owner"))
        .execute(&ctx)
        .await
        .unwrap();
    let row = db
        .get_agent_by_public_id(DEFAULT_ORG_ID, None, &agent.public_id.to_string())
        .await
        .unwrap()
        .unwrap();
    let (identity_id, _) = crate::domains::agent_identities::lifecycle::ensure_identity_for_agent(
        &db,
        DEFAULT_ORG_ID,
        &row,
    )
    .await
    .unwrap();
    for provider in ["mcp_oauth_one", "mcp_oauth_two"] {
        db.upsert_agent_identity_connection(CreateAgentIdentityConnectionRow {
            agent_identity_id: identity_id,
            provider: provider.to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: None,
            provider_username: None,
            access_token_encrypted: Some(vec![1, 2, 3]),
            refresh_token_encrypted: Some(vec![4, 5, 6]),
            scopes: None,
            expires_at: None,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
    }

    DeleteAgent {
        id: agent.public_id.to_string(),
    }
    .execute(&ctx)
    .await
    .unwrap();

    assert!(
        db.list_agent_identity_connections(identity_id)
            .await
            .unwrap()
            .is_empty()
    );
}

// ========================================================================
// Built-in agent protection (EVE-865)
// ========================================================================

/// Create an agent and flip it to built-in, as org bootstrap does.
async fn seed_built_in_agent(db: &StorageBackend, ctx: &Ctx, name: &str) -> Agent {
    let agent = CreateAgent(basic_agent_request(name))
        .execute(ctx)
        .await
        .expect("seed agent");
    db.mark_agent_built_in(DEFAULT_ORG_ID, AgentId::from_uuid(agent.internal_id))
        .await
        .expect("mark built-in");
    agent
}

/// Every guard must answer 400 with the copy-first hint, not 404 or 500.
fn assert_built_in_rejection(err: &CommandError) {
    assert_eq!(err.status().as_u16(), 400, "message: {}", err.message());
    assert!(
        err.message().contains("built-in agent"),
        "expected a built-in rejection, got: {}",
        err.message()
    );
    assert!(
        err.message().contains("Copy it first"),
        "rejection must point at the escape hatch, got: {}",
        err.message()
    );
}

#[tokio::test]
async fn built_in_agent_rejects_update() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
    let agent = seed_built_in_agent(&db, &ctx, "chat").await;

    let err = UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: UpdateAgentRequest {
            system_prompt: Some("hijacked".to_string()),
            ..Default::default()
        },
    }
    .execute(&ctx)
    .await
    .expect_err("built-in definition is immutable");
    assert_built_in_rejection(&err);

    // The prompt must actually be unchanged, not merely reported as such.
    let after = q::get_by_public_id(&db, DEFAULT_ORG_ID, None, &agent.public_id.to_string())
        .await
        .expect("reload")
        .expect("agent still present");
    assert_eq!(after.system_prompt, "initial prompt");
}

#[tokio::test]
async fn built_in_agent_rejects_archive() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
    let agent = seed_built_in_agent(&db, &ctx, "chat").await;

    // Archive travels through UpdateAgentCmd as a status change.
    let err = UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: UpdateAgentRequest {
            status: Some(AgentStatus::Archived),
            ..Default::default()
        },
    }
    .execute(&ctx)
    .await
    .expect_err("built-in agent cannot be archived");
    assert_built_in_rejection(&err);
}

#[tokio::test]
async fn built_in_agent_rejects_delete_and_destroy() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
    let agent = seed_built_in_agent(&db, &ctx, "chat").await;

    let err = DeleteAgent {
        id: agent.public_id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect_err("built-in agent cannot be archived away");
    assert_built_in_rejection(&err);

    let err = DestroyAgent {
        id: agent.public_id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect_err("built-in agent cannot be destroyed");
    assert_built_in_rejection(&err);

    assert!(
        q::get_by_public_id(&db, DEFAULT_ORG_ID, None, &agent.public_id.to_string())
            .await
            .expect("reload")
            .is_some(),
        "built-in agent must survive both delete verbs"
    );
}

#[tokio::test]
async fn built_in_agent_rejects_upsert() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
    let agent = seed_built_in_agent(&db, &ctx, "chat").await;

    let err = UpsertAgent {
        id: agent.public_id.to_string(),
        req: basic_agent_request("chat"),
    }
    .execute(&ctx)
    .await
    .expect_err("upsert must not overwrite a built-in");
    assert_built_in_rejection(&err);
}

#[tokio::test]
async fn built_in_agent_rejects_version_mutations() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
    let agent = seed_built_in_agent(&db, &ctx, "chat").await;
    let id = agent.public_id.to_string();

    let err = CreateAgentVersionCmd {
        agent_id: id.clone(),
        req: CreateAgentVersionRequest {
            change_kind: Some(AgentVersionChangeKind::Minor),
            summary: None,
        },
    }
    .execute(&ctx)
    .await
    .expect_err("versions are part of the protected definition");
    assert_built_in_rejection(&err);

    let err = SetDefaultAgentVersion {
        agent_id: id.clone(),
        req: SetDefaultAgentVersionRequest {
            version_id: AgentVersionId::new(),
        },
    }
    .execute(&ctx)
    .await
    .expect_err("default version is part of the protected definition");
    assert_built_in_rejection(&err);

    let err = RollbackAgentVersion {
        agent_id: id,
        version_id: AgentVersionId::new(),
        req: RollbackAgentVersionRequest {
            save_version: false,
            summary: None,
        },
    }
    .execute(&ctx)
    .await
    .expect_err("rollback rewrites the protected definition");
    assert_built_in_rejection(&err);
}

#[tokio::test]
async fn built_in_agent_can_be_copied() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
    let agent = seed_built_in_agent(&db, &ctx, "chat").await;

    let copy = CopyAgent {
        id: agent.public_id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect("copy is the escape hatch and must stay open");

    assert_ne!(copy.public_id, agent.public_id);

    // The copy must be editable, or the escape hatch leads nowhere.
    UpdateAgentCmd {
        id: copy.public_id.to_string(),
        req: UpdateAgentRequest {
            system_prompt: Some("edited".to_string()),
            ..Default::default()
        },
    }
    .execute(&ctx)
    .await
    .expect("the copy is an ordinary editable agent");
}

#[tokio::test]
async fn built_in_agents_do_not_count_toward_limit() {
    let db = Arc::new(StorageBackend::in_memory());
    let mut ctx = ctx_with_role(db.clone(), OrgRole::Owner);
    ctx.resource_limits.max_agents_per_org = 1;

    seed_built_in_agent(&db, &ctx, "platform-chat").await;

    // The built-in must not consume the cap, so a user agent still fits.
    CreateAgent(basic_agent_request("mine"))
        .execute(&ctx)
        .await
        .expect("built-in agent must not consume the org quota");
}

use super::*;
use crate::kernel_imports::{
    AgentLoopError, Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, DefaultPermissionResolver,
    LlmResponse, LlmResponseStream, OrgRole, Result as CoreResult, UtilityLlmRequest,
    UtilityLlmService,
};
use crate::records::FeatureFlags;
use crate::services::CapabilityService;
use crate::storage::StorageBackend;
use crate::storage::models::{CreateHarnessRow, CreateVirtualUserConnectionRow};
use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

async fn ctx_with_role_and_flags(
    db: Arc<StorageBackend>,
    role: OrgRole,
    feature_flags: FeatureFlags,
) -> Ctx {
    crate::setup::org_init::initialize_org_harnesses(&db, DEFAULT_ORG_ID)
        .await
        .expect("initialize built-in harnesses for agent command tests");
    let capability_service = Arc::new(CapabilityService::new(db.clone(), None));
    Ctx::new(
        Caller {
            org_id: DEFAULT_ORG_ID,
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

async fn ctx_with_role(db: Arc<StorageBackend>, role: OrgRole) -> Ctx {
    ctx_with_role_and_flags(
        db,
        role,
        FeatureFlags {
            ..FeatureFlags::default()
        },
    )
    .await
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
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_with_role(db, OrgRole::Owner)
        .await
        .with_utility_llm_service(Arc::new(ExhaustedUtilityLlm));

    let error = AnalyzeAgent {
        harness_id: None,
        initial_files: vec![],
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
        axum::Json<crate::common_dto::ErrorResponse>,
    ) = error.into();
    assert_eq!(status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body.code.as_deref(), Some("provider_quota_exhausted"));
    let detail = body.detail.expect("safe actionable detail");
    assert!(detail.contains("out of credits or quota"));
    assert!(!detail.contains("api_key"));
    assert!(!detail.contains("secret"));
}

fn basic_agent_request(name: &str) -> CreateAgentRequest {
    CreateAgentRequest {
        service_virtual_user_id: None,

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
        sandbox_policy: None,
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
        service_virtual_user_id: crate::storage::UpdateField::Unchanged,

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
        sandbox_policy: crate::storage::UpdateField::Unchanged,
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
async fn create_agent_defaults_to_conversation_harness() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner).await;
    crate::setup::org_init::initialize_org_harnesses(&db, DEFAULT_ORG_ID)
        .await
        .unwrap();
    let conversation_id = db
        .get_harness_by_name(DEFAULT_ORG_ID, "conversation")
        .await
        .unwrap()
        .unwrap()
        .id;

    let created = CreateAgent(basic_agent_request("default-harness-agent"))
        .run(&ctx)
        .await
        .expect("agent is created");

    assert_eq!(created.harness_id, conversation_id);
}

#[tokio::test]
async fn create_and_update_agent_resolve_harness_name_and_id() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner).await;
    let first_harness_id = create_test_harness(&db, "agent-harness-one").await;
    let second_harness_id = create_test_harness(&db, "agent-harness-two").await;

    let mut req = basic_agent_request("named-harness-agent");
    req.harness_name = Some("agent-harness-one".to_string());
    let created = CreateAgent(req).run(&ctx).await.expect("agent is created");
    assert_eq!(created.harness_id, first_harness_id);

    let renamed = UpdateAgentCmd {
        id: created.public_id.to_string(),
        req: UpdateAgentRequest {
            service_virtual_user_id: crate::storage::UpdateField::Unchanged,

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
            sandbox_policy: crate::storage::UpdateField::Unchanged,
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
            service_virtual_user_id: crate::storage::UpdateField::Unchanged,

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
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner).await;

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

#[tokio::test]
async fn update_agent_updates_parallel_tool_calls() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_with_role(db, OrgRole::Owner).await;
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
async fn agent_creation_rejected_at_limit_and_allowed_below() {
    let db = Arc::new(StorageBackend::test_database());
    let mut ctx = ctx_with_role(db, OrgRole::Owner).await;
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
    let db = Arc::new(StorageBackend::test_database());
    let mut ctx = ctx_with_role(db, OrgRole::Owner).await;
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
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner).await;
    let agent = CreateAgent(basic_agent_request("grant-owner"))
        .execute(&ctx)
        .await
        .unwrap();
    let row = db
        .get_agent_by_public_id(DEFAULT_ORG_ID, &agent.public_id.to_string())
        .await
        .unwrap()
        .unwrap();
    let (identity_id, _) = crate::domains::virtual_users::lifecycle::ensure_identity_for_agent(
        &db,
        DEFAULT_ORG_ID,
        &row,
    )
    .await
    .unwrap();
    for provider in ["mcp_oauth_one", "mcp_oauth_two"] {
        db.upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: identity_id,
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
        db.list_virtual_user_connections(identity_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn patch_archiving_agent_revokes_all_identity_connections() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner).await;
    let agent = CreateAgent(basic_agent_request("patch-grant-owner"))
        .execute(&ctx)
        .await
        .unwrap();
    let row = db
        .get_agent_by_public_id(DEFAULT_ORG_ID, &agent.public_id.to_string())
        .await
        .unwrap()
        .unwrap();
    let (identity_id, _) = crate::domains::virtual_users::lifecycle::ensure_identity_for_agent(
        &db,
        DEFAULT_ORG_ID,
        &row,
    )
    .await
    .unwrap();
    db.upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
        virtual_user_id: identity_id,
        provider: "mcp_oauth_patch".to_string(),
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

    UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: UpdateAgentRequest {
            service_virtual_user_id: crate::storage::UpdateField::Unchanged,

            status: Some(AgentStatus::Archived),
            ..Default::default()
        },
    }
    .execute(&ctx)
    .await
    .unwrap();

    assert!(
        db.list_virtual_user_connections(identity_id)
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

/// The UI hides edit, archive and delete from `is_built_in`, so the read
/// model must carry it for built-in agents and leave it false otherwise.
#[tokio::test]
async fn built_in_flag_is_exposed_on_reads() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner).await;
    let built_in = seed_built_in_agent(&db, &ctx, "chat").await;
    let regular = CreateAgent(basic_agent_request("regular"))
        .execute(&ctx)
        .await
        .expect("create");
    assert!(!regular.is_built_in);

    let after = GetAgent {
        id: built_in.public_id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect("get");
    assert!(after.is_built_in);
    let json = serde_json::to_value(&after).expect("serialize");
    assert_eq!(json["is_built_in"], serde_json::json!(true));
}

#[tokio::test]
async fn built_in_agent_rejects_update() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner).await;
    let agent = seed_built_in_agent(&db, &ctx, "chat").await;

    let err = UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: UpdateAgentRequest {
            service_virtual_user_id: crate::storage::UpdateField::Unchanged,

            system_prompt: Some("hijacked".to_string()),
            ..Default::default()
        },
    }
    .execute(&ctx)
    .await
    .expect_err("built-in definition is immutable");
    assert_built_in_rejection(&err);

    // The prompt must actually be unchanged, not merely reported as such.
    let after = q::get_by_public_id(&db, DEFAULT_ORG_ID, &agent.public_id.to_string())
        .await
        .expect("reload")
        .expect("agent still present");
    assert_eq!(after.system_prompt, "initial prompt");
}

#[tokio::test]
async fn built_in_agent_rejects_archive() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner).await;
    let agent = seed_built_in_agent(&db, &ctx, "chat").await;

    // Archive travels through UpdateAgentCmd as a status change.
    let err = UpdateAgentCmd {
        id: agent.public_id.to_string(),
        req: UpdateAgentRequest {
            service_virtual_user_id: crate::storage::UpdateField::Unchanged,

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
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner).await;
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
        q::get_by_public_id(&db, DEFAULT_ORG_ID, &agent.public_id.to_string())
            .await
            .expect("reload")
            .is_some(),
        "built-in agent must survive both delete verbs"
    );
}

#[tokio::test]
async fn built_in_agent_rejects_upsert() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner).await;
    let agent = seed_built_in_agent(&db, &ctx, "chat").await;

    let err = UpsertAgent {
        replace_capabilities: false,
        id: agent.public_id.to_string(),
        req: basic_agent_request("chat"),
    }
    .execute(&ctx)
    .await
    .expect_err("upsert must not overwrite a built-in");
    assert_built_in_rejection(&err);
}

#[tokio::test]
async fn built_in_agent_can_be_copied() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_with_role(db.clone(), OrgRole::Owner).await;
    let mut request = basic_agent_request("chat");
    request.intro_markdown = Some("Welcome to the platform".into());
    request.short_description = Some("Platform assistant".into());
    request.starters = serde_json::from_value(serde_json::json!([
        {"text": "List agents", "icon": "bot"}
    ]))
    .unwrap();
    let agent = CreateAgent(request).execute(&ctx).await.unwrap();
    db.mark_agent_built_in(DEFAULT_ORG_ID, AgentId::from_uuid(agent.internal_id))
        .await
        .unwrap();

    let copy = CopyAgent {
        id: agent.public_id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect("copy is the escape hatch and must stay open");

    assert_ne!(copy.public_id, agent.public_id);
    assert_eq!(copy.intro_markdown, agent.intro_markdown);
    assert_eq!(copy.short_description, agent.short_description);
    assert_eq!(copy.starters, agent.starters);

    // The copy must be editable, or the escape hatch leads nowhere.
    UpdateAgentCmd {
        id: copy.public_id.to_string(),
        req: UpdateAgentRequest {
            service_virtual_user_id: crate::storage::UpdateField::Unchanged,

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
    let db = Arc::new(StorageBackend::test_database());
    let mut ctx = ctx_with_role(db.clone(), OrgRole::Owner).await;
    ctx.resource_limits.max_agents_per_org = 1;

    seed_built_in_agent(&db, &ctx, "managed-test").await;

    // The built-in must not consume the cap, so a user agent still fits.
    CreateAgent(basic_agent_request("mine"))
        .execute(&ctx)
        .await
        .expect("built-in agent must not consume the org quota");
}

mod branding_slack_tests;

mod lifecycle_slack_tests;

use super::*;
use crate::domains::harnesses::CreateHarness;
use crate::domains::harnesses::types::CreateHarnessRequest;
use crate::storage::StorageBackend;
use everruns_contracts::typed_id::{HarnessId, SessionId};
use everruns_core::{Caller, DEFAULT_ORG_ID, DefaultPermissionResolver, OrgRole};
use std::sync::Arc;
use uuid::Uuid;

#[test]
fn parse_provider_type_accepts_mixed_case_known_values() {
    assert_eq!(parse_provider_type("OpenAI"), Some(DriverId::OpenAI));
    assert_eq!(
        parse_provider_type("AZURE_OPENAI"),
        Some(DriverId::AzureOpenAI)
    );
}

fn test_ctx(db: Arc<StorageBackend>, max_sessions_per_org: i64) -> Ctx {
    let session_service = Arc::new(crate::domains::sessions::SessionService::new(db.clone()));
    let event_service = Arc::new(crate::services::EventService::new(
        db.clone(),
        crate::live_updates::event_delivery::EventDelivery::in_memory(),
    ));
    let capability_service = Arc::new(crate::services::CapabilityService::new(db.clone(), None));
    // Internal caller: the owner principal resolves to the system principal,
    // avoiding a user lookup the in-memory store cannot satisfy.
    let mut ctx = Ctx::new(
        Caller::internal(DEFAULT_ORG_ID),
        db,
        capability_service,
        None,
        Arc::new(DefaultPermissionResolver),
    )
    .with_session_service(session_service)
    .with_event_service(event_service);
    ctx.resource_limits.max_sessions_per_org = max_sessions_per_org;
    ctx
}

fn external_test_ctx(db: Arc<StorageBackend>, user_id: Uuid) -> Ctx {
    let session_service = Arc::new(crate::domains::sessions::SessionService::new(db.clone()));
    let capability_service = Arc::new(crate::services::CapabilityService::new(db.clone(), None));
    Ctx::new(
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: everruns_core::organization::org_public_id_from_internal(DEFAULT_ORG_ID),
            user_id: Some(user_id),
            role: OrgRole::Owner,
            is_platform_user: false,
            is_internal: false,
        },
        db,
        capability_service,
        None,
        Arc::new(DefaultPermissionResolver),
    )
    .with_session_service(session_service)
}

fn create_request(harness_id: HarnessId) -> CreateSessionRequest {
    CreateSessionRequest {
        playground_user_id: None,
        source: None,
        workspace_id: None,
        harness_id: Some(harness_id),
        harness_name: None,
        agent_id: None,
        agent_name: None,
        virtual_user_id: None,
        title: Some("Test Session".to_string()),
        goal: None,
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: vec![],
        sandbox: None,
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
        seed: everruns_core::SessionSeedMode::Fresh,
    }
}

async fn seed_harness(ctx: &Ctx) -> HarnessId {
    CreateHarness(CreateHarnessRequest {
        name: "limit-harness".to_string(),
        display_name: None,
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(ctx)
    .await
    .expect("seed harness")
    .id
}

#[tokio::test]
async fn create_session_enforces_shared_org_rate_limit() {
    let db = Arc::new(StorageBackend::test_database());
    let limiter = crate::auth::rate_limit::OrgRateLimiter::for_test_with_session_rpm(1);
    limiter
        .check_session_create(DEFAULT_ORG_ID)
        .await
        .expect("initial permit");
    let ctx = test_ctx(db, 100).with_org_rate_limiter(limiter);

    let err = CreateSession(create_request(HarnessId::new()))
        .execute(&ctx)
        .await
        .expect_err("exhausted per-org bucket must reject command dispatch");

    assert!(matches!(err.kind, CommandErrorKind::RateLimited(_)));
    assert_eq!(err.code.as_deref(), Some("rate_limited"));
    assert_eq!(err.retry_after_seconds, Some(60));
}

#[tokio::test]
async fn fork_session_enforces_shared_org_rate_limit() {
    // Fork must consume the same per-org bucket as create so MCP dispatch
    // cannot bypass it. The limiter is checked before parent lookup, so an
    // exhausted bucket rejects the fork regardless of the parent id.
    let db = Arc::new(StorageBackend::test_database());
    let limiter = crate::auth::rate_limit::OrgRateLimiter::for_test_with_session_rpm(1);
    limiter
        .check_session_create(DEFAULT_ORG_ID)
        .await
        .expect("initial permit");
    let ctx = test_ctx(db, 100).with_org_rate_limiter(limiter);

    let err = ForkSession {
        session_id: SessionId::new().to_string(),
        overrides: Default::default(),
    }
    .execute(&ctx)
    .await
    .expect_err("exhausted per-org bucket must reject fork dispatch");

    assert!(matches!(err.kind, CommandErrorKind::RateLimited(_)));
    assert_eq!(err.code.as_deref(), Some("rate_limited"));
    assert_eq!(err.retry_after_seconds, Some(60));
}

// Minimal runner whose `cancel_run` succeeds without a real durable backend,
// so `CancelSession` can be exercised against the in-memory store.
struct CancelTestRunner;

#[async_trait::async_trait]
impl everruns_core::host::TurnBackend for CancelTestRunner {
    async fn start_turn(
        &self,
        request: everruns_core::host::TurnRequest,
    ) -> everruns_contracts::error::Result<everruns_core::host::TurnTicket> {
        // The server drops its tickets; this one never resolves.
        Ok(everruns_core::host::TurnTicket::new(
            request.session_id,
            request.turn_id,
            std::future::pending(),
        ))
    }

    async fn cancel(
        &self,
        _session_id: everruns_contracts::typed_id::SessionId,
    ) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }

    async fn is_running(&self, _session_id: everruns_contracts::typed_id::SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

// EVE-708: cancelling an active turn must settle the session back to `idle`.
// The durable workflow is marked cancelled and the worker short-circuits before
// the runtime's idle transition, so `CancelSession` itself must idle the session
// or it stays `active` forever, blocking clean follow-up turns.
#[tokio::test]
async fn cancel_active_session_transitions_to_idle() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = test_ctx(db.clone(), 100).with_runner(Arc::new(CancelTestRunner));
    let harness_id = seed_harness(&ctx).await;

    let session = CreateSession(create_request(harness_id))
        .execute(&ctx)
        .await
        .expect("create session");

    // Simulate an in-flight turn.
    q::session_service(&ctx)
        .unwrap()
        .update_status(&ctx.caller, session.id.uuid(), "active".to_string())
        .await
        .expect("mark active");

    let response = CancelSession {
        session_id: session.id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect("cancel");
    assert!(
        matches!(response.status, CancelStatus::Cancelled),
        "expected an active cancel, got {:?}",
        response.status
    );

    let after = q::get_session(&ctx, session.id, None)
        .await
        .expect("reload session");
    assert_eq!(
        after.status,
        crate::records::SessionStatus::Idle,
        "cancelled session must settle to idle"
    );
}

#[tokio::test]
async fn update_session_title_emits_one_semantic_event_per_change() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = test_ctx(db, 100);
    let harness_id = seed_harness(&ctx).await;
    let session = CreateSession(create_request(harness_id))
        .execute(&ctx)
        .await
        .expect("create session");

    for _ in 0..2 {
        UpdateSessionCmd {
            session_id: session.id.to_string(),
            req: serde_json::from_value(serde_json::json!({"title": "Updated title"}))
                .expect("update request"),
        }
        .execute(&ctx)
        .await
        .expect("update title");
    }

    let events = ctx
        .event_service
        .as_ref()
        .expect("event service")
        .list(
            session.id.uuid(),
            None,
            None,
            &[everruns_core::SESSION_TITLE_UPDATED.to_string()],
            &[],
            None,
            None,
        )
        .await
        .expect("list events");
    assert_eq!(events.len(), 1);
    assert!(events[0].context.turn_id.is_none());
    match &events[0].data {
        EventData::SessionTitleUpdated(data) => {
            assert_eq!(data.previous_title.as_deref(), Some("Test Session"));
            assert_eq!(data.title, "Updated title");
        }
        data => panic!("unexpected event data: {data:?}"),
    }
}

#[tokio::test]
async fn session_creation_rejected_at_limit_and_allowed_below() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = test_ctx(db, 1);
    let harness_id = seed_harness(&ctx).await;

    CreateSession(create_request(harness_id))
        .execute(&ctx)
        .await
        .expect("first session below limit");

    let err = CreateSession(create_request(harness_id))
        .execute(&ctx)
        .await
        .expect_err("second session exceeds the cap");
    assert_eq!(err.status().as_u16(), 409);
    assert!(err.message().contains("Session limit reached"));
}

#[tokio::test]
async fn create_session_with_non_internal_owner_sets_parent_session_id() {
    // Trusted subagent/handoff spawns dispatch CreateSession through
    // DirectPlatformStore, whose caller runs as the session owner
    // (`is_internal == false`, see caller_resolution.rs). The internal
    // parent link must survive that path. Forgery by untrusted clients is
    // prevented at the HTTP boundary, which strips `parent_session_id`
    // before dispatch (see `strip_internal_only_fields` in api/sessions/mod.rs),
    // so the command layer must not reject a caller-set parent link.
    let db = Arc::new(StorageBackend::test_database());
    let internal_ctx = test_ctx(db.clone(), 10);
    let harness_id = seed_harness(&internal_ctx).await;
    let owner = db
        .create_user(crate::storage::CreateUserRow {
            email: format!("owner-{}@example.com", Uuid::now_v7()),
            name: "Owner".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .expect("create owner user");
    db.add_organization_member(DEFAULT_ORG_ID, owner.id, "owner")
        .await
        .unwrap();
    let parent = db.create_test_session().await;
    let ctx = external_test_ctx(db, owner.id);
    let mut req = create_request(harness_id);
    req.parent_session_id = Some(parent);

    let session = CreateSession(req)
        .execute(&ctx)
        .await
        .expect("owner-caller spawn with a parent link should succeed");

    assert_eq!(session.parent_session_id, Some(parent));
}

async fn seed_agent(ctx: &Ctx, harness_id: HarnessId, name: &str) -> AgentId {
    let public_id = format!("agent_{}", Uuid::now_v7().simple());
    let row = ctx
        .db
        .create_agent(
            ctx.org_id(),
            crate::storage::CreateAgentRow {
                public_id: public_id.clone(),
                name: name.to_string(),
                display_name: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: "You are helpful.".to_string(),
                default_model_id: None,
                harness_id,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .expect("seed agent");
    row.public_id.parse().expect("agent public id")
}

#[tokio::test]
async fn create_session_from_agent_inherits_agent_harness() {
    // Agent-first: a session created with only an agent runs on the agent's
    // own harness (D4), and an explicit request harness still overrides it.
    let db = Arc::new(StorageBackend::test_database());
    let ctx = test_ctx(db.clone(), 10);
    let agent_harness = seed_harness(&ctx).await;
    let override_harness = CreateHarness(CreateHarnessRequest {
        name: "override-harness".to_string(),
        display_name: None,
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .expect("seed override harness")
    .id;
    let agent_id = seed_agent(&ctx, agent_harness, "support").await;

    // 1. agent only, no harness → inherits the agent's harness.
    let mut req = create_request(agent_harness);
    req.harness_id = None;
    req.agent_id = Some(agent_id);
    let session = CreateSession(req)
        .execute(&ctx)
        .await
        .expect("create session from agent");
    assert_eq!(session.harness_id, agent_harness);

    // 2. explicit request harness overrides the agent's harness.
    let mut req = create_request(override_harness);
    req.agent_id = Some(agent_id);
    let session = CreateSession(req)
        .execute(&ctx)
        .await
        .expect("create session with explicit harness override");
    assert_eq!(session.harness_id, override_harness);
}

#[tokio::test]
async fn create_session_rejects_both_agent_id_and_agent_name() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = test_ctx(db, 10);
    let harness_id = seed_harness(&ctx).await;
    let mut req = create_request(harness_id);
    req.agent_id = Some(AgentId::new());
    req.agent_name = Some("support".to_string());

    let err = CreateSession(req)
        .execute(&ctx)
        .await
        .expect_err("agent_id + agent_name is rejected");
    assert_eq!(err.status().as_u16(), 400);
}

#[tokio::test]
async fn participant_commands_list_add_and_leave_history() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = test_ctx(db.clone(), 10);
    let harness_id = seed_harness(&ctx).await;
    let host_public_id = AgentId::new();
    let host_agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            crate::storage::CreateAgentRow {
                public_id: host_public_id.to_string(),
                name: "participant-host".to_string(),
                display_name: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: "You are helpful.".to_string(),
                default_model_id: None,
                harness_id,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .expect("create host agent");
    let member_public_id = AgentId::new();
    let member_agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            crate::storage::CreateAgentRow {
                public_id: member_public_id.to_string(),
                name: "participant-member".to_string(),
                display_name: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: "You are helpful.".to_string(),
                default_model_id: None,
                harness_id,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .expect("create member agent");

    let mut req = create_request(harness_id);
    req.agent_id = Some(host_public_id);
    let session = CreateSession(req)
        .execute(&ctx)
        .await
        .expect("create session with host agent");

    let initial = ListSessionParticipants {
        session_id: session.id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect("list initial participants");
    assert_eq!(initial.len(), 2);
    let host = initial
        .iter()
        .find(|participant| participant.role == SessionParticipantRole::Host)
        .expect("host participant");
    assert_eq!(host.kind, SessionParticipantKind::Agent);
    assert_eq!(host.agent_id, Some(host_agent.id));

    let user_again = AddSessionParticipant {
        session_id: session.id.to_string(),
        req: AddSessionParticipantRequest {
            kind: SessionParticipantKind::User,
            agent_id: None,
            role: None,
        },
    }
    .execute(&ctx)
    .await
    .expect("user participant add is idempotent");
    assert_eq!(user_again.kind, SessionParticipantKind::User);
    assert_eq!(
        initial
            .iter()
            .filter(|p| p.kind == SessionParticipantKind::User)
            .count(),
        1
    );

    let added = AddSessionParticipant {
        session_id: session.id.to_string(),
        req: AddSessionParticipantRequest {
            kind: SessionParticipantKind::Agent,
            agent_id: Some(member_public_id),
            role: None,
        },
    }
    .execute(&ctx)
    .await
    .expect("add member participant");
    assert_eq!(added.role, SessionParticipantRole::Member);
    assert_eq!(added.agent_id, Some(member_agent.id));

    let duplicate_agent = AddSessionParticipant {
        session_id: session.id.to_string(),
        req: AddSessionParticipantRequest {
            kind: SessionParticipantKind::Agent,
            agent_id: Some(member_public_id),
            role: None,
        },
    }
    .execute(&ctx)
    .await
    .expect_err("duplicate active agent participant is rejected");
    assert_eq!(duplicate_agent.status().as_u16(), 409);

    let left = LeaveSessionParticipant {
        session_id: session.id.to_string(),
        participant_id: added.id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect("leave member participant");
    assert!(left.left_at.is_some());

    let history = ListSessionParticipants {
        session_id: session.id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect("list participant history");
    assert_eq!(history.len(), 3);
    assert!(
        history
            .iter()
            .find(|participant| participant.id == added.id)
            .and_then(|participant| participant.left_at)
            .is_some()
    );

    let err = LeaveSessionParticipant {
        session_id: session.id.to_string(),
        participant_id: host.id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect_err("host cannot leave through member endpoint");
    assert_eq!(err.status().as_u16(), 409);
}

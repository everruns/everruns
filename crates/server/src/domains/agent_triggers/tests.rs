// Unit tests for agent-trigger commands: cron validation, per-org cap, and
// durable-binding create/teardown on enable/disable/delete. Mirrors the
// app schedule-channel tests but with no DB dependency (in-memory storage +
// in-memory workflow store).

use super::*;
use crate::domains::agent_triggers::types::{CreateAgentTriggerRequest, UpdateAgentTriggerRequest};
use crate::domains::common::Ctx;
use crate::event_delivery::EventDelivery;
use crate::storage::StorageBackend;
use crate::storage::models::{CreateAgentRow, CreateHarnessRow, CreateSessionRow};
use async_trait::async_trait;
use everruns_core::{Caller, DEFAULT_ORG_ID};
use everruns_durable::InMemoryWorkflowEventStore;
use everruns_platform::InvocationSessionMode;
use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, SessionId};
use everruns_scale::RunController;
use std::sync::{Arc, Mutex};

// Serializes the env-mutating cap tests.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Default)]
struct RecordingRunner {
    harness_ids: Mutex<Vec<HarnessId>>,
}

#[async_trait]
impl RunController for RecordingRunner {
    async fn start_run(
        &self,
        _org_id: i64,
        _session_id: SessionId,
        harness_id: HarnessId,
        _agent_id: Option<AgentId>,
        _input_message_id: MessageId,
        _request_id: Option<String>,
    ) -> anyhow::Result<()> {
        self.harness_ids.lock().unwrap().push(harness_id);
        Ok(())
    }

    async fn resume_after_tool_results(&self, _session_id: SessionId) -> anyhow::Result<()> {
        Ok(())
    }

    async fn cancel_run(&self, _session_id: SessionId) -> anyhow::Result<()> {
        Ok(())
    }

    async fn is_running(&self, _session_id: SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        self.harness_ids.lock().unwrap().len()
    }
}

async fn seed_agent(db: &Arc<StorageBackend>) -> (String, everruns_provider::typed_id::HarnessId) {
    let harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: "trigger-harness".to_string(),
                display_name: Some("Trigger Harness".to_string()),
                description: None,
                system_prompt: Some(String::new()),
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
        .expect("create harness");
    let public_id = AgentId::new().to_string();
    db.create_agent(
        DEFAULT_ORG_ID,
        CreateAgentRow {
            public_id: public_id.clone(),
            name: "trigger-agent".to_string(),
            display_name: Some("Trigger Agent".to_string()),
            description: None,
            system_prompt: String::new(),
            default_model_id: None,
            harness_id: harness.id,
            tags: vec![],
            initial_files: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            is_built_in: false,
        },
    )
    .await
    .expect("create agent");
    (public_id, harness.id)
}

fn test_ctx(db: Arc<StorageBackend>, store: Arc<InMemoryWorkflowEventStore>) -> Ctx {
    Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db, None)
        .with_workflow_store(Some(store))
}

fn create_req(cron: &str, message: &str, enabled: bool) -> CreateAgentTriggerRequest {
    CreateAgentTriggerRequest {
        cron_expression: cron.to_string(),
        timezone: "UTC".to_string(),
        session_mode: InvocationSessionMode::SharedSession,
        message: message.to_string(),
        enabled,
    }
}

#[tokio::test]
async fn resolve_trigger_execution_context_preserves_migrated_app_context() {
    let db = Arc::new(StorageBackend::in_memory());
    let agent_harness_id = everruns_provider::typed_id::HarnessId::from_seed(10);
    let app_harness_id = everruns_provider::typed_id::HarnessId::from_seed(20);
    let owner_principal_id = everruns_provider::typed_id::PrincipalId::from_seed(30);
    let resolved_owner_user_id = Some(uuid::Uuid::from_u128(40));
    let agent_identity_id = Some(everruns_provider::typed_id::AgentIdentityId::from_uuid(
        uuid::Uuid::from_u128(50),
    ));
    let app_id = Some(uuid::Uuid::from_u128(60));
    let now = chrono::Utc::now();
    let agent = crate::storage::models::AgentRow {
        id: AgentId::from_uuid(uuid::Uuid::from_u128(70)),
        public_id: AgentId::from_uuid(uuid::Uuid::from_u128(71)).to_string(),
        org_id: DEFAULT_ORG_ID,
        name: "trigger-agent".to_string(),
        display_name: None,
        description: None,
        system_prompt: String::new(),
        default_model_id: None,
        harness_id: agent_harness_id,
        harness_source: "explicit".to_string(),
        agent_identity_id: None,
        default_version_id: None,
        forked_from_agent_id: None,
        forked_from_version_id: None,
        root_agent_id: None,
        tags: vec![],
        status: "active".to_string(),
        created_at: now,
        updated_at: now,
        archived_at: None,
        deleted_at: None,
        initial_files: serde_json::json!([]),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!({}),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        total_input_tokens: 0,
        total_output_tokens: 0,
        total_cache_read_tokens: 0,
        total_cache_creation_tokens: 0,
        total_actual_cost_usd: 0.0,
        total_estimated_cost_usd: 0.0,
        total_cost_usd: 0.0,
        is_built_in: false,
    };
    let trigger = crate::storage::models::AgentTriggerRow {
        id: everruns_provider::typed_id::TriggerId::from_uuid(uuid::Uuid::from_u128(80)),
        org_id: DEFAULT_ORG_ID,
        agent_id: agent.id,
        trigger_type: "schedule".to_string(),
        config: serde_json::json!({}),
        enabled: true,
        durable_schedule_id: None,
        execution_harness_id: Some(app_harness_id),
        execution_owner_principal_id: Some(owner_principal_id),
        execution_resolved_owner_user_id: resolved_owner_user_id,
        execution_agent_identity_id: agent_identity_id,
        execution_app_id: app_id,
        status: "active".to_string(),
        created_at: now,
        updated_at: now,
        archived_at: None,
        deleted_at: None,
    };

    let context = resolve_trigger_execution_context(&db, DEFAULT_ORG_ID, &agent, &trigger)
        .await
        .expect("resolve migrated context");

    assert_eq!(context.harness_id, app_harness_id);
    assert_eq!(context.owner_principal_id, owner_principal_id);
    assert_eq!(context.resolved_owner_user_id, resolved_owner_user_id);
    assert_eq!(context.agent_identity_id, agent_identity_id);
    assert_eq!(context.app_id, app_id);
}

#[tokio::test]
async fn dispatch_trigger_message_uses_preserved_harness() {
    let db = Arc::new(StorageBackend::in_memory());
    let (agent_public_id, _) = seed_agent(&db).await;
    let agent = db
        .get_agent_by_public_id(DEFAULT_ORG_ID, &agent_public_id)
        .await
        .unwrap()
        .unwrap();
    let preserved_harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: "preserved-app-harness".to_string(),
                display_name: None,
                description: None,
                system_prompt: Some(String::new()),
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
    let (_, owner) = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent)
        .await
        .unwrap();
    let session = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            harness_id: Some(preserved_harness.id),
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: owner.id,
            resolved_owner_user_id: owner.resolved_user_id,
            title: Some("preserved app trigger".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
            workspace_id: None,
        })
        .await
        .unwrap();
    assert_eq!(session.harness_id, Some(preserved_harness.id));

    let runner = Arc::new(RecordingRunner::default());
    let message_service =
        MessageService::new(db, runner.clone(), false, EventDelivery::in_memory());
    dispatch_trigger_message(
        &message_service,
        DEFAULT_ORG_ID,
        &agent,
        TriggerId::new(),
        session.id,
        preserved_harness.id,
        owner.id,
        "scheduled message".to_string(),
    )
    .await
    .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while runner.harness_ids.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("runner called");
    assert_eq!(
        *runner.harness_ids.lock().unwrap(),
        vec![preserved_harness.id]
    );
}

// ---- cron / config validation -------------------------------------------

#[test]
fn validate_schedule_config_accepts_daily_cron() {
    let normalized = validate_schedule_config("0 9 * * *", "hello").expect("valid daily cron");
    // 5-field input is normalized to 7-field (sec … year).
    assert_eq!(normalized, "0 0 9 * * * *");
}

#[test]
fn validate_schedule_config_rejects_empty_message() {
    let err = validate_schedule_config("0 9 * * *", "   ").unwrap_err();
    assert!(err.message().contains("non-empty message"), "got: {err}");
}

#[test]
fn validate_schedule_config_rejects_too_frequent() {
    // Every minute is below the 300s default minimum interval.
    let err = validate_schedule_config("* * * * *", "hi").unwrap_err();
    assert!(err.message().contains("no more than once"), "got: {err}");
}

#[test]
fn validate_schedule_config_rejects_garbage_cron() {
    let err = validate_schedule_config("not a cron", "hi").unwrap_err();
    assert!(err.message().to_lowercase().contains("cron"), "got: {err}");
}

// ---- per-org enabled cap ------------------------------------------------

#[tokio::test]
#[allow(clippy::await_holding_lock)] // env guard intentionally spans the awaits
async fn create_enforces_per_org_enabled_cap() {
    let _guard = ENV_LOCK.lock().unwrap();
    // Safety: ENV_LOCK serializes env-mutating tests in this module.
    unsafe { std::env::set_var("AGENT_TRIGGER_MAX_PER_ORG", "1") };

    let db = Arc::new(StorageBackend::in_memory());
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let ctx = test_ctx(db.clone(), store);
    let (agent_id, _) = seed_agent(&db).await;

    CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: create_req("0 9 * * *", "first", true),
    }
    .execute(&ctx)
    .await
    .expect("first enabled trigger fits under the cap");

    let err = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: create_req("0 10 * * *", "second", true),
    }
    .execute(&ctx)
    .await
    .expect_err("second enabled trigger exceeds the cap");
    assert!(err.message().contains("at most 1"), "got: {err}");

    // A disabled trigger is not counted against the cap.
    CreateAgentTrigger {
        agent_id,
        req: create_req("0 11 * * *", "disabled", false),
    }
    .execute(&ctx)
    .await
    .expect("disabled trigger is exempt from the enabled cap");

    unsafe { std::env::remove_var("AGENT_TRIGGER_MAX_PER_ORG") };
}

#[tokio::test]
async fn responses_use_public_agent_id_not_internal_fk() {
    let db = Arc::new(StorageBackend::in_memory());
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let ctx = test_ctx(db.clone(), store);
    let (agent_id, _) = seed_agent(&db).await;
    let public_agent_id: AgentId = agent_id.parse().expect("public agent id parses");
    let internal_agent_id = db
        .get_agent_by_public_id(DEFAULT_ORG_ID, &agent_id)
        .await
        .unwrap()
        .expect("agent row")
        .id;
    assert_ne!(
        public_agent_id, internal_agent_id,
        "test must exercise distinct public and internal agent IDs"
    );

    let created = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: create_req("0 9 * * *", "hello", true),
    }
    .execute(&ctx)
    .await
    .expect("create trigger");
    assert_eq!(created.agent_id, public_agent_id);

    let listed = ListAgentTriggers {
        agent_id: agent_id.clone(),
        include_archived: false,
    }
    .execute(&ctx)
    .await
    .expect("list triggers");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].agent_id, public_agent_id);

    let fetched = GetAgentTrigger {
        agent_id,
        trigger_id: created.id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect("get trigger");
    assert_eq!(fetched.agent_id, public_agent_id);
}

// ---- durable binding create / teardown ----------------------------------

#[tokio::test]
async fn binding_created_on_enabled_create() {
    let db = Arc::new(StorageBackend::in_memory());
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let ctx = test_ctx(db.clone(), store.clone());
    let (agent_id, _) = seed_agent(&db).await;

    let trigger = CreateAgentTrigger {
        agent_id,
        req: create_req("0 9 * * *", "hello", true),
    }
    .execute(&ctx)
    .await
    .expect("create enabled trigger");

    let trigger_id: TriggerId = trigger.id;
    let row = db
        .get_agent_trigger(DEFAULT_ORG_ID, trigger_id)
        .await
        .unwrap()
        .expect("trigger row");
    let schedule_id = row.durable_schedule_id.expect("durable schedule bound");
    let schedule = store.get_schedule(schedule_id).await.expect("schedule row");
    assert!(
        schedule.enabled,
        "enabled trigger binds an enabled schedule"
    );
    assert_eq!(schedule.target_name, "invoke_agent_trigger");
}

#[tokio::test]
async fn recent_runs_use_the_trigger_schedule_execution_history() {
    let db = Arc::new(StorageBackend::in_memory());
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let ctx = test_ctx(db.clone(), store.clone());
    let (agent_id, _) = seed_agent(&db).await;

    let trigger = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: create_req("0 9 * * *", "hello", true),
    }
    .execute(&ctx)
    .await
    .expect("create trigger");
    let schedule_id = db
        .get_agent_trigger(DEFAULT_ORG_ID, trigger.id)
        .await
        .unwrap()
        .unwrap()
        .durable_schedule_id
        .expect("schedule bound");
    let execution_id = store
        .create_schedule_execution(schedule_id, Utc::now())
        .await
        .expect("create execution");
    store
        .complete_schedule_execution(execution_id, Uuid::now_v7(), false)
        .await
        .expect("complete execution");

    let runs = ListAgentTriggerRuns {
        agent_id,
        trigger_id: trigger.id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect("list recent runs");

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, execution_id.to_string());
    assert_eq!(runs[0].status, "completed");
    assert!(runs[0].completed_at.is_some());
}

#[tokio::test]
async fn binding_absent_on_disabled_create() {
    let db = Arc::new(StorageBackend::in_memory());
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let ctx = test_ctx(db.clone(), store.clone());
    let (agent_id, _) = seed_agent(&db).await;

    let trigger = CreateAgentTrigger {
        agent_id,
        req: create_req("0 9 * * *", "hello", false),
    }
    .execute(&ctx)
    .await
    .expect("create disabled trigger");

    let row = db
        .get_agent_trigger(DEFAULT_ORG_ID, trigger.id)
        .await
        .unwrap()
        .expect("trigger row");
    // A disabled trigger still binds a (disabled) durable schedule so it can be
    // flipped on later; assert the schedule exists but is not enabled.
    let schedule_id = row.durable_schedule_id.expect("durable schedule bound");
    let schedule = store.get_schedule(schedule_id).await.expect("schedule row");
    assert!(
        !schedule.enabled,
        "disabled trigger binds a disabled schedule"
    );
}

#[tokio::test]
async fn binding_torn_down_on_disable() {
    let db = Arc::new(StorageBackend::in_memory());
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let ctx = test_ctx(db.clone(), store.clone());
    let (agent_id, _) = seed_agent(&db).await;

    let trigger = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: create_req("0 9 * * *", "hello", true),
    }
    .execute(&ctx)
    .await
    .expect("create enabled trigger");
    let schedule_id = db
        .get_agent_trigger(DEFAULT_ORG_ID, trigger.id)
        .await
        .unwrap()
        .unwrap()
        .durable_schedule_id
        .expect("schedule bound");

    UpdateAgentTriggerCmd {
        agent_id,
        trigger_id: trigger.id.to_string(),
        req: UpdateAgentTriggerRequest {
            enabled: Some(false),
            ..Default::default()
        },
    }
    .execute(&ctx)
    .await
    .expect("disable trigger");

    let row = db
        .get_agent_trigger(DEFAULT_ORG_ID, trigger.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        row.durable_schedule_id.is_none(),
        "binding cleared on disable"
    );
    assert!(
        store.get_schedule(schedule_id).await.is_err(),
        "durable schedule deleted on disable"
    );
}

#[tokio::test]
async fn binding_torn_down_on_delete() {
    let db = Arc::new(StorageBackend::in_memory());
    let store = Arc::new(InMemoryWorkflowEventStore::new());
    let ctx = test_ctx(db.clone(), store.clone());
    let (agent_id, _) = seed_agent(&db).await;

    let trigger = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: create_req("0 9 * * *", "hello", true),
    }
    .execute(&ctx)
    .await
    .expect("create enabled trigger");
    let schedule_id = db
        .get_agent_trigger(DEFAULT_ORG_ID, trigger.id)
        .await
        .unwrap()
        .unwrap()
        .durable_schedule_id
        .expect("schedule bound");

    DeleteAgentTrigger {
        agent_id,
        trigger_id: trigger.id.to_string(),
    }
    .execute(&ctx)
    .await
    .expect("delete trigger");

    assert!(
        store.get_schedule(schedule_id).await.is_err(),
        "durable schedule deleted on archive"
    );
    // Archived trigger no longer appears in the default (non-archived) list.
    let active = db
        .list_agent_triggers(DEFAULT_ORG_ID, None, false)
        .await
        .unwrap();
    assert!(
        active.is_empty(),
        "archived trigger excluded from active list"
    );
}

// ---- ensure_identity_for_agent (EVE-758) --------------------------------

/// Full `AgentRow` for identity tests (the helper above returns only ids).
async fn seed_agent_row(db: &Arc<StorageBackend>) -> crate::storage::models::AgentRow {
    let (public_id, _) = seed_agent(db).await;
    db.get_agent_by_public_id(DEFAULT_ORG_ID, &public_id)
        .await
        .unwrap()
        .expect("seeded agent row")
}

#[tokio::test]
async fn ensure_identity_for_agent_creates_and_links_when_none() {
    let db = Arc::new(StorageBackend::in_memory());
    let agent = seed_agent_row(&db).await;
    assert!(agent.agent_identity_id.is_none());

    let (identity_id, owner) = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent)
        .await
        .expect("lazily create identity");

    // The trigger session owner is the agent's own identity principal.
    assert_eq!(owner.kind, "agent_identity");
    // The agent row is now linked to the freshly-created identity.
    let linked = db
        .get_agent(DEFAULT_ORG_ID, agent.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(linked.agent_identity_id, Some(identity_id));
}

#[tokio::test]
async fn ensure_identity_for_agent_is_idempotent_across_fires() {
    let db = Arc::new(StorageBackend::in_memory());
    let agent = seed_agent_row(&db).await;

    let (first, _) = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent)
        .await
        .expect("first fire");
    // Second fire re-reads the (now linked) agent, mirroring the real path.
    let agent = db
        .get_agent(DEFAULT_ORG_ID, agent.id)
        .await
        .unwrap()
        .unwrap();
    let (second, _) = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent)
        .await
        .expect("second fire");

    assert_eq!(first, second, "same identity reused on subsequent fires");
}

#[tokio::test]
async fn ensure_identity_for_agent_never_overrides_explicit_identity() {
    use crate::storage::models::CreateAgentIdentityRow;
    use everruns_provider::typed_id::AgentIdentityId;

    let db = Arc::new(StorageBackend::in_memory());
    let mut agent = seed_agent_row(&db).await;

    let explicit = AgentIdentityId::new();
    db.create_agent_identity(CreateAgentIdentityRow {
        org_id: DEFAULT_ORG_ID,
        id: explicit,
        name: "Explicit".to_string(),
        description: None,
        avatar_url: None,
        locale: None,
        timezone: None,
    })
    .await
    .unwrap();
    assert!(
        db.set_agent_identity_id(DEFAULT_ORG_ID, agent.id, explicit)
            .await
            .unwrap()
    );
    agent.agent_identity_id = Some(explicit);

    let (identity_id, owner) = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent)
        .await
        .expect("resolve explicit identity");

    assert_eq!(identity_id, explicit, "explicit identity is returned as-is");
    assert_eq!(owner.kind, "agent_identity");
    let linked = db
        .get_agent(DEFAULT_ORG_ID, agent.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        linked.agent_identity_id,
        Some(explicit),
        "explicit identity is never overridden"
    );

    // A guarded set on an already-linked agent is a no-op.
    assert!(
        !db.set_agent_identity_id(DEFAULT_ORG_ID, agent.id, AgentIdentityId::new())
            .await
            .unwrap(),
        "set_agent_identity_id refuses to override an existing link"
    );
}

#[tokio::test]
async fn ensure_identity_for_agent_rejects_archived_linked_identity() {
    use crate::storage::models::CreateAgentIdentityRow;
    use everruns_provider::typed_id::AgentIdentityId;

    let db = Arc::new(StorageBackend::in_memory());
    let mut agent = seed_agent_row(&db).await;

    let identity_id = AgentIdentityId::new();
    db.create_agent_identity(CreateAgentIdentityRow {
        org_id: DEFAULT_ORG_ID,
        id: identity_id,
        name: "Archived".to_string(),
        description: None,
        avatar_url: None,
        locale: None,
        timezone: None,
    })
    .await
    .unwrap();
    db.set_agent_identity_id(DEFAULT_ORG_ID, agent.id, identity_id)
        .await
        .unwrap();
    db.delete_agent_identity(DEFAULT_ORG_ID, identity_id)
        .await
        .unwrap();
    agent.agent_identity_id = Some(identity_id);

    let err = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent)
        .await
        .expect_err("archived identity must not own new trigger sessions");
    assert!(
        err.to_string().contains("is not active"),
        "unexpected error: {err:#}"
    );
}

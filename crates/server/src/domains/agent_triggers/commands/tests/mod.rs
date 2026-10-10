// Unit tests for agent-trigger commands: cron validation, per-org cap, and
// durable-binding create/teardown on enable/disable/delete. Mirrors the
// app schedule-channel tests but with no DB dependency (in-memory storage +
// in-memory workflow store).

use super::*;
use crate::domains::agent_triggers::deliveries::ListAgentTriggerDeliveries;
use crate::domains::agent_triggers::types::{CreateAgentTriggerRequest, UpdateAgentTriggerRequest};
use crate::domains::agent_triggers::webhook_invocation::{
    WebhookTriggerInvocationRequest, invoke_webhook_agent_trigger,
};
use crate::domains::common::Ctx;
use crate::live_updates::event_delivery::EventDelivery;
use crate::storage::StorageBackend;
use crate::storage::{CreateAgentRow, CreateHarnessRow, CreateSessionRow};
use async_trait::async_trait;
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};
use everruns_core::channel::SessionBinding;
use everruns_core::host::{TurnBackend, TurnRequest, TurnTicket};
use everruns_core::{Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole};
use everruns_durable::{PostgresWorkflowEventStore, Schedules};
use std::sync::{Arc, Mutex};

mod script_target_tests;

// Serializes the env-mutating cap tests.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Default)]
struct RecordingRunner {
    harness_ids: Mutex<Vec<HarnessId>>,
}

#[async_trait]
impl TurnBackend for RecordingRunner {
    async fn start_turn(
        &self,
        request: TurnRequest,
    ) -> everruns_contracts::error::Result<TurnTicket> {
        if let Some(scope) = request.scope {
            self.harness_ids.lock().unwrap().push(scope.harness_id);
        }
        // The server drops its tickets; this one never resolves.
        Ok(TurnTicket::new(
            request.session_id,
            request.turn_id,
            std::future::pending(),
        ))
    }

    async fn cancel(&self, _session_id: SessionId) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }

    async fn is_running(&self, _session_id: SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        self.harness_ids.lock().unwrap().len()
    }
}

async fn seed_agent(db: &Arc<StorageBackend>) -> (String, everruns_contracts::typed_id::HarnessId) {
    let harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: "trigger-harness".to_string(),
                display_name: Some("Trigger Harness".to_string()),
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
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
            intro_markdown: None,
            short_description: None,
            starters: serde_json::json!([]),
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
            communication: Default::default(),
            environments: None,
            is_built_in: false,
        },
    )
    .await
    .expect("create agent");
    (public_id, harness.id)
}

fn workflow_store(db: &StorageBackend) -> Arc<PostgresWorkflowEventStore> {
    PostgresWorkflowEventStore::new(db.database().pool().clone()).into()
}

fn test_ctx(db: Arc<StorageBackend>, store: Arc<PostgresWorkflowEventStore>) -> Ctx {
    Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db, None)
        .with_workflow_store(Some(store))
}

fn role_ctx(db: Arc<StorageBackend>, role: OrgRole) -> Ctx {
    let encryption = crate::storage::encryption::EncryptionService::new(
        &crate::storage::encryption::generate_encryption_key("test"),
        &[],
    )
    .expect("test encryption service");
    Ctx::minimal_for_test(
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            user_id: Some(uuid::Uuid::nil()),
            role,
            is_platform_user: false,
            is_internal: false,
        },
        db,
        Some(Arc::new(encryption)),
    )
}

fn webhook_req(enabled: bool) -> CreateAgentTriggerRequest {
    CreateAgentTriggerRequest {
        trigger_type: AgentTriggerType::Webhook,
        cron_expression: None,
        timezone: "UTC".to_string(),
        session_mode: SessionBinding::Shared,
        message: "Webhook: {{body}}".to_string(),
        script: None,
        token: Some("secret".to_string()),
        rate_limit_per_minute: None,
        event_id_template: None,
        subject_template: None,
        filter: None,
        github_events: None,
        repositories: None,
        mcp_server: None,
        mcp_event: None,
        mcp_event_arguments: None,
        auth: None,
        enabled,
    }
}

#[tokio::test]
async fn webhook_publication_requires_dangerous_permission() {
    for role in [OrgRole::Member, OrgRole::Admin] {
        let db = Arc::new(StorageBackend::test_database());
        let (agent_id, _) = seed_agent(&db).await;
        let ctx = role_ctx(db, role);

        let error = CreateAgentTrigger {
            agent_id,
            req: webhook_req(true),
        }
        .run(&ctx)
        .await
        .expect_err("non-owner must not publish a webhook");
        assert_eq!(error.status(), axum::http::StatusCode::FORBIDDEN);
    }

    let db = Arc::new(StorageBackend::test_database());
    let (agent_id, _) = seed_agent(&db).await;
    let member_ctx = role_ctx(db.clone(), OrgRole::Member);
    let disabled = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: webhook_req(false),
    }
    .run(&member_ctx)
    .await
    .expect("member may configure a disabled webhook");
    let error = UpdateAgentTriggerCmd {
        agent_id: agent_id.clone(),
        trigger_id: disabled.id.to_string(),
        req: UpdateAgentTriggerRequest {
            enabled: Some(true),
            ..Default::default()
        },
    }
    .run(&member_ctx)
    .await
    .expect_err("member must not enable a webhook");
    assert_eq!(error.status(), axum::http::StatusCode::FORBIDDEN);

    let mut owner_ctx = member_ctx.clone();
    owner_ctx.caller.role = OrgRole::Owner;
    UpdateAgentTriggerCmd {
        agent_id,
        trigger_id: disabled.id.to_string(),
        req: UpdateAgentTriggerRequest {
            enabled: Some(true),
            ..Default::default()
        },
    }
    .run(&owner_ctx)
    .await
    .expect("owner may publish a webhook");
}

fn create_req(cron: &str, message: &str, enabled: bool) -> CreateAgentTriggerRequest {
    CreateAgentTriggerRequest {
        trigger_type: AgentTriggerType::Schedule,
        cron_expression: Some(cron.to_string()),
        timezone: "UTC".to_string(),
        session_mode: SessionBinding::Shared,
        message: message.to_string(),
        script: None,
        token: None,
        rate_limit_per_minute: None,
        event_id_template: None,
        subject_template: None,
        filter: None,
        github_events: None,
        repositories: None,
        mcp_server: None,
        mcp_event: None,
        mcp_event_arguments: None,
        auth: None,
        enabled,
    }
}

#[tokio::test]
async fn resolve_trigger_execution_context_preserves_migrated_app_context() {
    let db = Arc::new(StorageBackend::test_database());
    let agent_harness_id = everruns_contracts::typed_id::HarnessId::from_seed(10);
    let app_harness_id = everruns_contracts::typed_id::HarnessId::from_seed(20);
    let owner_principal_id = everruns_contracts::typed_id::PrincipalId::from_seed(30);
    let resolved_owner_user_id = Some(uuid::Uuid::from_u128(40));
    let virtual_user_id = Some(everruns_contracts::typed_id::VirtualUserId::from_uuid(
        uuid::Uuid::from_u128(50),
    ));
    let app_id = Some(uuid::Uuid::from_u128(60));
    let now = chrono::Utc::now();
    let agent = crate::storage::AgentRow {
        avatar_id: None,
        id: AgentId::from_uuid(uuid::Uuid::from_u128(70)),
        public_id: AgentId::from_uuid(uuid::Uuid::from_u128(71)).to_string(),
        org_id: DEFAULT_ORG_ID,
        name: "trigger-agent".to_string(),
        display_name: None,
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: serde_json::json!([]),
        system_prompt: String::new(),
        default_model_id: None,
        harness_id: agent_harness_id,
        harness_source: "explicit".to_string(),
        virtual_user_id: None,
        forked_from_agent_id: None,
        root_agent_id: None,
        tags: vec![],
        status: "active".to_string(),
        exposures_suspended: false,
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
        communication: String::new(),
        environments: None,
        total_input_tokens: 0,
        total_output_tokens: 0,
        total_cache_read_tokens: 0,
        total_cache_creation_tokens: 0,
        total_actual_cost_usd: 0.0,
        total_estimated_cost_usd: 0.0,
        total_cost_usd: 0.0,
        is_built_in: false,
    };
    let trigger = crate::storage::AgentTriggerRow {
        id: everruns_contracts::typed_id::TriggerId::from_uuid(uuid::Uuid::from_u128(80)),
        org_id: DEFAULT_ORG_ID,
        agent_id: agent.id,
        trigger_type: "schedule".to_string(),
        ingress_id: None,
        config: serde_json::json!({}),
        config_encrypted: None,
        enabled: true,
        durable_schedule_id: None,
        execution_harness_id: Some(app_harness_id),
        execution_owner_principal_id: Some(owner_principal_id),
        execution_resolved_owner_user_id: resolved_owner_user_id,
        execution_virtual_user_id: virtual_user_id,
        execution_app_id: app_id,
        legacy_alias_id: Some("app_frozen".to_string()),
        legacy_alias_name: Some("Frozen App".to_string()),
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
    assert_eq!(context.virtual_user_id, virtual_user_id);
    assert_eq!(context.app_id, app_id);
}

#[tokio::test]
async fn dispatch_trigger_message_uses_preserved_harness() {
    let db = Arc::new(StorageBackend::test_database());
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
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
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
            playground_user_id: None,
            source: crate::domains::sessions::record::SessionSource::Api,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            channel_id: None,
            trigger_id: None,
            harness_id: Some(preserved_harness.id),
            agent_id: Some(agent.id),
            agent_revision: None,
            virtual_user_id: None,
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
    let message_service_db = db.clone();
    let message_service = MessageService::new(db, runner.clone(), EventDelivery::in_memory());
    dispatch_trigger_message(
        &message_service,
        DEFAULT_ORG_ID,
        &agent,
        TriggerId::new(),
        session.id,
        preserved_harness.id,
        owner.id,
        "scheduled message".to_string(),
        None,
        None,
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

    // A trigger that targets a saved script marks its message as a script
    // run, which only the platform can do.
    let run = everruns_contracts::runtime::saved_scripts::ScriptRun {
        script: "triage".to_string(),
        input: Some(serde_json::json!({"repo": "x"})),
        wake_agent_on_failure: true,
    };
    dispatch_trigger_message(
        &message_service,
        DEFAULT_ORG_ID,
        &agent,
        TriggerId::new(),
        session.id,
        preserved_harness.id,
        owner.id,
        "run triage".to_string(),
        Some(&run),
        None,
    )
    .await
    .unwrap();
    let events = message_service_db
        .list_message_events(session.id)
        .await
        .unwrap();
    let marked: Vec<_> = events
        .iter()
        .filter_map(|event| {
            event
                .data
                .pointer("/message/metadata")
                .and_then(|metadata| metadata.get("everruns_script_run"))
        })
        .collect();
    assert_eq!(marked, vec![&serde_json::to_value(&run).unwrap()]);
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

    let db = Arc::new(StorageBackend::test_database());
    let store = workflow_store(&db);
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
    let db = Arc::new(StorageBackend::test_database());
    let store = workflow_store(&db);
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
    let db = Arc::new(StorageBackend::test_database());
    let store = workflow_store(&db);
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
    let db = Arc::new(StorageBackend::test_database());
    let store = workflow_store(&db);
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
    let db = Arc::new(StorageBackend::test_database());
    let store = workflow_store(&db);
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
    let db = Arc::new(StorageBackend::test_database());
    let store = workflow_store(&db);
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
    let db = Arc::new(StorageBackend::test_database());
    let store = workflow_store(&db);
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
async fn seed_agent_row(db: &Arc<StorageBackend>) -> crate::storage::AgentRow {
    let (public_id, _) = seed_agent(db).await;
    db.get_agent_by_public_id(DEFAULT_ORG_ID, &public_id)
        .await
        .unwrap()
        .expect("seeded agent row")
}

#[tokio::test]
async fn ensure_identity_for_agent_creates_and_links_when_none() {
    let db = Arc::new(StorageBackend::test_database());
    let agent = seed_agent_row(&db).await;
    assert!(agent.virtual_user_id.is_none());

    let (identity_id, owner) = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent)
        .await
        .expect("lazily create identity");

    // The trigger session owner is the agent's own identity principal.
    assert_eq!(owner.kind, "virtual_user");
    // The agent row is now linked to the freshly-created identity.
    let linked = db
        .get_agent(DEFAULT_ORG_ID, agent.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(linked.virtual_user_id, Some(identity_id));
}

#[tokio::test]
async fn ensure_identity_for_agent_is_idempotent_across_fires() {
    let db = Arc::new(StorageBackend::test_database());
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
    use crate::storage::CreateVirtualUserRow;
    use everruns_contracts::typed_id::VirtualUserId;

    let db = Arc::new(StorageBackend::test_database());
    let mut agent = seed_agent_row(&db).await;

    let explicit = VirtualUserId::new();
    db.create_virtual_user(CreateVirtualUserRow {
        usage: "service".to_string(),
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
        db.set_virtual_user_id(DEFAULT_ORG_ID, agent.id, explicit)
            .await
            .unwrap()
    );
    agent.virtual_user_id = Some(explicit);

    let (identity_id, owner) = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent)
        .await
        .expect("resolve explicit identity");

    assert_eq!(identity_id, explicit, "explicit identity is returned as-is");
    assert_eq!(owner.kind, "virtual_user");
    let linked = db
        .get_agent(DEFAULT_ORG_ID, agent.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        linked.virtual_user_id,
        Some(explicit),
        "explicit identity is never overridden"
    );

    // A guarded set on an already-linked agent is a no-op.
    assert!(
        !db.set_virtual_user_id(DEFAULT_ORG_ID, agent.id, VirtualUserId::new())
            .await
            .unwrap(),
        "set_virtual_user_id refuses to override an existing link"
    );
}

#[tokio::test]
async fn ensure_identity_for_agent_rejects_archived_linked_identity() {
    use crate::storage::CreateVirtualUserRow;
    use everruns_contracts::typed_id::VirtualUserId;

    let db = Arc::new(StorageBackend::test_database());
    let mut agent = seed_agent_row(&db).await;

    let identity_id = VirtualUserId::new();
    db.create_virtual_user(CreateVirtualUserRow {
        usage: "service".to_string(),
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
    db.set_virtual_user_id(DEFAULT_ORG_ID, agent.id, identity_id)
        .await
        .unwrap();
    db.delete_virtual_user(DEFAULT_ORG_ID, identity_id)
        .await
        .unwrap();
    agent.virtual_user_id = Some(identity_id);

    let err = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent)
        .await
        .expect_err("archived identity must not own new trigger sessions");
    assert!(
        err.to_string().contains("is not active"),
        "unexpected error: {err:#}"
    );
}

/// EVE-1005: one `SessionBinding` now spans messaging and invocations, so the
/// constraint that used to be carried by the type system — triggers simply had
/// no `per_thread` to express — has to be enforced at write time instead.
#[tokio::test]
async fn create_trigger_rejects_message_keyed_bindings() {
    let db = Arc::new(StorageBackend::test_database());
    let store = workflow_store(&db);
    let ctx = test_ctx(db.clone(), store);
    let (agent_id, _) = seed_agent(&db).await;

    for binding in SessionBinding::MESSAGE_KEYED {
        let mut req = create_req("0 9 * * *", "nope", true);
        req.session_mode = binding;
        let err = CreateAgentTrigger {
            agent_id: agent_id.clone(),
            req,
        }
        .execute(&ctx)
        .await
        .expect_err("a trigger has no thread, conversation or requester to key on");
        assert!(
            err.message().contains("not valid for an agent trigger"),
            "{binding:?} got: {err}"
        );
    }

    // The invocation-keyed bindings remain accepted.
    for binding in SessionBinding::INVOCATION_KEYED {
        let mut req = create_req("0 9 * * *", "fine", false);
        req.session_mode = binding;
        CreateAgentTrigger {
            agent_id: agent_id.clone(),
            req,
        }
        .execute(&ctx)
        .await
        .unwrap_or_else(|e| panic!("{binding:?} must be accepted, got: {e}"));
    }
}

/// The same guard on the update path, which merges onto a stored config.
#[tokio::test]
async fn update_trigger_rejects_message_keyed_bindings() {
    let db = Arc::new(StorageBackend::test_database());
    let store = workflow_store(&db);
    let ctx = test_ctx(db.clone(), store);
    let (agent_id, _) = seed_agent(&db).await;

    let created = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: create_req("0 9 * * *", "hello", false),
    }
    .execute(&ctx)
    .await
    .expect("create");

    let err = UpdateAgentTriggerCmd {
        agent_id: agent_id.clone(),
        trigger_id: created.id.to_string(),
        req: UpdateAgentTriggerRequest {
            session_mode: Some(SessionBinding::Thread),
            ..Default::default()
        },
    }
    .execute(&ctx)
    .await
    .expect_err("update must refuse a message-keyed binding too");
    assert!(
        err.message().contains("not valid for an agent trigger"),
        "got: {err}"
    );
}

// ---- event pipeline -------------------------------------------------------

fn pr_event(
    delivery: &str,
    repo: &str,
    number: u64,
    action: &str,
) -> WebhookTriggerInvocationRequest {
    let payload = serde_json::json!({
        "action": action,
        "number": number,
        "repository": {"full_name": repo},
    });
    WebhookTriggerInvocationRequest {
        ingress_id: String::new(),
        body: payload.to_string(),
        json_payload: Some(payload),
        headers: [("x-github-delivery".to_string(), delivery.to_string())]
            .into_iter()
            .collect(),
    }
}

#[tokio::test]
async fn webhook_events_are_filtered_deduplicated_and_routed_per_subject() {
    let db = Arc::new(StorageBackend::test_database());
    let (agent_id, _) = seed_agent(&db).await;
    let ctx = role_ctx(db.clone(), OrgRole::Owner);
    let trigger = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: CreateAgentTriggerRequest {
            session_mode: SessionBinding::Thread,
            message: "PR {{payload.number}} {{payload.action}}".to_string(),
            event_id_template: Some("{{webhook.headers.x-github-delivery}}".to_string()),
            subject_template: Some(
                "{{payload.repository.full_name}}#{{payload.number}}".to_string(),
            ),
            filter: Some(crate::domains::agent_triggers::record::TriggerEventFilter {
                conditions: vec![
                    crate::domains::agent_triggers::record::TriggerFilterCondition {
                        path: "payload.action".to_string(),
                        any_of: vec![
                            serde_json::json!("opened"),
                            serde_json::json!("synchronize"),
                        ],
                    },
                ],
            }),
            ..webhook_req(true)
        },
    }
    .run(&ctx)
    .await
    .expect("create webhook trigger");
    let ingress_id = trigger.ingress_id.expect("webhook ingress").to_string();

    let runner = Arc::new(RecordingRunner::default());
    let session_service = SessionService::new(db.clone());
    let message_service =
        MessageService::new(db.clone(), runner.clone(), EventDelivery::in_memory());
    let encryption = ctx.encryption.clone();
    let fire = async |event: WebhookTriggerInvocationRequest| {
        invoke_webhook_agent_trigger(
            &db,
            encryption.as_ref(),
            &session_service,
            &message_service,
            WebhookTriggerInvocationRequest {
                ingress_id: ingress_id.clone(),
                ..event
            },
            None,
        )
        .await
        .expect("event handled")
    };

    let events::TriggerEventOutcome::Dispatched(first) =
        fire(pr_event("d1", "acme/api", 7, "opened")).await
    else {
        panic!("opened PR must dispatch");
    };
    assert!(first.created_session);

    let events::TriggerEventOutcome::Dispatched(push) =
        fire(pr_event("d2", "acme/api", 7, "synchronize")).await
    else {
        panic!("push to the same PR must dispatch");
    };
    assert_eq!(
        push.session_id, first.session_id,
        "same PR continues one session"
    );
    assert!(!push.created_session);

    assert!(matches!(
        fire(pr_event("d2", "acme/api", 7, "synchronize")).await,
        events::TriggerEventOutcome::Duplicate
    ));
    assert!(matches!(
        fire(pr_event("d3", "acme/api", 7, "labeled")).await,
        events::TriggerEventOutcome::Filtered
    ));

    let events::TriggerEventOutcome::Dispatched(other) =
        fire(pr_event("d4", "acme/api", 8, "opened")).await
    else {
        panic!("another PR must dispatch");
    };
    assert_ne!(
        other.session_id, first.session_id,
        "another PR gets its own session"
    );

    let deliveries = ListAgentTriggerDeliveries {
        agent_id,
        trigger_id: trigger.id.to_string(),
        limit: None,
    }
    .run(&ctx)
    .await
    .expect("list deliveries");
    let summary: Vec<_> = deliveries
        .iter()
        .rev()
        .map(|delivery| {
            (
                delivery.event_id.clone().unwrap_or_default(),
                delivery.status,
                delivery.subject.clone().unwrap_or_default(),
            )
        })
        .collect();
    use crate::domains::agent_triggers::record::TriggerDeliveryStatus::*;
    assert_eq!(
        summary,
        vec![
            ("d1".to_string(), Dispatched, "acme/api#7".to_string()),
            ("d2".to_string(), Dispatched, "acme/api#7".to_string()),
            ("d2".to_string(), Duplicate, "acme/api#7".to_string()),
            ("d3".to_string(), Filtered, "acme/api#7".to_string()),
            ("d4".to_string(), Dispatched, "acme/api#8".to_string()),
        ]
    );
    assert_eq!(
        deliveries
            .iter()
            .find(|d| d.status == Filtered)
            .unwrap()
            .reason
            .as_deref(),
        Some("payload.action is labeled")
    );
    assert_eq!(deliveries[0].session_id, Some(other.session_id));
}

#[tokio::test]
async fn per_thread_requires_a_subject_template() {
    let db = Arc::new(StorageBackend::test_database());
    let (agent_id, _) = seed_agent(&db).await;
    let ctx = role_ctx(db, OrgRole::Owner);
    let error = CreateAgentTrigger {
        agent_id,
        req: CreateAgentTriggerRequest {
            session_mode: SessionBinding::Thread,
            ..webhook_req(true)
        },
    }
    .run(&ctx)
    .await
    .expect_err("per_thread without a subject");
    assert!(error.message().contains("subject_template"), "got: {error}");
}

#[tokio::test]
async fn filter_conditions_need_a_path_and_values() {
    let db = Arc::new(StorageBackend::test_database());
    let (agent_id, _) = seed_agent(&db).await;
    let ctx = role_ctx(db, OrgRole::Owner);
    let error = CreateAgentTrigger {
        agent_id,
        req: CreateAgentTriggerRequest {
            filter: Some(crate::domains::agent_triggers::record::TriggerEventFilter {
                conditions: vec![
                    crate::domains::agent_triggers::record::TriggerFilterCondition {
                        path: "payload.action".to_string(),
                        any_of: vec![],
                    },
                ],
            }),
            ..webhook_req(true)
        },
    }
    .run(&ctx)
    .await
    .expect_err("empty any_of");
    assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn delivery_history_is_bounded_per_trigger() {
    let db = Arc::new(StorageBackend::test_database());
    let (agent_id, _) = seed_agent(&db).await;
    let trigger_id = CreateAgentTrigger {
        agent_id,
        req: webhook_req(false),
    }
    .run(&role_ctx(db.clone(), OrgRole::Owner))
    .await
    .expect("create trigger")
    .id;
    for index in 0..5 {
        db.record_agent_trigger_delivery(
            crate::storage::agent_trigger_deliveries::CreateAgentTriggerDeliveryRow {
                org_id: DEFAULT_ORG_ID,
                trigger_id,
                source: "webhook".to_string(),
                event_id: Some(format!("e{index}")),
                event_type: None,
                subject: None,
                status: "dispatched".to_string(),
                reason: None,
            },
        )
        .await
        .unwrap();
    }
    assert_eq!(
        db.prune_agent_trigger_deliveries(trigger_id, 3)
            .await
            .unwrap(),
        2
    );
    let kept = db
        .list_agent_trigger_deliveries(DEFAULT_ORG_ID, trigger_id, 10)
        .await
        .unwrap();
    let ids: Vec<_> = kept.iter().filter_map(|row| row.event_id.clone()).collect();
    assert_eq!(ids, vec!["e4", "e3", "e2"]);
}

// ---- GitHub triggers ------------------------------------------------------

fn github_req(repositories: Option<Vec<String>>) -> CreateAgentTriggerRequest {
    CreateAgentTriggerRequest {
        trigger_type: AgentTriggerType::GitHub,
        session_mode: SessionBinding::Thread,
        message: "Summarize {{github.repository}}#{{github.number}}".to_string(),
        token: None,
        repositories,
        ..webhook_req(true)
    }
}

fn github_delivery(
    delivery_id: &str,
    event: &str,
    action: &str,
    repo: &str,
    sender: &str,
) -> crate::domains::agent_triggers::github::GitHubDelivery {
    crate::domains::agent_triggers::github::GitHubDelivery {
        event: event.to_string(),
        delivery_id: delivery_id.to_string(),
        payload: serde_json::json!({
            "action": action,
            "number": 7,
            "pull_request": {"number": 7, "title": "Fix", "html_url": "https://x/pull/7"},
            "repository": {"full_name": repo},
            "sender": {"login": sender},
        }),
    }
}

#[tokio::test]
async fn github_trigger_needs_the_identitys_app_and_routes_its_deliveries() {
    use crate::domains::agent_triggers::github::dispatch_github_delivery;
    use crate::storage::CreateGitHubAppRow;

    let db = Arc::new(StorageBackend::test_database());
    let (agent_id, _) = seed_agent(&db).await;
    let ctx = role_ctx(db.clone(), OrgRole::Owner);

    // No GitHub connection yet: creating the trigger is refused.
    let err = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: github_req(None),
    }
    .run(&ctx)
    .await
    .expect_err("GitHub trigger needs GitHub connected");
    assert!(err.message().contains("GitHub"), "got: {err}");

    let agent = db
        .get_agent_by_public_id(DEFAULT_ORG_ID, &agent_id)
        .await
        .unwrap()
        .unwrap();
    let (identity_id, _) = ensure_identity_for_agent(&db, DEFAULT_ORG_ID, &agent)
        .await
        .unwrap();
    let app = db
        .create_github_app(CreateGitHubAppRow {
            id: uuid::Uuid::new_v4(),
            org_id: DEFAULT_ORG_ID,
            virtual_user_id: identity_id,
            app_id: 42,
            slug: "pr-summarizer".to_string(),
            name: "pr-summarizer".to_string(),
            html_url: "https://github.com/apps/pr-summarizer".to_string(),
            owner_login: Some("acme".to_string()),
            client_id: None,
            client_secret_encrypted: None,
            private_key_encrypted: vec![1],
            webhook_secret_encrypted: Some(vec![1]),
            created_by_user_id: None,
        })
        .await
        .unwrap();

    let trigger = CreateAgentTrigger {
        agent_id: agent_id.clone(),
        req: github_req(Some(vec!["Acme/API".to_string()])),
    }
    .run(&ctx)
    .await
    .expect("create GitHub trigger");
    assert_eq!(trigger.trigger_type, AgentTriggerType::GitHub);
    assert!(
        trigger.ingress_id.is_none(),
        "GitHub triggers share the App's webhook"
    );

    let runner = Arc::new(RecordingRunner::default());
    let session_service = SessionService::new(db.clone());
    let message_service =
        MessageService::new(db.clone(), runner.clone(), EventDelivery::in_memory());
    let fire = async |delivery| {
        dispatch_github_delivery(
            &db,
            &session_service,
            &message_service,
            &app,
            &delivery,
            None,
        )
        .await
        .expect("delivery routed")
        .into_iter()
        .map(|dispatch| dispatch.outcome.expect("dispatch ok"))
        .collect::<Vec<_>>()
    };

    let opened = fire(github_delivery(
        "d1",
        "pull_request",
        "opened",
        "acme/api",
        "octo",
    ))
    .await;
    let [events::TriggerEventOutcome::Dispatched(first)] = opened.as_slice() else {
        panic!("opened PR must dispatch, got {opened:?}");
    };
    let redelivered = fire(github_delivery(
        "d1",
        "pull_request",
        "opened",
        "acme/api",
        "octo",
    ))
    .await;
    assert!(matches!(
        redelivered.as_slice(),
        [events::TriggerEventOutcome::Duplicate]
    ));
    let pushed = fire(github_delivery(
        "d2",
        "pull_request",
        "synchronize",
        "acme/api",
        "octo",
    ))
    .await;
    let [events::TriggerEventOutcome::Dispatched(second)] = pushed.as_slice() else {
        panic!("push to the same PR must dispatch, got {pushed:?}");
    };
    assert_eq!(first.session_id, second.session_id, "one session per PR");

    let other_repo = fire(github_delivery(
        "d3",
        "pull_request",
        "opened",
        "acme/web",
        "octo",
    ))
    .await;
    assert!(matches!(
        other_repo.as_slice(),
        [events::TriggerEventOutcome::Filtered]
    ));
    // Unsubscribed actions and the App's own activity are not routed at all.
    assert!(
        fire(github_delivery(
            "d4",
            "pull_request",
            "closed",
            "acme/api",
            "octo"
        ))
        .await
        .is_empty()
    );
    assert!(
        fire(github_delivery(
            "d5",
            "pull_request",
            "opened",
            "acme/api",
            "pr-summarizer[bot]"
        ))
        .await
        .is_empty()
    );
}

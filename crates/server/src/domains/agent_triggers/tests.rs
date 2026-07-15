// Unit tests for agent-trigger commands: cron validation, per-org cap, and
// durable-binding create/teardown on enable/disable/delete. Mirrors the
// app schedule-channel tests but with no DB dependency (in-memory storage +
// in-memory workflow store).

use super::*;
use crate::domains::agent_triggers::types::{CreateAgentTriggerRequest, UpdateAgentTriggerRequest};
use crate::domains::common::Ctx;
use crate::storage::StorageBackend;
use crate::storage::models::{CreateAgentRow, CreateHarnessRow};
use everruns_core::typed_id::AgentId;
use everruns_core::{Caller, DEFAULT_ORG_ID, InvocationSessionMode};
use everruns_durable::InMemoryWorkflowEventStore;
use std::sync::{Arc, Mutex};

// Serializes the env-mutating cap tests.
static ENV_LOCK: Mutex<()> = Mutex::new(());

async fn seed_agent(db: &Arc<StorageBackend>) -> (String, everruns_core::typed_id::HarnessId) {
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

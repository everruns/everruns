//! Dual-backend conformance tests for storage behavior that has drifted before.
//!
//! Run with: cargo test -p everruns-server --test repository_conformance_test -- --test-threads=1

mod test_harness;

use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use everruns_core::DEFAULT_ORG_ID;
use everruns_core::message_filter::MessageQuery;
use everruns_provider::typed_id::TriggerId;
use everruns_provider::typed_id::{AgentId, HarnessId, PrincipalId};
use everruns_server::org_init;
use everruns_server::storage::{
    CreateAgentRow, CreateAgentTriggerRow, CreateBudgetRow, CreateEventRow, CreatePrincipalRow,
    CreateProviderRow, CreateSessionRow, CreateUsageJournalRow, CreateUsageLedgerRow, Database,
    MESSAGE_SAFETY_LIMIT, Repository, StorageBackend, UpdateAgentTrigger,
};
use test_harness::get_database_url;

async fn create_postgres_backend() -> StorageBackend {
    let pool = PgPool::connect(&get_database_url())
        .await
        .expect("Failed to connect to PostgreSQL");
    StorageBackend::Postgres(Database::new(pool))
}

async fn create_test_principal(repo: &dyn Repository, label: &str) -> PrincipalId {
    repo.create_principal(CreatePrincipalRow {
        id: PrincipalId::new(),
        org_id: DEFAULT_ORG_ID,
        kind: "system".to_string(),
        subject_id: Some(Uuid::now_v7()),
        parent_principal_id: None,
        resolved_user_id: None,
        metadata: json!({ "source": "repository_conformance_test", "label": label }),
    })
    .await
    .expect("create principal")
    .id
}

fn session_input(owner_principal_id: PrincipalId, label: &str) -> CreateSessionRow {
    CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        org_id: DEFAULT_ORG_ID,
        app_id: None,
        harness_id: None,
        agent_id: None,
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: None,
        owner_principal_id,
        resolved_owner_user_id: None,
        title: Some(format!("conformance-{label}")),
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: json!([]),
        tools: json!([]),
        mcp_servers: json!({}),
        system_prompt: None,
        initial_files: json!([]),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: None,
        budget_root_session_id: None,
        workspace_id: None,
    }
}

fn agent_input(name: String, harness_id: HarnessId) -> CreateAgentRow {
    CreateAgentRow {
        public_id: AgentId::new().to_string(),
        name,
        display_name: Some("Conformance Agent".to_string()),
        description: None,
        system_prompt: "You are a conformance test agent.".to_string(),
        default_model_id: None,
        harness_id,
        tags: vec![],
        initial_files: json!([]),
        tools: json!([]),
        mcp_servers: json!({}),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        is_built_in: false,
    }
}

async fn assert_one_outbox(
    repo: &dyn Repository,
    source_type: &str,
    source_id: &str,
    reason: &str,
) {
    let rows = repo
        .list_reporting_outbox(DEFAULT_ORG_ID, source_type, source_id, reason)
        .await
        .expect("list reporting outbox");
    assert_eq!(
        rows.len(),
        1,
        "expected one reporting outbox row for {source_type}/{source_id}/{reason}"
    );
    assert_eq!(rows[0].status, "pending");
}

async fn run_repository_conformance(repo: &dyn Repository, label: &str, harness_id: HarnessId) {
    let principal_id = create_test_principal(repo, label).await;

    let session = repo
        .create_session(session_input(principal_id, label))
        .await
        .expect("create session");
    assert_one_outbox(
        repo,
        "session",
        &session.id.uuid().to_string(),
        "session_snapshot",
    )
    .await;

    let event = repo
        .create_event(CreateEventRow {
            session_id: session.id,
            event_type: "input.message".to_string(),
            ts: Utc::now(),
            context: json!({}),
            data: json!({
                "message": {
                    "content": [{ "type": "text", "text": "hello" }]
                }
            }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("create event");
    assert_one_outbox(
        repo,
        "event",
        &event.id.uuid().to_string(),
        "event_projection",
    )
    .await;

    let provider_prefix = format!("conf-{label}-{}", Uuid::now_v7());
    let first = repo
        .create_provider(
            DEFAULT_ORG_ID,
            CreateProviderRow {
                name: format!("{provider_prefix}-first"),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .expect("create first provider");
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let second = repo
        .create_provider(
            DEFAULT_ORG_ID,
            CreateProviderRow {
                name: format!("{provider_prefix}-second"),
                provider_type: "anthropic".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .expect("create second provider");
    let providers = repo
        .list_providers(DEFAULT_ORG_ID)
        .await
        .expect("list providers");
    let first_pos = providers
        .iter()
        .position(|provider| provider.id == first.id)
        .expect("first provider listed");
    let second_pos = providers
        .iter()
        .position(|provider| provider.id == second.id)
        .expect("second provider listed");
    assert!(
        second_pos < first_pos,
        "providers must be newest-first by created_at"
    );

    let agent_name = format!("conf-agent-{label}-{}", Uuid::now_v7());
    let (created_agent, was_created) = repo
        .upsert_agent_by_name(DEFAULT_ORG_ID, agent_input(agent_name.clone(), harness_id))
        .await
        .expect("create agent by name");
    assert!(was_created);
    let (updated_agent, was_created) = repo
        .upsert_agent_by_name(DEFAULT_ORG_ID, agent_input(agent_name, harness_id))
        .await
        .expect("update agent by name");
    assert!(!was_created);
    assert_eq!(
        updated_agent.id, created_agent.id,
        "same non-deleted org/name must update in place"
    );

    let budget = repo
        .create_budget(CreateBudgetRow {
            org_id: DEFAULT_ORG_ID,
            subject_type: "session".to_string(),
            subject_id: session.id.to_string(),
            currency: "usd".to_string(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .expect("create budget");
    let journal = repo
        .create_usage_journal_entry(CreateUsageJournalRow {
            org_id: DEFAULT_ORG_ID,
            kind: "llm_usage".to_string(),
            source_type: Some("repository_conformance_test".to_string()),
            source_id: Some(Uuid::now_v7().to_string()),
            event_id: Some(event.id.uuid()),
            session_id: Some(session.id.uuid()),
            turn_id: None,
            user_id: None,
            principal_id: Some(principal_id.uuid()),
            agent_id: None,
            harness_id: None,
            measures: json!({ "tokens": 1 }),
            metadata: json!({}),
        })
        .await
        .expect("create usage journal");
    let (ledger, updated_budget) = repo
        .create_usage_ledger_entry(CreateUsageLedgerRow {
            journal_id: journal.id,
            budget_id: Some(budget.id),
            org_id: DEFAULT_ORG_ID,
            session_id: Some(session.id.uuid()),
            user_id: None,
            principal_id: Some(principal_id.uuid()),
            agent_id: None,
            harness_id: None,
            currency: "usd".to_string(),
            amount: 2.0,
            meter_source: "conformance".to_string(),
            ref_type: Some("event".to_string()),
            ref_id: Some(event.id.uuid()),
            description: Some("repository conformance usage".to_string()),
            rating_metadata: Some(json!({ "source": "repository_conformance_test" })),
        })
        .await
        .expect("create usage ledger");
    assert_eq!(
        updated_budget.expect("updated budget").balance,
        budget.balance - 2.0
    );
    assert_one_outbox(
        repo,
        "usage_ledger",
        &ledger.id.to_string(),
        "budget_posting_projection",
    )
    .await;

    for index in 0..=MESSAGE_SAFETY_LIMIT {
        repo.create_event(CreateEventRow {
            session_id: session.id,
            event_type: "input.message".to_string(),
            ts: Utc::now(),
            context: json!({}),
            data: json!({
                "message": {
                    "content": [{ "type": "text", "text": format!("message {index}") }]
                }
            }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("create capped message event");
    }

    let uncapped = repo
        .list_message_events_limited(session.id, None)
        .await
        .expect("list message events without explicit limit");
    assert_eq!(uncapped.len(), MESSAGE_SAFETY_LIMIT);
    assert!(
        uncapped.first().expect("first capped event").sequence > 1,
        "unbounded list_message_events_limited keeps the latest capped window"
    );

    let filtered = repo
        .list_message_events_filtered(&MessageQuery::new(session.id))
        .await
        .expect("list filtered message events without explicit limit");
    assert_eq!(filtered.len(), MESSAGE_SAFETY_LIMIT);
    assert!(
        filtered.first().expect("first filtered event").sequence > 1,
        "unbounded filtered message listing keeps the latest capped window"
    );
}

/// Dual-backend round-trip for agent triggers (EVE-757): create -> get -> list
/// (with agent filter) -> update -> bind durable schedule -> soft delete. Runs
/// against both the in-memory and PostgreSQL backends via `StorageBackend`.
async fn run_agent_trigger_conformance(
    backend: &StorageBackend,
    label: &str,
    harness_id: HarnessId,
) {
    // A real agent is required so the PostgreSQL FK is satisfied.
    let (agent, _) = backend
        .upsert_agent_by_name(
            DEFAULT_ORG_ID,
            agent_input(format!("trigger-owner-{label}"), harness_id),
        )
        .await
        .expect("create agent");

    let created = backend
        .create_agent_trigger(CreateAgentTriggerRow {
            org_id: DEFAULT_ORG_ID,
            id: TriggerId::new(),
            agent_id: agent.id,
            trigger_type: "schedule".to_string(),
            config: json!({
                "cron_expression": "0 0 * * * *",
                "timezone": "UTC",
                "session_mode": "shared_session",
                "message": "hello",
            }),
            enabled: true,
            durable_schedule_id: None,
            execution_harness_id: None,
            execution_owner_principal_id: None,
            execution_resolved_owner_user_id: None,
            execution_agent_identity_id: None,
            execution_app_id: None,
        })
        .await
        .expect("create agent trigger");
    assert_eq!(created.status, "active");
    assert_eq!(created.agent_id, agent.id);

    let fetched = backend
        .get_agent_trigger(DEFAULT_ORG_ID, created.id)
        .await
        .expect("get agent trigger")
        .expect("trigger exists");
    assert_eq!(fetched.id, created.id);

    let by_agent = backend
        .list_agent_triggers(DEFAULT_ORG_ID, Some(agent.id), false)
        .await
        .expect("list by agent");
    assert!(by_agent.iter().any(|t| t.id == created.id));
    assert!(by_agent.iter().all(|t| t.agent_id == agent.id));

    let updated = backend
        .update_agent_trigger(
            DEFAULT_ORG_ID,
            created.id,
            UpdateAgentTrigger {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("update agent trigger")
        .expect("update returns row");
    assert!(!updated.enabled);

    // Clearing the durable schedule binding round-trips (a NULL binding avoids
    // needing a real durable_schedules row for the FK).
    let cleared = backend
        .set_agent_trigger_durable_schedule_id(DEFAULT_ORG_ID, created.id, None)
        .await
        .expect("clear durable schedule")
        .expect("returns row");
    assert!(cleared.durable_schedule_id.is_none());

    assert!(
        backend
            .delete_agent_trigger(DEFAULT_ORG_ID, created.id)
            .await
            .expect("soft delete")
    );
    let archived = backend
        .get_agent_trigger(DEFAULT_ORG_ID, created.id)
        .await
        .expect("get after delete")
        .expect("row present after soft delete");
    assert_eq!(archived.status, "archived");
    assert!(
        !backend
            .list_agent_triggers(DEFAULT_ORG_ID, None, false)
            .await
            .expect("list active")
            .iter()
            .any(|t| t.id == created.id)
    );
}

#[tokio::test]
async fn in_memory_repository_conformance() {
    let backend = StorageBackend::in_memory();
    let harness_id = HarnessId::from_uuid(Uuid::nil());
    run_repository_conformance(&backend, "memory", harness_id).await;
    run_agent_trigger_conformance(&backend, "memory", harness_id).await;
}

#[tokio::test]
async fn postgres_repository_conformance() {
    let backend = create_postgres_backend().await;
    org_init::initialize_org_harnesses(&backend, DEFAULT_ORG_ID)
        .await
        .expect("initialize built-in harnesses");
    let harness_id = org_init::generic_harness_id(&backend, DEFAULT_ORG_ID)
        .await
        .expect("generic harness id");
    run_repository_conformance(&backend, "postgres", harness_id).await;
    run_agent_trigger_conformance(&backend, "postgres", harness_id).await;
}

/// EVE-870: the reconciliation half of the checkpoint crash window.
///
/// `attach_checkpoint` commits a revision as authoritative before the tool
/// result it belongs to is settled, so a crash in between leaves the workspace
/// ahead of the conversation. `rollback_current_checkpoint` is what puts them
/// back in step, and every property that makes it safe lives in its SQL rather
/// than in the caller — hence a Postgres test rather than a unit test.
#[tokio::test]
async fn postgres_sandbox_checkpoint_rollback() {
    use everruns_platform::sandbox_checkpoint::{
        NewSandboxCheckpoint, SandboxCheckpointError, SandboxCheckpointKind, SandboxCheckpointStore,
    };
    use everruns_server::storage::PgSandboxCheckpointStore;

    let pool = PgPool::connect(&get_database_url())
        .await
        .expect("connect to PostgreSQL");
    let backend = StorageBackend::Postgres(Database::new(pool.clone()));
    let owner = create_test_principal(&backend, "sandbox-rollback").await;
    let session = backend
        .create_session(session_input(owner, "sandbox-rollback"))
        .await
        .expect("create session");

    let store = PgSandboxCheckpointStore::new(pool);
    let sandbox = store
        .ensure_sandbox(session.id, "daytona")
        .await
        .expect("ensure sandbox");

    let new_checkpoint = |revision: &str, call: &str| NewSandboxCheckpoint {
        sandbox_id: sandbox.id,
        generation: sandbox.generation,
        source_turn_id: Some("turn-1".to_string()),
        source_tool_call_id: Some(call.to_string()),
        kind: SandboxCheckpointKind::PortableWorkspace,
        provider_ref: None,
        workspace_revision: revision.to_string(),
    };

    let first = store
        .record_checkpoint(new_checkpoint("rev-1", "call-1"))
        .await
        .expect("record first");
    store
        .attach_checkpoint(sandbox.id, first.id, sandbox.generation)
        .await
        .expect("attach first");
    let second = store
        .record_checkpoint(new_checkpoint("rev-2", "call-2"))
        .await
        .expect("record second");
    store
        .attach_checkpoint(sandbox.id, second.id, sandbox.generation)
        .await
        .expect("attach second");

    // Rolling back the head falls to the previous *attached* revision.
    let restored = store
        .rollback_current_checkpoint(sandbox.id, second.id, sandbox.generation)
        .await
        .expect("rollback")
        .expect("an earlier committed checkpoint exists");
    assert_eq!(restored.id, first.id);
    assert_eq!(restored.workspace_revision, "rev-1");
    assert_eq!(
        store
            .current_checkpoint(sandbox.id)
            .await
            .expect("read current")
            .map(|c| c.id),
        Some(first.id)
    );

    // The rejected revision is detached, which is what returns it to the
    // collectable pool instead of stranding it forever.
    let collected = store
        .collect_unattached_checkpoints(sandbox.id, Utc::now(), 10)
        .await
        .expect("collect");
    assert_eq!(collected, vec!["rev-2".to_string()]);
    // ...and collection still refuses to touch the authoritative one.
    assert_eq!(
        store
            .current_checkpoint(sandbox.id)
            .await
            .expect("read current")
            .map(|c| c.id),
        Some(first.id)
    );

    // Rolling back with nothing earlier committed clears the pointer rather
    // than leaving a revision no committed turn produced.
    assert!(
        store
            .rollback_current_checkpoint(sandbox.id, first.id, sandbox.generation)
            .await
            .expect("rollback to empty")
            .is_none()
    );
    assert!(
        store
            .current_checkpoint(sandbox.id)
            .await
            .expect("read current")
            .is_none()
    );

    // Fenced: a rollback carrying a superseded generation is refused outright,
    // the same guard `attach_checkpoint` applies.
    let stale = store
        .rollback_current_checkpoint(sandbox.id, first.id, sandbox.generation + 1)
        .await;
    assert!(matches!(
        stale,
        Err(SandboxCheckpointError::StaleGeneration { .. })
    ));

    // A rollback of a checkpoint the sandbox no longer points at is a no-op:
    // something newer already decided, and this must not undo it.
    let third = store
        .record_checkpoint(new_checkpoint("rev-3", "call-3"))
        .await
        .expect("record third");
    store
        .attach_checkpoint(sandbox.id, third.id, sandbox.generation)
        .await
        .expect("attach third");
    assert_eq!(
        store
            .rollback_current_checkpoint(sandbox.id, first.id, sandbox.generation)
            .await
            .expect("stale-pointer rollback")
            .map(|c| c.id),
        Some(third.id)
    );
}

/// EVE-867: the run-summary write is fenced on the terminal turn it describes.
///
/// Summarisation runs out of band, so a slow call for turn N can land after
/// turn N+1 has already been summarised. The guard lives in the `WHERE` clause
/// rather than the caller, so it belongs in a backend test.
async fn run_run_summary_fence_conformance(backend: &StorageBackend, label: &str) {
    let owner = create_test_principal(backend, &format!("run-summary-{label}")).await;
    let session = backend
        .create_session(session_input(owner, &format!("run-summary-{label}")))
        .await
        .expect("create session");

    assert!(
        backend
            .set_session_run_summary(DEFAULT_ORG_ID, session.id, "Ran the report.", 10)
            .await
            .expect("first summary"),
        "{label}: the first summary is written"
    );

    // A late write for an older turn loses.
    assert!(
        !backend
            .set_session_run_summary(DEFAULT_ORG_ID, session.id, "Stale summary.", 5)
            .await
            .expect("stale summary"),
        "{label}: an older turn must not overwrite a newer summary"
    );
    // Re-summarising the same turn is not progress either, so it also loses.
    assert!(
        !backend
            .set_session_run_summary(DEFAULT_ORG_ID, session.id, "Same turn.", 10)
            .await
            .expect("same-turn summary"),
        "{label}: the same turn must not rewrite its summary"
    );

    let stored = backend
        .get_session(DEFAULT_ORG_ID, session.id)
        .await
        .expect("read session")
        .expect("session exists");
    assert_eq!(stored.run_summary.as_deref(), Some("Ran the report."));
    assert_eq!(stored.run_summary_turn_sequence, Some(10));

    // A newer turn wins.
    assert!(
        backend
            .set_session_run_summary(DEFAULT_ORG_ID, session.id, "Ran again, failed.", 11)
            .await
            .expect("newer summary"),
        "{label}: a newer turn replaces the summary"
    );
    let stored = backend
        .get_session(DEFAULT_ORG_ID, session.id)
        .await
        .expect("read session")
        .expect("session exists");
    assert_eq!(stored.run_summary.as_deref(), Some("Ran again, failed."));
    assert_eq!(stored.run_summary_turn_sequence, Some(11));

    // Another org cannot write into this session's summary.
    assert!(
        !backend
            .set_session_run_summary(DEFAULT_ORG_ID + 1, session.id, "Cross-org.", 99)
            .await
            .expect("cross-org summary"),
        "{label}: the write is org-scoped"
    );
}

#[tokio::test]
async fn in_memory_run_summary_fence() {
    let backend = StorageBackend::in_memory();
    run_run_summary_fence_conformance(&backend, "memory").await;
}

#[tokio::test]
async fn postgres_run_summary_fence() {
    let backend = create_postgres_backend().await;
    run_run_summary_fence_conformance(&backend, "postgres").await;
}

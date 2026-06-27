//! Dual-backend conformance tests for storage behavior that has drifted before.
//!
//! Run with: cargo test -p everruns-server --test repository_conformance_test -- --test-threads=1

mod test_harness;

use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use everruns_core::message_filter::MessageQuery;
use everruns_core::{AgentId, DEFAULT_ORG_ID, PrincipalId};
use everruns_server::storage::{
    CreateAgentRow, CreateBudgetRow, CreateEventRow, CreatePrincipalRow, CreateProviderRow,
    CreateSessionRow, CreateUsageJournalRow, CreateUsageLedgerRow, Database, MESSAGE_SAFETY_LIMIT,
    Repository, StorageBackend,
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
        org_id: DEFAULT_ORG_ID,
        app_id: None,
        harness_id: None,
        agent_id: None,
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
        workspace_id: None,
    }
}

fn agent_input(name: String) -> CreateAgentRow {
    CreateAgentRow {
        public_id: AgentId::new().to_string(),
        name,
        display_name: Some("Conformance Agent".to_string()),
        description: None,
        system_prompt: "You are a conformance test agent.".to_string(),
        default_model_id: None,
        tags: vec![],
        initial_files: json!([]),
        tools: json!([]),
        mcp_servers: json!({}),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
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

async fn run_repository_conformance(repo: &dyn Repository, label: &str) {
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
        .upsert_agent_by_name(DEFAULT_ORG_ID, agent_input(agent_name.clone()))
        .await
        .expect("create agent by name");
    assert!(was_created);
    let (updated_agent, was_created) = repo
        .upsert_agent_by_name(DEFAULT_ORG_ID, agent_input(agent_name))
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
    assert_eq!(
        uncapped.first().expect("first capped event").sequence,
        1,
        "unbounded list_message_events_limited keeps the earliest capped window"
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

#[tokio::test]
async fn in_memory_repository_conformance() {
    let backend = StorageBackend::in_memory();
    run_repository_conformance(&backend, "memory").await;
}

#[tokio::test]
async fn postgres_repository_conformance() {
    let backend = create_postgres_backend().await;
    run_repository_conformance(&backend, "postgres").await;
}

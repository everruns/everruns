// Budget system tests
//
// Tests for BudgetService (rules engine, metering, cost computation),
// storage layer (CRUD, ledger, hierarchy), and end-to-end integration.

use crate::domains::budgets::BudgetService;
use crate::storage::StorageBackend;
use crate::storage::models::*;
use everruns_core::EventListener;
use everruns_core::budget::BudgetAction;
use everruns_core::events::{Event, EventContext, LlmGenerationData, TokenUsage};
use everruns_core::org_public_id_from_internal;
use everruns_provider::typed_id::{AgentId, PrincipalId};
use std::sync::Arc;
use uuid::Uuid;

fn make_db() -> Arc<StorageBackend> {
    Arc::new(StorageBackend::in_memory())
}

fn make_service() -> (BudgetService, Arc<StorageBackend>) {
    let db = make_db();
    let svc = BudgetService::new(db.clone());
    (svc, db)
}

async fn create_session_with_owner(
    db: &Arc<StorageBackend>,
    org_id: i64,
    agent_id: Option<AgentId>,
    resolved_owner_user_id: Option<Uuid>,
) -> SessionRow {
    create_session_with_owner_and_tags(db, org_id, agent_id, resolved_owner_user_id, vec![]).await
}

async fn create_session_with_owner_and_tags(
    db: &Arc<StorageBackend>,
    org_id: i64,
    agent_id: Option<AgentId>,
    resolved_owner_user_id: Option<Uuid>,
    tags: Vec<String>,
) -> SessionRow {
    db.create_session(CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id,
        app_id: None,
        harness_id: None,
        agent_id,
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: None,
        owner_principal_id: PrincipalId::new(),
        resolved_owner_user_id,
        title: Some("Budget test session".into()),
        locale: None,
        tags,
        model_id: None,
        capabilities: serde_json::json!({}),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!([]),
        system_prompt: None,
        initial_files: serde_json::json!({}),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: None,
        budget_root_session_id: None,
    })
    .await
    .unwrap()
}

async fn create_child_session(
    db: &Arc<StorageBackend>,
    parent: &SessionRow,
    agent_id: Option<AgentId>,
) -> SessionRow {
    db.create_session(CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id: parent.org_id,
        app_id: parent.app_id,
        harness_id: parent.harness_id,
        agent_id,
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: parent.agent_identity_id,
        owner_principal_id: parent.owner_principal_id,
        resolved_owner_user_id: parent.resolved_owner_user_id,
        title: Some("Budget test child session".into()),
        locale: parent.locale.clone(),
        tags: parent.tags.clone(),
        model_id: parent.model_id,
        capabilities: serde_json::json!({}),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!([]),
        system_prompt: None,
        initial_files: serde_json::json!({}),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: Some(parent.id),
        budget_root_session_id: None,
    })
    .await
    .unwrap()
}

async fn create_detached_session(db: &Arc<StorageBackend>, origin: &SessionRow) -> SessionRow {
    let input = CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id: origin.org_id,
        app_id: origin.app_id,
        harness_id: origin.harness_id,
        agent_id: origin.agent_id,
        agent_identity_id: origin.agent_identity_id,
        agent_version_id: None,
        agent_config_hash: None,
        owner_principal_id: origin.owner_principal_id,
        resolved_owner_user_id: origin.resolved_owner_user_id,
        title: Some("Detached budget peer".into()),
        locale: origin.locale.clone(),
        tags: origin.tags.clone(),
        model_id: origin.model_id,
        capabilities: serde_json::json!({}),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!([]),
        system_prompt: None,
        initial_files: serde_json::json!({}),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: None,
        budget_root_session_id: Some(origin.id),
    };
    db.create_session(input).await.unwrap()
}

fn make_budget_row(limit: f64, balance: f64, soft_limit: Option<f64>, currency: &str) -> BudgetRow {
    BudgetRow {
        id: uuid::Uuid::now_v7(),
        org_id: 1,
        subject_type: "session".into(),
        subject_id: "session_test".into(),
        currency: currency.into(),
        limit,
        soft_limit,
        balance,
        period: None,
        period_started_at: None,
        metadata: None,
        status: "active".into(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

// ========================================================================
// Rules engine tests
// ========================================================================

#[test]
fn test_rule_continue_when_budget_healthy() {
    let (svc, _) = make_service();
    let budget = make_budget_row(100.0, 80.0, None, "usd");
    let action = svc.evaluate_rules(&budget);
    assert_eq!(action, BudgetAction::Continue);
}

#[test]
fn test_rule_warn_at_80_percent_consumed() {
    let (svc, _) = make_service();
    let budget = make_budget_row(100.0, 15.0, None, "usd");
    let action = svc.evaluate_rules(&budget);
    assert!(matches!(action, BudgetAction::Warn { .. }));
}

#[test]
fn test_rule_warn_at_exactly_20_percent_remaining() {
    let (svc, _) = make_service();
    let budget = make_budget_row(100.0, 20.0, None, "usd");
    let action = svc.evaluate_rules(&budget);
    assert!(matches!(action, BudgetAction::Warn { .. }));
}

#[test]
fn test_rule_no_warn_above_20_percent() {
    let (svc, _) = make_service();
    let budget = make_budget_row(100.0, 21.0, None, "usd");
    let action = svc.evaluate_rules(&budget);
    assert_eq!(action, BudgetAction::Continue);
}

#[test]
fn test_rule_stop_when_balance_zero() {
    let (svc, _) = make_service();
    let budget = make_budget_row(100.0, 0.0, None, "usd");
    let action = svc.evaluate_rules(&budget);
    assert!(matches!(action, BudgetAction::Stop { .. }));
}

#[test]
fn test_rule_stop_when_balance_negative() {
    let (svc, _) = make_service();
    let budget = make_budget_row(100.0, -5.0, None, "usd");
    let action = svc.evaluate_rules(&budget);
    assert!(matches!(action, BudgetAction::Stop { .. }));
}

#[test]
fn test_rule_pause_at_soft_limit() {
    let (svc, _) = make_service();
    let budget = make_budget_row(100.0, 15.0, Some(80.0), "usd");
    let action = svc.evaluate_rules(&budget);
    assert!(matches!(action, BudgetAction::Pause { .. }));
}

#[test]
fn test_rule_no_pause_before_soft_limit() {
    let (svc, _) = make_service();
    let budget = make_budget_row(100.0, 25.0, Some(80.0), "usd");
    let action = svc.evaluate_rules(&budget);
    assert_eq!(action, BudgetAction::Continue);
}

#[test]
fn test_rule_stop_takes_priority_over_pause() {
    let (svc, _) = make_service();
    let budget = make_budget_row(100.0, 0.0, Some(80.0), "usd");
    let action = svc.evaluate_rules(&budget);
    assert!(matches!(action, BudgetAction::Stop { .. }));
}

#[test]
fn test_rule_stop_message_includes_currency() {
    let (svc, _) = make_service();
    let budget = make_budget_row(50.0, -2.0, None, "tokens");
    let action = svc.evaluate_rules(&budget);
    if let BudgetAction::Stop { message } = action {
        assert!(message.contains("tokens"));
    } else {
        panic!("Expected Stop action");
    }
}

// ========================================================================
// Metering / cost computation tests
// ========================================================================

#[test]
fn test_metered_total_tokens_includes_disjoint_cache_buckets() {
    assert_eq!(
        BudgetService::metered_total_tokens(200_000, 0, 800_000, 25_000),
        1_025_000
    );
}

#[test]
fn test_compute_debit_tokens_currency() {
    let (svc, _) = make_service();
    let debit = svc.compute_debit(
        "tokens",
        1500,
        1000,
        500,
        0,
        0,
        Some("gpt-4o"),
        Some("openai"),
        None,
    );
    assert_eq!(debit, 1500.0);
}

#[test]
fn test_compute_debit_usd_with_known_model() {
    let (svc, _) = make_service();
    let debit = svc.compute_debit(
        "usd",
        1_000_000 + 500_000,
        1_000_000,
        500_000,
        0,
        0,
        Some("gpt-4o"),
        Some("openai"),
        None,
    );
    assert!(debit > 0.0, "Debit should be positive for known model");
    assert!(debit < 100.0, "Debit should be reasonable for 1.5M tokens");
}

#[test]
fn test_compute_debit_usd_prefers_provider_cost() {
    let (svc, _) = make_service();
    // A provider-reported cost wins over the price-table estimate, even for a
    // known model — it is the authoritative charge (e.g. OpenRouter usage.cost).
    let debit = svc.compute_debit(
        "usd",
        1_500_000,
        1_000_000,
        500_000,
        0,
        0,
        Some("gpt-4o"),
        Some("openai"),
        Some(0.0123),
    );
    assert_eq!(debit, 0.0123);
}

#[test]
fn test_compute_debit_usd_provider_cost_covers_unknown_model() {
    let (svc, _) = make_service();
    // Without a profile we would fall back to raw token count; the provider's
    // cost rescues USD metering for the long tail of models.
    let debit = svc.compute_debit(
        "usd",
        1500,
        1000,
        500,
        0,
        0,
        Some("some/exotic-model"),
        Some("openai"),
        Some(0.0042),
    );
    assert_eq!(debit, 0.0042);
}

#[test]
fn test_compute_debit_usd_zero_provider_cost_falls_back() {
    let (svc, _) = make_service();
    // A zero/absent cost must not short-circuit metering; fall back as before.
    let debit = svc.compute_debit(
        "usd",
        1500,
        1000,
        500,
        0,
        0,
        Some("unknown-model"),
        Some("openai"),
        Some(0.0),
    );
    assert_eq!(debit, 1500.0);
}

#[test]
fn test_compute_debit_raw_token_quotas_include_cached_prompt_tokens() {
    let (svc, _) = make_service();
    let total_tokens = BudgetService::metered_total_tokens(200_000, 0, 800_000, 0);

    assert_eq!(
        svc.compute_debit(
            "tokens",
            total_tokens,
            200_000,
            0,
            800_000,
            0,
            None,
            None,
            None
        ),
        1_000_000.0
    );
    assert_eq!(
        svc.compute_debit(
            "credits",
            total_tokens,
            200_000,
            0,
            800_000,
            0,
            None,
            None,
            None
        ),
        1_000.0
    );
    assert_eq!(
        svc.compute_debit(
            "custom",
            total_tokens,
            200_000,
            0,
            800_000,
            0,
            None,
            None,
            None,
        ),
        1_000_000.0
    );
}

#[test]
fn test_compute_debit_usd_with_unknown_model_falls_back_to_tokens() {
    let (svc, _) = make_service();
    let debit = svc.compute_debit(
        "usd",
        1500,
        1000,
        500,
        0,
        0,
        Some("unknown-model"),
        Some("openai"),
        None,
    );
    assert_eq!(debit, 1500.0);
}

#[test]
fn test_compute_debit_credits_currency() {
    let (svc, _) = make_service();
    let debit = svc.compute_debit("credits", 5000, 3000, 2000, 0, 0, None, None, None);
    assert_eq!(debit, 5.0);
}

#[test]
fn test_compute_debit_custom_currency_uses_token_count() {
    let (svc, _) = make_service();
    let debit = svc.compute_debit("my_custom", 2000, 1500, 500, 0, 0, None, None, None);
    assert_eq!(debit, 2000.0);
}

#[test]
fn test_compute_debit_zero_tokens() {
    let (svc, _) = make_service();
    let debit = svc.compute_debit("tokens", 0, 0, 0, 0, 0, None, None, None);
    assert_eq!(debit, 0.0);
}

#[test]
fn test_compute_debit_usd_discounts_cached_tokens() {
    let (svc, _) = make_service();
    // Disjoint buckets: `input_tokens` is non-cached, `cache_read_tokens`
    // additive. gpt-4o: input $2.50/M, cache_read $1.25/M. A 1M-token prompt
    // split as 200K non-cached + 800K cache reads.
    let cached = svc.compute_debit(
        "usd",
        1_000_000,
        200_000,
        0,
        800_000,
        0,
        Some("gpt-4o"),
        Some("openai"),
        None,
    );
    // The same prompt billed cache-blind (all 1M at the full input rate).
    let naive = svc.compute_debit(
        "usd",
        1_000_000,
        1_000_000,
        0,
        0,
        0,
        Some("gpt-4o"),
        Some("openai"),
        None,
    );
    // 200K * 2.50 + 800K * 1.25 = 0.50 + 1.00 = 1.50, well below the cache-blind 2.50.
    assert!((cached - 1.50).abs() < 1e-9, "cached debit {cached}");
    assert!((naive - 2.50).abs() < 1e-9, "naive debit {naive}");
}

// ========================================================================
// Storage layer tests (in-memory)
// ========================================================================

#[tokio::test]
async fn test_create_and_get_budget() {
    let db = make_db();
    let input = CreateBudgetRow {
        org_id: 1,
        subject_type: "session".into(),
        subject_id: "session_abc".into(),
        currency: "usd".into(),
        limit: 10.0,
        soft_limit: Some(8.0),
        period: None,
        metadata: None,
    };
    let created = db.create_budget(input).await.unwrap();
    assert_eq!(created.limit, 10.0);
    assert_eq!(created.balance, 10.0);
    assert_eq!(created.soft_limit, Some(8.0));
    assert_eq!(created.currency, "usd");
    assert_eq!(created.status, "active");

    let fetched = db.get_budget(1, created.id).await.unwrap().unwrap();
    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.limit, 10.0);
}

#[tokio::test]
async fn test_get_budget_wrong_org_returns_none() {
    let db = make_db();
    let input = CreateBudgetRow {
        org_id: 1,
        subject_type: "session".into(),
        subject_id: "s".into(),
        currency: "usd".into(),
        limit: 10.0,
        soft_limit: None,
        period: None,
        metadata: None,
    };
    let created = db.create_budget(input).await.unwrap();
    assert!(db.get_budget(999, created.id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_list_budgets_filters() {
    let db = make_db();
    for (stype, sid) in [("session", "s1"), ("agent", "a1")] {
        db.create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: stype.into(),
            subject_id: sid.into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    }
    let all = db.list_budgets(1, None, None).await.unwrap();
    assert_eq!(all.len(), 2);
    let sessions = db.list_budgets(1, Some("session"), None).await.unwrap();
    assert_eq!(sessions.len(), 1);
    let specific = db.list_budgets(1, None, Some("a1")).await.unwrap();
    assert_eq!(specific.len(), 1);
}

#[tokio::test]
async fn test_update_budget_increases_balance() {
    let db = make_db();
    let created = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "s1".into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    let updated = db
        .update_budget(
            1,
            created.id,
            UpdateBudgetRow {
                limit: Some(20.0),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.limit, 20.0);
    assert_eq!(updated.balance, 20.0);
}

#[tokio::test]
async fn test_delete_budget_soft_deletes() {
    let db = make_db();
    let created = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "s1".into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    assert!(db.delete_budget(1, created.id).await.unwrap());
    let fetched = db.get_budget(1, created.id).await.unwrap().unwrap();
    assert_eq!(fetched.status, "disabled");
    assert!(db.list_budgets(1, None, None).await.unwrap().is_empty());
}

// ========================================================================
// Ledger tests
// ========================================================================

#[tokio::test]
async fn test_ledger_entry_reduces_balance() {
    let db = make_db();
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "s1".into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    let (entry, updated) = db
        .create_budget_ledger_entry(CreateBudgetLedgerRow {
            budget_id: budget.id,
            amount: 3.0,
            meter_source: "llm_tokens".into(),
            ref_type: Some("llm_generation".into()),
            ref_id: Some(uuid::Uuid::now_v7()),
            session_id: None,
            description: Some("test debit".into()),
        })
        .await
        .unwrap();
    assert_eq!(entry.amount, 3.0);
    assert_eq!(updated.balance, 7.0);
}

#[tokio::test]
async fn test_budget_ledger_entry_creates_usage_journal_link() {
    let db = make_db();
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "s1".into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    let (entry, _) = db
        .create_budget_ledger_entry(CreateBudgetLedgerRow {
            budget_id: budget.id,
            amount: 3.0,
            meter_source: "llm_tokens".into(),
            ref_type: Some("llm_generation".into()),
            ref_id: Some(uuid::Uuid::now_v7()),
            session_id: None,
            description: Some("test debit".into()),
        })
        .await
        .unwrap();

    let journal = db
        .get_usage_journal(entry.journal_id)
        .await
        .unwrap()
        .expect("journal should be created");
    assert_eq!(journal.kind, "budget_adjustment");
    assert_eq!(journal.org_id, budget.org_id);
    assert_eq!(journal.measures["amount"], serde_json::json!(3.0));
}

#[tokio::test]
async fn test_ledger_multiple_debits_accumulate() {
    let db = make_db();
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "s1".into(),
            currency: "tokens".into(),
            limit: 1000.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    for _ in 0..5 {
        db.create_budget_ledger_entry(CreateBudgetLedgerRow {
            budget_id: budget.id,
            amount: 100.0,
            meter_source: "llm_tokens".into(),
            ref_type: None,
            ref_id: None,
            session_id: None,
            description: None,
        })
        .await
        .unwrap();
    }
    let fetched = db.get_budget(1, budget.id).await.unwrap().unwrap();
    assert_eq!(fetched.balance, 500.0);
}

#[tokio::test]
async fn test_ledger_negative_amount_is_credit() {
    let db = make_db();
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "s1".into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    db.create_budget_ledger_entry(CreateBudgetLedgerRow {
        budget_id: budget.id,
        amount: 8.0,
        meter_source: "llm_tokens".into(),
        ref_type: None,
        ref_id: None,
        session_id: None,
        description: None,
    })
    .await
    .unwrap();
    let (_, updated) = db
        .create_budget_ledger_entry(CreateBudgetLedgerRow {
            budget_id: budget.id,
            amount: -5.0,
            meter_source: "manual".into(),
            ref_type: Some("top_up".into()),
            ref_id: None,
            session_id: None,
            description: Some("top up".into()),
        })
        .await
        .unwrap();
    assert_eq!(updated.balance, 7.0);
}

#[tokio::test]
async fn test_list_ledger_pagination() {
    let db = make_db();
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "s1".into(),
            currency: "tokens".into(),
            limit: 10000.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    for i in 0..10 {
        db.create_budget_ledger_entry(CreateBudgetLedgerRow {
            budget_id: budget.id,
            amount: (i + 1) as f64,
            meter_source: "llm_tokens".into(),
            ref_type: None,
            ref_id: None,
            session_id: None,
            description: Some(format!("entry {i}")),
        })
        .await
        .unwrap();
    }
    assert_eq!(
        db.list_budget_ledger(budget.id, 3, 0).await.unwrap().len(),
        3
    );
    assert_eq!(
        db.list_budget_ledger(budget.id, 3, 3).await.unwrap().len(),
        3
    );
    assert_eq!(
        db.list_budget_ledger(budget.id, 100, 0)
            .await
            .unwrap()
            .len(),
        10
    );
}

// ========================================================================
// Budget hierarchy tests
// ========================================================================

#[tokio::test]
async fn test_active_budgets_for_session_hierarchy() {
    let db = make_db();
    db.create_budget(CreateBudgetRow {
        org_id: 1,
        subject_type: "session".into(),
        subject_id: "session_1".into(),
        currency: "usd".into(),
        limit: 10.0,
        soft_limit: None,
        period: None,
        metadata: None,
    })
    .await
    .unwrap();
    db.create_budget(CreateBudgetRow {
        org_id: 1,
        subject_type: "agent".into(),
        subject_id: "agent_1".into(),
        currency: "usd".into(),
        limit: 50.0,
        soft_limit: None,
        period: None,
        metadata: None,
    })
    .await
    .unwrap();
    db.create_budget(CreateBudgetRow {
        org_id: 1,
        subject_type: "session".into(),
        subject_id: "session_other".into(),
        currency: "usd".into(),
        limit: 999.0,
        soft_limit: None,
        period: None,
        metadata: None,
    })
    .await
    .unwrap();

    let budgets = db
        .get_active_budgets_for_session(1, "session_1", Some("agent_1"), None, None)
        .await
        .unwrap();
    assert_eq!(budgets.len(), 2);
    let subjects: Vec<&str> = budgets.iter().map(|b| b.subject_id.as_str()).collect();
    assert!(subjects.contains(&"session_1"));
    assert!(subjects.contains(&"agent_1"));
    assert!(!subjects.contains(&"session_other"));
}

#[tokio::test]
async fn test_active_budgets_excludes_disabled() {
    let db = make_db();
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "session_1".into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    db.delete_budget(1, budget.id).await.unwrap();
    assert!(
        db.get_active_budgets_for_session(1, "session_1", None, None, None)
            .await
            .unwrap()
            .is_empty()
    );
}

// ========================================================================
// End-to-end: check_budgets_for_session
// ========================================================================

#[tokio::test]
async fn test_check_budgets_no_budgets_returns_ok() {
    let (svc, _) = make_service();
    let result = svc.check_budgets_for_session(1, "session_1", None).await;
    assert_eq!(result.action, "continue");
}

#[tokio::test]
async fn test_check_budgets_healthy_budget_returns_continue() {
    let (svc, db) = make_service();
    db.create_budget(CreateBudgetRow {
        org_id: 1,
        subject_type: "session".into(),
        subject_id: "session_1".into(),
        currency: "usd".into(),
        limit: 100.0,
        soft_limit: None,
        period: None,
        metadata: None,
    })
    .await
    .unwrap();
    let result = svc.check_budgets_for_session(1, "session_1", None).await;
    assert_eq!(result.action, "continue");
}

#[tokio::test]
async fn test_check_budgets_exhausted_budget_returns_stop() {
    let (svc, db) = make_service();
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "session_1".into(),
            currency: "tokens".into(),
            limit: 100.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    db.create_budget_ledger_entry(CreateBudgetLedgerRow {
        budget_id: budget.id,
        amount: 100.0,
        meter_source: "llm_tokens".into(),
        ref_type: None,
        ref_id: None,
        session_id: None,
        description: None,
    })
    .await
    .unwrap();
    db.set_budget_status(budget.id, "exhausted").await.unwrap();
    let result = svc.check_budgets_for_session(1, "session_1", None).await;
    assert!(result.should_stop());
    assert_eq!(
        result.message.as_deref(),
        Some(
            "Budget exhausted. 100.00 tokens spent reached the 100.00 tokens limit. Increase the budget to continue."
        )
    );
    assert_eq!(result.error_code.as_deref(), Some("budget_exhausted"));
    assert_eq!(
        result
            .error_fields
            .as_ref()
            .and_then(|fields| fields.get("spent")),
        Some(&serde_json::json!(100.0))
    );
    assert_eq!(
        result
            .error_fields
            .as_ref()
            .and_then(|fields| fields.get("limit")),
        Some(&serde_json::json!(100.0))
    );
    assert_eq!(
        result
            .error_fields
            .as_ref()
            .and_then(|fields| fields.get("currency")),
        Some(&serde_json::json!("tokens"))
    );
}

#[tokio::test]
async fn test_check_budgets_paused_budget_returns_pause() {
    let (svc, db) = make_service();
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "session_1".into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: Some(8.0),
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    db.create_budget_ledger_entry(CreateBudgetLedgerRow {
        budget_id: budget.id,
        amount: 8.0,
        meter_source: "llm_tokens".into(),
        ref_type: None,
        ref_id: None,
        session_id: None,
        description: None,
    })
    .await
    .unwrap();
    db.set_budget_status(budget.id, "paused").await.unwrap();
    let result = svc.check_budgets_for_session(1, "session_1", None).await;
    assert!(result.should_pause());
    assert_eq!(
        result.message.as_deref(),
        Some(
            "Budget paused. 8.00 usd spent reached the 8.00 usd soft limit. Increase or resume the budget to continue."
        )
    );
    assert_eq!(result.error_code.as_deref(), Some("budget_paused"));
    assert_eq!(
        result
            .error_fields
            .as_ref()
            .and_then(|fields| fields.get("soft_limit")),
        Some(&serde_json::json!(8.0))
    );
}

#[tokio::test]
async fn test_check_budgets_manually_paused_budget_uses_neutral_message() {
    let (svc, db) = make_service();
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "session_1".into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: Some(8.0),
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    db.set_budget_status(budget.id, "paused").await.unwrap();

    let result = svc.check_budgets_for_session(1, "session_1", None).await;

    assert!(result.should_pause());
    assert_eq!(
        result.message.as_deref(),
        Some("Budget paused with 0.00 usd spent. Increase or resume the budget to continue.")
    );
}

#[tokio::test]
async fn test_check_budgets_paused_budget_above_soft_limit_uses_exceeded_message() {
    let (svc, db) = make_service();
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "session_1".into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: Some(8.0),
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    db.create_budget_ledger_entry(CreateBudgetLedgerRow {
        budget_id: budget.id,
        amount: 9.0,
        meter_source: "llm_tokens".into(),
        ref_type: None,
        ref_id: None,
        session_id: None,
        description: None,
    })
    .await
    .unwrap();
    db.set_budget_status(budget.id, "paused").await.unwrap();

    let result = svc.check_budgets_for_session(1, "session_1", None).await;

    assert!(result.should_pause());
    assert_eq!(
        result.message.as_deref(),
        Some(
            "Budget paused. 9.00 usd spent exceeded the 8.00 usd soft limit. Increase or resume the budget to continue."
        )
    );
}

#[tokio::test]
async fn test_check_budgets_most_restrictive_wins() {
    let (svc, db) = make_service();
    db.create_budget(CreateBudgetRow {
        org_id: 1,
        subject_type: "session".into(),
        subject_id: "session_1".into(),
        currency: "usd".into(),
        limit: 10.0,
        soft_limit: None,
        period: None,
        metadata: None,
    })
    .await
    .unwrap();
    let agent_budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "agent".into(),
            subject_id: "agent_1".into(),
            currency: "usd".into(),
            limit: 50.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    db.create_budget_ledger_entry(CreateBudgetLedgerRow {
        budget_id: agent_budget.id,
        amount: 50.0,
        meter_source: "llm_tokens".into(),
        ref_type: None,
        ref_id: None,
        session_id: None,
        description: None,
    })
    .await
    .unwrap();
    db.set_budget_status(agent_budget.id, "exhausted")
        .await
        .unwrap();
    // Session budget healthy, but agent budget exhausted → should stop
    let result = svc
        .check_budgets_for_session(1, "session_1", Some("agent_1"))
        .await;
    assert_eq!(result.action, "stop");
}

#[tokio::test]
async fn test_list_budgets_for_session_hierarchy_resolves_user_and_org_from_session() {
    let (svc, db) = make_service();
    let user_id = Uuid::now_v7();
    let agent_id = AgentId::new();
    let session = create_session_with_owner(&db, 1, Some(agent_id), Some(user_id)).await;
    let session_public_id = session.id.to_string();
    let org_public_id = org_public_id_from_internal(session.org_id);

    for (subject_type, subject_id) in [
        ("session", session_public_id.clone()),
        ("agent", agent_id.to_string()),
        ("user", user_id.to_string()),
        ("org", org_public_id.clone()),
    ] {
        db.create_budget(CreateBudgetRow {
            org_id: session.org_id,
            subject_type: subject_type.into(),
            subject_id,
            currency: "usd".into(),
            limit: 25.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    }

    let budgets = svc
        .list_budgets_for_session_hierarchy(session.org_id, &session_public_id, None)
        .await;

    let subjects: Vec<(&str, &str)> = budgets
        .iter()
        .map(|budget| (budget.subject_type.as_str(), budget.subject_id.as_str()))
        .collect();
    assert!(subjects.contains(&("session", session_public_id.as_str())));
    assert!(subjects.contains(&("agent", agent_id.to_string().as_str())));
    assert!(subjects.contains(&("user", user_id.to_string().as_str())));
    assert!(subjects.contains(&("org", org_public_id.as_str())));
}

#[tokio::test]
async fn test_list_budgets_for_session_hierarchy_includes_app_and_channel_from_tags() {
    let (svc, db) = make_service();
    let session = create_session_with_owner_and_tags(
        &db,
        1,
        None,
        None,
        vec![
            "app:app_for_budget_test".to_string(),
            "app_channel:appchan_for_budget_test".to_string(),
        ],
    )
    .await;
    let session_public_id = session.id.to_string();

    db.create_budget(CreateBudgetRow {
        org_id: session.org_id,
        subject_type: "app".into(),
        subject_id: "app_for_budget_test".into(),
        currency: "usd".into(),
        limit: 50.0,
        soft_limit: None,
        period: None,
        metadata: None,
    })
    .await
    .unwrap();
    db.create_budget(CreateBudgetRow {
        org_id: session.org_id,
        subject_type: "app_channel".into(),
        subject_id: "appchan_for_budget_test".into(),
        currency: "usd".into(),
        limit: 5.0,
        soft_limit: None,
        period: None,
        metadata: None,
    })
    .await
    .unwrap();

    let budgets = svc
        .list_budgets_for_session_hierarchy(session.org_id, &session_public_id, None)
        .await;
    let subjects: Vec<&str> = budgets.iter().map(|b| b.subject_type.as_str()).collect();
    assert!(subjects.contains(&"app"), "subjects: {subjects:?}");
    assert!(subjects.contains(&"app_channel"), "subjects: {subjects:?}");
}

#[tokio::test]
async fn test_child_session_uses_root_session_budget_pool() {
    let (svc, db) = make_service();
    let root = create_session_with_owner(&db, 1, None, None).await;
    let child = create_child_session(&db, &root, None).await;
    let grandchild = create_child_session(&db, &child, None).await;
    let root_subject_id = root.id.to_string();
    let child_subject_id = child.id.to_string();
    let grandchild_subject_id = grandchild.id.to_string();

    db.create_budget(CreateBudgetRow {
        org_id: root.org_id,
        subject_type: "session".into(),
        subject_id: root_subject_id.clone(),
        currency: "tokens".into(),
        limit: 100.0,
        soft_limit: None,
        period: None,
        metadata: None,
    })
    .await
    .unwrap();
    db.create_budget(CreateBudgetRow {
        org_id: child.org_id,
        subject_type: "session".into(),
        subject_id: child_subject_id.clone(),
        currency: "tokens".into(),
        limit: 1.0,
        soft_limit: None,
        period: None,
        metadata: None,
    })
    .await
    .unwrap();

    let budgets = svc
        .list_budgets_for_session_hierarchy(grandchild.org_id, &grandchild_subject_id, None)
        .await;
    let session_subjects: Vec<&str> = budgets
        .iter()
        .filter(|budget| budget.subject_type == "session")
        .map(|budget| budget.subject_id.as_str())
        .collect();

    assert_eq!(session_subjects, vec![root_subject_id.as_str()]);
    assert!(
        !session_subjects.contains(&child_subject_id.as_str()),
        "descendant-local budgets must not split the shared root pool"
    );
}

#[tokio::test]
async fn test_check_budgets_for_child_respects_exhausted_root_budget() {
    let (svc, db) = make_service();
    let root = create_session_with_owner(&db, 1, None, None).await;
    let child = create_child_session(&db, &root, None).await;
    let root_subject_id = root.id.to_string();
    let child_subject_id = child.id.to_string();

    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: root.org_id,
            subject_type: "session".into(),
            subject_id: root_subject_id,
            currency: "tokens".into(),
            limit: 100.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    db.create_budget_ledger_entry(CreateBudgetLedgerRow {
        budget_id: budget.id,
        amount: 100.0,
        meter_source: "llm_tokens".into(),
        ref_type: None,
        ref_id: None,
        session_id: Some(child.id.uuid()),
        description: None,
    })
    .await
    .unwrap();
    db.set_budget_status(budget.id, "exhausted").await.unwrap();

    let result = svc
        .check_budgets_for_session(child.org_id, &child_subject_id, None)
        .await;

    assert!(result.should_stop());
    assert_eq!(
        result.budget_id,
        Some(everruns_provider::typed_id::BudgetId::from_uuid(budget.id))
    );
    assert_eq!(result.error_code.as_deref(), Some("budget_exhausted"));
}

#[tokio::test]
async fn test_period_resets_balance_after_window_elapses() {
    let (svc, db) = make_service();
    // Zero-second sliding window — rollover triggers on the next check
    // without any wall-clock sleeps. Keeps the test deterministic and fast.
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "session_period".into(),
            currency: "tokens".into(),
            limit: 100.0,
            soft_limit: None,
            period: Some(serde_json::json!({"type": "duration", "seconds": 0})),
            metadata: None,
        })
        .await
        .unwrap();

    // Burn the budget down to zero.
    db.create_budget_ledger_entry(CreateBudgetLedgerRow {
        budget_id: budget.id,
        amount: 100.0,
        meter_source: "llm_tokens".into(),
        ref_type: None,
        ref_id: None,
        session_id: None,
        description: None,
    })
    .await
    .unwrap();
    let before = db.get_budget(1, budget.id).await.unwrap().unwrap();
    assert_eq!(before.balance, 0.0);

    let refreshed = svc.maybe_reset_period(before.clone()).await;
    assert_eq!(
        refreshed.balance, 100.0,
        "period rollover should reset balance to limit"
    );
    assert!(refreshed.period_started_at.is_some());
    assert!(
        refreshed.period_started_at.unwrap() >= before.period_started_at.unwrap(),
        "rollover should advance period_started_at"
    );
}

#[tokio::test]
async fn test_period_does_not_reset_inside_window() {
    let (svc, db) = make_service();
    // Long window — same-tick check must NOT reset.
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "session_period_long".into(),
            currency: "tokens".into(),
            limit: 100.0,
            soft_limit: None,
            period: Some(serde_json::json!({"type": "duration", "seconds": 3_600})),
            metadata: None,
        })
        .await
        .unwrap();

    db.create_budget_ledger_entry(CreateBudgetLedgerRow {
        budget_id: budget.id,
        amount: 40.0,
        meter_source: "llm_tokens".into(),
        ref_type: None,
        ref_id: None,
        session_id: None,
        description: None,
    })
    .await
    .unwrap();
    let snap = db.get_budget(1, budget.id).await.unwrap().unwrap();
    assert_eq!(snap.balance, 60.0);

    let refreshed = svc.maybe_reset_period(snap.clone()).await;
    assert_eq!(
        refreshed.balance, 60.0,
        "in-window check must not reset balance"
    );
    assert_eq!(
        refreshed.period_started_at, snap.period_started_at,
        "in-window check must not advance period_started_at"
    );
}

#[tokio::test]
async fn test_check_budgets_respects_user_budget_from_session_hierarchy() {
    let (svc, db) = make_service();
    let user_id = Uuid::now_v7();
    let session = create_session_with_owner(&db, 1, None, Some(user_id)).await;
    let session_public_id = session.id.to_string();

    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: session.org_id,
            subject_type: "user".into(),
            subject_id: user_id.to_string(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();

    db.create_budget_ledger_entry(CreateBudgetLedgerRow {
        budget_id: budget.id,
        amount: 10.0,
        meter_source: "llm_tokens".into(),
        ref_type: None,
        ref_id: None,
        session_id: Some(session.id.uuid()),
        description: None,
    })
    .await
    .unwrap();
    db.set_budget_status(budget.id, "exhausted").await.unwrap();

    let result = svc
        .check_budgets_for_session(session.org_id, &session_public_id, None)
        .await;

    assert_eq!(result.action, "stop");
    assert_eq!(result.error_code.as_deref(), Some("budget_exhausted"));
}

// ========================================================================
// Set budget status + DTO conversion tests
// ========================================================================

#[tokio::test]
async fn test_set_budget_status() {
    let db = make_db();
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: 1,
            subject_type: "session".into(),
            subject_id: "s1".into(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    let updated = db
        .set_budget_status(budget.id, "paused")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.status, "paused");
    let updated2 = db
        .set_budget_status(budget.id, "active")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated2.status, "active");
}

#[test]
fn test_row_to_budget_dto() {
    let row = make_budget_row(100.0, 75.0, Some(80.0), "usd");
    let dto = BudgetService::row_to_budget(&row);
    assert_eq!(dto.limit, 100.0);
    assert_eq!(dto.balance, 75.0);
    assert_eq!(dto.soft_limit, Some(80.0));
    assert_eq!(dto.currency, "usd");
    assert!(matches!(
        dto.status,
        everruns_core::budget::BudgetStatus::Active
    ));
}

#[test]
fn test_row_to_ledger_entry_dto() {
    let row = BudgetLedgerRow {
        id: uuid::Uuid::now_v7(),
        journal_id: uuid::Uuid::now_v7(),
        budget_id: Some(uuid::Uuid::now_v7()),
        org_id: 1,
        session_id: None,
        user_id: None,
        principal_id: None,
        agent_id: None,
        harness_id: None,
        currency: "usd".into(),
        amount: 5.5,
        meter_source: "llm_tokens".into(),
        ref_type: Some("llm_generation".into()),
        ref_id: Some(uuid::Uuid::now_v7()),
        description: Some("test".into()),
        rating_metadata: None,
        created_at: chrono::Utc::now(),
    };
    let dto = BudgetService::row_to_ledger_entry(&row);
    assert_eq!(dto.amount, 5.5);
    assert_eq!(dto.meter_source, "llm_tokens");
    assert_eq!(dto.description, Some("test".into()));
}

// ========================================================================
// Delegation tree budget pool
// ========================================================================

#[tokio::test]
async fn test_llm_generation_from_child_debits_root_session_budget() {
    let db = make_db();
    let svc = BudgetService::new(db.clone());

    let root = create_session_with_owner(&db, 1, None, None).await;
    let child = create_child_session(&db, &root, None).await;
    let root_subject_id = root.id.to_string();

    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: root.org_id,
            subject_type: "session".into(),
            subject_id: root_subject_id,
            currency: "tokens".into(),
            limit: 1_000.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();

    let data = LlmGenerationData::success(
        vec![],
        vec![],
        Some("child spent tokens".into()),
        vec![],
        "gpt-5.4-mini".into(),
        Some("openai".into()),
        Some(TokenUsage::new(100, 50)),
        Some(42),
        Some(10),
    );
    let event = Event::new(child.id, EventContext::empty(), data);

    svc.on_event(&event).await;

    let ledger_entries = db
        .list_usage_ledger_for_budget(budget.id, 10, 0)
        .await
        .unwrap();
    assert_eq!(
        ledger_entries.len(),
        1,
        "child generation should debit the root session budget"
    );
    assert_eq!(ledger_entries[0].session_id, Some(child.id.uuid()));
    assert_eq!(ledger_entries[0].amount, 150.0);

    let updated = db
        .get_budget(root.org_id, budget.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.balance, 850.0);
}

#[tokio::test]
async fn test_detached_chain_spend_debits_and_exhausts_origin_root_budget() {
    let db = make_db();
    let svc = BudgetService::new(db.clone());
    let root = create_session_with_owner(&db, 1, None, None).await;
    let detached = create_detached_session(&db, &root).await;
    let chained = create_detached_session(&db, &detached).await;
    assert_eq!(chained.root_session_id, Some(root.id));

    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: root.org_id,
            subject_type: "session".into(),
            subject_id: root.id.to_string(),
            currency: "tokens".into(),
            limit: 150.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    let data = LlmGenerationData::success(
        vec![],
        vec![],
        Some("detached chain spent tokens".into()),
        vec![],
        "gpt-5.4-mini".into(),
        Some("openai".into()),
        Some(TokenUsage::new(100, 50)),
        None,
        None,
    );
    svc.on_event(&Event::new(chained.id, EventContext::empty(), data))
        .await;

    let updated = db
        .get_budget(root.org_id, budget.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.balance, 0.0);
    let action = svc
        .check_budgets_for_session(chained.org_id, &chained.id.to_string(), None)
        .await;
    assert_eq!(action.action, "stop");
}

// ========================================================================
// Listener: unpersisted llm.generation event journaling (regression: EVE-416)
// ========================================================================

#[tokio::test]
async fn test_llm_generation_listener_omits_event_id_for_unpersisted_events() {
    // Regression for EVE-416: listener inputs without an events-table sequence
    // do not have a durable FK target. The listener must omit the FK reference
    // while still journaling the generation. The public id is preserved via
    // `source_type`/`source_id`.
    let db = make_db();
    let svc = BudgetService::new(db.clone());

    let session = create_session_with_owner(&db, 1, None, None).await;
    let session_id = session.id;
    let session_subject_id = session_id.to_string();

    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: session.org_id,
            subject_type: "session".into(),
            subject_id: session_subject_id.clone(),
            currency: "tokens".into(),
            limit: 10_000.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();

    let data = LlmGenerationData::success(
        vec![],
        vec![],
        Some("hello".into()),
        vec![],
        "gpt-5.4-mini".into(),
        Some("openai".into()),
        Some(TokenUsage::new(100, 50)),
        Some(42),
        Some(10),
    );
    let event = Event::new(session_id, EventContext::empty(), data);
    let public_event_id = event.id.to_string();

    assert!(event.sequence.is_none(), "test input must be unpersisted");

    svc.on_event(&event).await;

    // The journal should be created via the ledger entry attached to the budget.
    let ledger_entries = db
        .list_usage_ledger_for_budget(budget.id, 10, 0)
        .await
        .unwrap();
    assert_eq!(
        ledger_entries.len(),
        1,
        "expected exactly one ledger entry for the llm generation"
    );
    let journal = db
        .get_usage_journal(ledger_entries[0].journal_id)
        .await
        .unwrap()
        .expect("journal row should exist for the ledger entry");

    assert!(
        journal.event_id.is_none(),
        "usage_journal.event_id must be NULL for unpersisted source events to avoid FK violation, got {:?}",
        journal.event_id
    );
    assert_eq!(journal.kind, "llm_generation");
    assert_eq!(journal.source_type.as_deref(), Some("event"));
    assert_eq!(journal.source_id.as_deref(), Some(public_event_id.as_str()));
}

// Org costs and usage policies: every usage is recorded with its cost and
// where that cost came from, policies debit what the usage cost, ask steps are
// answered per session, and a host's charge replaces the estimate.

use super::tests::{create_session_with_owner, make_db};
use crate::domains::budgets::{BudgetService, HostCharge};
use crate::storage::costs::CreateCostRateRow;
use crate::storage::{CreateBudgetRow, StorageBackend, UpdateBudgetRow};
use everruns_core::EventListener;
use everruns_core::budget::{PolicySource, PolicyStep, PolicyStepAction, UsagePolicy};
use everruns_core::events::{Event, EventContext, LlmGenerationData, TokenUsage};
use std::sync::Arc;

fn generation(model: &str, usage: TokenUsage) -> LlmGenerationData {
    LlmGenerationData::success(
        vec![],
        vec![],
        None,
        vec![],
        model.into(),
        Some("openai".into()),
        Some(usage),
        None,
        None,
    )
}

fn usage(input: u32, output: u32, actual: Option<f64>, estimate: Option<f64>) -> TokenUsage {
    let mut usage = TokenUsage::new(input, output);
    usage.actual_cost_usd = actual;
    usage.estimated_cost_usd = estimate;
    usage
}

async fn policy(
    db: &Arc<StorageBackend>,
    org_id: i64,
    subject_type: &str,
    subject_id: &str,
    currency: &str,
    limit: f64,
    steps: Vec<PolicyStep>,
) -> crate::storage::BudgetRow {
    let budget = db
        .create_budget(CreateBudgetRow {
            org_id,
            subject_type: subject_type.into(),
            subject_id: subject_id.into(),
            currency: currency.into(),
            limit,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    let policy = UsagePolicy {
        name: Some("Test policy".into()),
        steps,
        source: PolicySource::All,
    };
    db.update_budget(
        org_id,
        budget.id,
        UpdateBudgetRow {
            policy: Some(serde_json::to_value(policy).unwrap()),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .unwrap()
}

fn step(at_percent: f64, action: PolicyStepAction) -> PolicyStep {
    PolicyStep { at_percent, action }
}

async fn cost_of(db: &Arc<StorageBackend>, org_id: i64, event: &Event) -> (Option<f64>, bool) {
    let journal = db
        .usage_journal_by_source(org_id, "event", &event.id.to_string())
        .await
        .unwrap()
        .expect("every usage is recorded, with or without a policy");
    db.journal_cost(journal.id).await.unwrap()
}

#[tokio::test]
async fn usage_is_recorded_with_its_cost_even_without_any_policy() {
    let db = make_db();
    let svc = BudgetService::new(db.clone());
    let session = create_session_with_owner(&db, 1, None, None).await;

    let event = Event::new(
        session.id,
        EventContext::empty(),
        generation("gpt-6.1-sol", usage(1000, 500, Some(0.25), Some(0.4))),
    );
    svc.on_event(&event).await;

    // The provider's own figure wins over the catalog estimate.
    assert_eq!(
        cost_of(&db, session.org_id, &event).await,
        (Some(0.25), false)
    );
}

#[tokio::test]
async fn unknown_price_is_unpriced_and_never_debits_a_dollar_policy() {
    let db = make_db();
    let svc = BudgetService::new(db.clone());
    let session = create_session_with_owner(&db, 1, None, None).await;
    let sid = session.id.to_string();
    let dollars = policy(&db, session.org_id, "session", &sid, "usd", 10.0, vec![]).await;
    let tokens = policy(
        &db,
        session.org_id,
        "session",
        &sid,
        "tokens",
        10_000.0,
        vec![],
    )
    .await;

    let event = Event::new(
        session.id,
        EventContext::empty(),
        generation("some/exotic-model", usage(1500, 500, None, None)),
    );
    svc.on_event(&event).await;

    assert_eq!(cost_of(&db, session.org_id, &event).await, (None, false));
    let dollars = db
        .get_budget(session.org_id, dollars.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(dollars.balance, 10.0, "an unknown cost is not 2000 dollars");
    let tokens = db
        .get_budget(session.org_id, tokens.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tokens.balance, 8_000.0, "token policies still count tokens");
}

#[tokio::test]
async fn price_list_rate_sets_the_cost_and_the_debit() {
    let db = make_db();
    let svc = BudgetService::new(db.clone());
    let session = create_session_with_owner(&db, 1, None, None).await;
    let budget = policy(
        &db,
        session.org_id,
        "session",
        &session.id.to_string(),
        "usd",
        10.0,
        vec![],
    )
    .await;
    let rate = db
        .create_cost_rate(CreateCostRateRow {
            org_id: session.org_id,
            meter: "model.tokens".into(),
            match_: serde_json::json!({"provider": "openai"}),
            unit_price_usd: None,
            list_price_multiplier: Some(0.5),
            unit: "1m_tokens".into(),
            effective_from: chrono::Utc::now() - chrono::Duration::minutes(1),
            retired: false,
            reason: Some("negotiated discount".into()),
            created_by: None,
        })
        .await
        .unwrap();

    let event = Event::new(
        session.id,
        EventContext::empty(),
        generation("gpt-6.1-sol", usage(1000, 500, Some(2.0), None)),
    );
    svc.on_event(&event).await;

    assert_eq!(
        cost_of(&db, session.org_id, &event).await,
        (Some(1.0), false)
    );
    let entries = db
        .list_usage_ledger_for_budget(budget.id, 10, 0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].amount, 1.0);
    assert_eq!(entries[0].basis.as_deref(), Some("rate"));
    assert_eq!(entries[0].rate_id, Some(rate.id));
}

#[tokio::test]
async fn host_charge_replaces_the_estimate_once() {
    let db = make_db();
    let svc = BudgetService::new(db.clone());
    let session = create_session_with_owner(&db, 1, None, None).await;
    let budget = policy(
        &db,
        session.org_id,
        "session",
        &session.id.to_string(),
        "usd",
        10.0,
        vec![],
    )
    .await;
    let event = Event::new(
        session.id,
        EventContext::empty(),
        generation("gpt-6.1-sol", usage(1000, 500, Some(1.0), None)),
    );
    svc.on_event(&event).await;

    let source_id = event.id.to_string();
    let charge = HostCharge {
        org_id: session.org_id,
        source_type: "event",
        source_id: &source_id,
        amount_usd: 1.2,
        label: "Everruns credits",
        service: "llm",
    };
    assert!(svc.record_host_charge(charge).await.unwrap());
    assert!(
        !svc.record_host_charge(charge).await.unwrap(),
        "a second report changes nothing"
    );

    let (amount, by_host) = cost_of(&db, session.org_id, &event).await;
    assert!(by_host);
    assert!((amount.unwrap() - 1.2).abs() < 1e-9);
    let budget = db
        .get_budget(session.org_id, budget.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        (budget.balance - 8.8).abs() < 1e-9,
        "a $10 policy counts what the host charged, got {}",
        budget.balance
    );

    let unknown = HostCharge {
        source_id: "evt_missing",
        ..charge
    };
    assert!(!svc.record_host_charge(unknown).await.unwrap());
}

#[tokio::test]
async fn ask_step_is_answered_per_session_and_stop_still_stops() {
    let db = make_db();
    let svc = BudgetService::new(db.clone());
    let session = create_session_with_owner(&db, 1, None, None).await;
    let other = create_session_with_owner(&db, 1, None, None).await;
    let org = everruns_core::org_public_id_from_internal(session.org_id);
    let budget = policy(
        &db,
        session.org_id,
        "org",
        &org,
        "usd",
        10.0,
        vec![
            step(50.0, PolicyStepAction::Ask),
            step(100.0, PolicyStepAction::Stop),
        ],
    )
    .await;

    let event = Event::new(
        session.id,
        EventContext::empty(),
        generation("gpt-6.1-sol", usage(1000, 500, Some(6.0), None)),
    );
    svc.on_event(&event).await;

    let sid = session.id.to_string();
    let check = svc
        .check_budgets_for_session(session.org_id, &sid, None)
        .await;
    assert_eq!(check.action, "pause");
    assert_eq!(check.error_code.as_deref(), Some("budget_paused"));

    assert_eq!(
        svc.approve_session(session.org_id, session.id, None)
            .await
            .unwrap(),
        1
    );
    let check = svc
        .check_budgets_for_session(session.org_id, &sid, None)
        .await;
    assert_eq!(check.action, "continue", "this session may continue");
    let check = svc
        .check_budgets_for_session(other.org_id, &other.id.to_string(), None)
        .await;
    assert_eq!(check.action, "pause", "other sessions are still asked");

    let event = Event::new(
        session.id,
        EventContext::empty(),
        generation("gpt-6.1-sol", usage(1000, 500, Some(5.0), None)),
    );
    svc.on_event(&event).await;
    let check = svc
        .check_budgets_for_session(session.org_id, &sid, None)
        .await;
    assert_eq!(check.action, "stop", "an approval never passes a stop step");
    let budget = db
        .get_budget(session.org_id, budget.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(budget.status, "exhausted");
}

#[tokio::test]
async fn notify_step_tells_owners_once_per_window() {
    let db = make_db();
    let svc = BudgetService::new(db.clone());
    let session = create_session_with_owner(&db, 1, None, None).await;
    let budget = policy(
        &db,
        session.org_id,
        "session",
        &session.id.to_string(),
        "usd",
        10.0,
        vec![step(50.0, PolicyStepAction::Notify)],
    )
    .await;
    for _ in 0..2 {
        let event = Event::new(
            session.id,
            EventContext::empty(),
            generation("gpt-6.1-sol", usage(1000, 500, Some(3.0), None)),
        );
        svc.on_event(&event).await;
    }
    let window = db
        .get_budget(session.org_id, budget.id)
        .await
        .unwrap()
        .unwrap()
        .created_at;
    assert!(
        !db.claim_usage_policy_notice(budget.id, 50.0, window)
            .await
            .unwrap(),
        "the step at 50% already fired for this window"
    );
}

#[tokio::test]
async fn org_timezone_round_trips() {
    let db = make_db();
    assert_eq!(db.org_timezone(1).await.unwrap(), None);
    db.set_org_timezone(1, Some("America/Chicago"))
        .await
        .unwrap();
    assert_eq!(
        db.org_timezone(1).await.unwrap().as_deref(),
        Some("America/Chicago")
    );
}

#[tokio::test]
async fn cost_summary_groups_by_where_the_cost_came_from() {
    let db = make_db();
    let svc = BudgetService::new(db.clone());
    let session = create_session_with_owner(&db, 1, None, None).await;
    for (actual, estimate) in [(Some(1.0), None), (None, Some(0.5)), (None, None)] {
        let event = Event::new(
            session.id,
            EventContext::empty(),
            generation("some/exotic-model", usage(100, 100, actual, estimate)),
        );
        svc.on_event(&event).await;
    }
    let now = chrono::Utc::now();
    let rows = db
        .cost_summary(
            session.org_id,
            now - chrono::Duration::hours(1),
            now + chrono::Duration::hours(1),
            "basis",
            "UTC",
        )
        .await
        .unwrap();
    let by = |basis: &str| rows.iter().find(|row| row.key.as_deref() == Some(basis));
    assert_eq!(by("provider").unwrap().amount_usd, 1.0);
    assert_eq!(by("list_price").unwrap().amount_usd, 0.5);
    let unpriced = by("unpriced").unwrap();
    assert_eq!((unpriced.amount_usd, unpriced.unpriced), (0.0, 1));
    assert!(
        db.cost_summary(session.org_id, now, now, "nope", "UTC")
            .await
            .is_err()
    );
}

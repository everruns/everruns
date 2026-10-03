// Retired budget subject levels (EVE-1129, EVE-1138).
//
// The `app` level is gone, converted onto the agent — a column rather than the
// `app:` tag it used to come from.
//
// `app_channel` is gone too, and it was the last subject resolved from a tag.
// Migration 138 had moved App webhooks onto `agent_triggers`, moved their
// budgets back from `agent_channel` to `app_channel`, and deleted the endpoint
// rows those budgets had been keyed on, leaving the tag as a webhook trigger's
// only identifier. Migration 153 adds `sessions.trigger_id` and re-keys those
// budgets onto the `agent_trigger` subject, so the resolver reads a column.
//
// All four App-era prefixes stay in `RESERVED_SESSION_TAG_PREFIXES` regardless
// of what reads them — that list is append-only.

use crate::domains::budgets::BudgetService;
use crate::storage::StorageBackend;
use crate::storage::models::*;
use everruns_contracts::typed_id::{PrincipalId, TriggerId};
use std::sync::Arc;

fn make_service() -> (BudgetService, Arc<StorageBackend>) {
    let db = Arc::new(StorageBackend::in_memory());
    let svc = BudgetService::new(db.clone());
    (svc, db)
}

async fn create_session(
    db: &Arc<StorageBackend>,
    org_id: i64,
    tags: Vec<String>,
    trigger_id: Option<uuid::Uuid>,
) -> SessionRow {
    db.create_session(CreateSessionRow {
        playground_user_id: None,
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id,
        app_id: None,
        channel_id: None,
        trigger_id,
        harness_id: None,
        agent_id: None,
        agent_version_id: None,
        agent_config_hash: None,
        virtual_user_id: None,
        owner_principal_id: PrincipalId::new(),
        resolved_owner_user_id: None,
        title: Some("Retired level budget test session".into()),
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

async fn seed_budget(db: &Arc<StorageBackend>, org_id: i64, subject_type: &str, subject_id: &str) {
    db.create_budget(CreateBudgetRow {
        org_id,
        subject_type: subject_type.into(),
        subject_id: subject_id.into(),
        currency: "usd".into(),
        limit: 5.0,
        soft_limit: None,
        period: None,
        metadata: None,
    })
    .await
    .unwrap();
}

async fn resolved_subject_types(svc: &BudgetService, session: &SessionRow) -> Vec<String> {
    svc.list_budgets_for_session_hierarchy(session.org_id, &session.id.to_string(), None)
        .await
        .iter()
        .map(|b| b.subject_type.clone())
        .collect()
}

/// No budget subject is resolved from a session tag. A session carrying every
/// App-era routing tag, with a budget seeded against each of them, resolves
/// none of them — including `app_channel`, which was the last one to go.
#[tokio::test]
async fn no_app_era_tag_contributes_a_budget_subject() {
    let (svc, db) = make_service();
    let session = create_session(
        &db,
        1,
        vec![
            "app:app_for_budget_test".to_string(),
            "app_channel:appchan_for_budget_test".to_string(),
            "slack:app:app_legacy".to_string(),
            "ag_ui:app:app_ag_ui".to_string(),
        ],
        None,
    )
    .await;

    for (subject_type, subject_id) in [
        ("app", "app_for_budget_test"),
        ("app", "app_legacy"),
        ("app", "app_ag_ui"),
        ("app_channel", "appchan_for_budget_test"),
    ] {
        seed_budget(&db, session.org_id, subject_type, subject_id).await;
    }

    let subjects = resolved_subject_types(&svc, &session).await;
    assert!(
        subjects.is_empty(),
        "no App-era tag may resolve a budget subject: {subjects:?}"
    );
}

/// The successor binds: a trigger's cap is selected by `sessions.trigger_id`,
/// keyed on the trigger's API id — the identifier an operator can actually put
/// in a budget (EVE-1136's rule, applied to the new level).
#[tokio::test]
async fn the_trigger_column_contributes_a_budget_subject() {
    let (svc, db) = make_service();
    let trigger_uuid = uuid::Uuid::now_v7();
    let session = create_session(&db, 1, vec![], Some(trigger_uuid)).await;

    seed_budget(
        &db,
        session.org_id,
        "agent_trigger",
        &TriggerId::from_uuid(trigger_uuid).to_string(),
    )
    .await;

    let subjects = resolved_subject_types(&svc, &session).await;
    assert_eq!(
        subjects,
        vec!["agent_trigger".to_string()],
        "the trigger budget must bind from the column"
    );
}

/// A trigger budget keyed on the internal uuid rather than the `trg_` id must
/// not bind. This is the exact shape of EVE-1136, where agent budgets were
/// keyed on `agents.id` instead of `agents.public_id` and silently never bound.
#[tokio::test]
async fn a_trigger_budget_keyed_on_the_raw_uuid_does_not_bind() {
    let (svc, db) = make_service();
    let trigger_uuid = uuid::Uuid::now_v7();
    let session = create_session(&db, 1, vec![], Some(trigger_uuid)).await;

    seed_budget(
        &db,
        session.org_id,
        "agent_trigger",
        &trigger_uuid.to_string(),
    )
    .await;

    let subjects = resolved_subject_types(&svc, &session).await;
    assert!(
        subjects.is_empty(),
        "only the `trg_` API id may key a trigger budget: {subjects:?}"
    );
}

/// EVE-1138's acceptance criterion: the cap still *stops* a trigger session
/// after the conversion. A row count would not have caught a subject that is
/// stored and listed but never evaluated — that is exactly how the agent level
/// was broken for as long as it was (EVE-1136).
#[tokio::test]
async fn an_exhausted_trigger_budget_stops_the_session() {
    let (svc, db) = make_service();
    let trigger_uuid = uuid::Uuid::now_v7();
    let session = create_session(&db, 1, vec![], Some(trigger_uuid)).await;
    let subject_id = TriggerId::from_uuid(trigger_uuid).to_string();

    let budget = db
        .create_budget(CreateBudgetRow {
            org_id: session.org_id,
            subject_type: "agent_trigger".into(),
            subject_id: subject_id.clone(),
            currency: "usd".into(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    db.set_budget_status(budget.id, "exhausted").await.unwrap();

    let result = svc
        .check_budgets_for_session(session.org_id, &session.id.to_string(), None)
        .await;
    assert!(
        result.should_stop(),
        "an exhausted trigger cap must refuse the turn, not merely appear in a list: {result:?}"
    );
}

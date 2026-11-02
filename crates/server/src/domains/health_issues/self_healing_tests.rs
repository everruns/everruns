use super::background_work::{RUNAWAY_WARN_RUNS, SCHEDULE_RUNAWAY, observe_runaway};
use super::self_healing::*;
use crate::domains::common::{Command, Ctx};
use crate::storage::{CreateSessionScheduleRow, SessionScheduleRow, StorageBackend};
use chrono::Utc;
use everruns_contracts::typed_id::PrincipalId;
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

#[test]
fn defaults_pause_schedules_and_keep_agent_changes_with_a_person() {
    let policy = SelfHealingPolicy::default();
    assert_eq!(policy.runaway_schedules, SelfHealingMode::Auto);
    assert_eq!(policy.agent_fixes, SelfHealingMode::Suggest);
    assert!(policy.validate().is_ok());
    // Omitted fields take the defaults.
    let parsed: SelfHealingPolicy = serde_json::from_value(json!({"agent_fixes": "off"})).unwrap();
    assert_eq!(parsed.runaway_schedules, SelfHealingMode::Auto);
    assert_eq!(parsed.agent_fixes, SelfHealingMode::Off);
}

#[test]
fn agent_changes_can_never_be_automatic() {
    let policy = SelfHealingPolicy {
        agent_fixes: SelfHealingMode::Auto,
        ..Default::default()
    };
    assert!(policy.validate().unwrap_err().contains("person approves"));
}

async fn schedule(db: &Arc<StorageBackend>) -> SessionScheduleRow {
    let agent = db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await;
    let session = db.create_test_session().await;
    sqlx::query("UPDATE sessions SET agent_id = $1 WHERE id = $2")
        .bind(agent)
        .bind(session.uuid())
        .execute(db.database().pool())
        .await
        .unwrap();
    db.create_session_schedule(CreateSessionScheduleRow {
        org_id: DEFAULT_ORG_ID,
        session_id: session,
        owner_principal_id: PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        description: "Poll the claim".to_string(),
        scheduled_at: None,
        cron_expression: Some("*/5 * * * *".to_string()),
        timezone: "UTC".to_string(),
        next_trigger_at: Some(Utc::now()),
        ends_at: None,
        max_runs: Some(96),
        stop_when: None,
    })
    .await
    .unwrap()
}

fn ctx(db: &Arc<StorageBackend>) -> Ctx {
    Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db.clone(), None)
}

async fn enabled(db: &StorageBackend, row: &SessionScheduleRow) -> (bool, Option<String>) {
    let row = db
        .get_session_schedule(DEFAULT_ORG_ID, row.id)
        .await
        .unwrap()
        .unwrap();
    (row.enabled, row.stop_reason)
}

async fn runaway_issue(db: &StorageBackend, row: &SessionScheduleRow) -> Uuid {
    db.open_subject_health_issues(SCHEDULE_RUNAWAY)
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.subject_id.as_deref() == Some(row.public_id.as_str()))
        .expect("the runaway finding is open")
        .id
}

async fn set_policy(db: &StorageBackend, value: serde_json::Value) {
    db.set_org_self_healing(DEFAULT_ORG_ID, &value)
        .await
        .unwrap();
}

#[tokio::test]
async fn by_default_a_repeating_schedule_is_paused_at_the_warning_and_can_be_resumed() {
    let db = Arc::new(StorageBackend::test_database());
    set_policy(&db, json!({})).await;
    let row = schedule(&db).await;

    assert!(pause_runaway(&db, &row, RUNAWAY_WARN_RUNS).await);
    assert_eq!(enabled(&db, &row).await, (false, Some("runaway".into())));
    let issue_id = runaway_issue(&db, &row).await;

    let ctx = ctx(&db);
    let listed = ListFindingActions { issue_id }.run(&ctx).await.unwrap();
    assert_eq!(listed.mode, Some(SelfHealingMode::Auto));
    assert_eq!(listed.actions.len(), 1);
    let action = &listed.actions[0];
    assert!(action.automatic && action.current && action.undone_at.is_none());
    assert_eq!(action.target_id, row.public_id);
    // Already paused: nothing more to offer.
    assert!(listed.available.is_empty());

    let undone = UndoFindingAction {
        issue_id,
        action_id: action.id.clone(),
    }
    .run(&ctx)
    .await
    .unwrap();
    assert!(undone.actions[0].undone_at.is_some());
    assert_eq!(enabled(&db, &row).await, (true, None));
    assert_eq!(
        db.schedule_same_outcome_runs(row.id.uuid()).await.unwrap(),
        0,
        "resuming starts the repeated-outcome count over"
    );

    // Undo happens once.
    let again = UndoFindingAction {
        issue_id,
        action_id: action.id.clone(),
    }
    .run(&ctx)
    .await;
    assert!(again.is_err());
}

#[tokio::test]
async fn in_suggest_mode_the_schedule_keeps_running_and_a_person_can_pause_it() {
    let db = Arc::new(StorageBackend::test_database());
    set_policy(&db, json!({"runaway_schedules": "suggest"})).await;
    let row = schedule(&db).await;

    assert!(!pause_runaway(&db, &row, RUNAWAY_WARN_RUNS).await);
    assert!(enabled(&db, &row).await.0);
    // The scheduler still warns and opens the finding.
    observe_runaway(&db, &row, RUNAWAY_WARN_RUNS, false).await;
    let issue_id = runaway_issue(&db, &row).await;

    let ctx = ctx(&db);
    let listed = ListFindingActions { issue_id }.run(&ctx).await.unwrap();
    assert!(listed.actions.is_empty());
    assert_eq!(listed.available.len(), 1);
    assert_eq!(listed.available[0].kind, PAUSE_SCHEDULE);

    let taken = TakeFindingAction {
        issue_id,
        req: TakeFindingActionRequest {
            kind: PAUSE_SCHEDULE.into(),
        },
    }
    .run(&ctx)
    .await
    .unwrap();
    assert_eq!(taken.actions.len(), 1);
    assert!(!taken.actions[0].automatic, "a person took it");
    assert!(taken.available.is_empty());
    assert_eq!(enabled(&db, &row).await, (false, Some("runaway".into())));
}

#[tokio::test]
async fn off_offers_nothing_and_leaves_the_hard_stop_to_the_scheduler() {
    let db = Arc::new(StorageBackend::test_database());
    set_policy(&db, json!({"runaway_schedules": "off"})).await;
    let row = schedule(&db).await;

    assert!(!pause_runaway(&db, &row, RUNAWAY_WARN_RUNS).await);
    observe_runaway(&db, &row, RUNAWAY_WARN_RUNS, false).await;
    let issue_id = runaway_issue(&db, &row).await;
    let ctx = ctx(&db);
    let listed = ListFindingActions { issue_id }.run(&ctx).await.unwrap();
    assert_eq!(listed.mode, Some(SelfHealingMode::Off));
    assert!(listed.available.is_empty());
    let refused = TakeFindingAction {
        issue_id,
        req: TakeFindingActionRequest {
            kind: PAUSE_SCHEDULE.into(),
        },
    }
    .run(&ctx)
    .await;
    assert!(refused.is_err());
    assert!(enabled(&db, &row).await.0);
}

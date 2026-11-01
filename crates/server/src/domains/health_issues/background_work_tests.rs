use super::background_work::*;
use crate::domains::session_schedules::{ScheduleChange, SessionScheduleService};
use crate::storage::{CreateSessionScheduleRow, SessionScheduleRow, StorageBackend};
use chrono::Utc;
use everruns_contracts::typed_id::PrincipalId;
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::session_schedule::stop_reason;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

#[test]
fn answers_that_differ_only_in_numbers_case_and_spacing_match() {
    let a = outcome_fingerprint(
        &json!([{"type": "text", "text": "Still pending: expired_token at 10:05."}]),
    );
    let b = outcome_fingerprint(&json!("still  pending: EXPIRED_TOKEN at 11:40."));
    let other = outcome_fingerprint(&json!("Approved. Assertion saved."));
    assert!(a.is_some());
    assert_eq!(a, b);
    assert_ne!(a, other);
    assert_eq!(outcome_fingerprint(&json!([{"type": "image"}])), None);
    assert_eq!(outcome_fingerprint(&json!("   ")), None);
}

#[test]
fn the_warning_says_how_many_runs_are_left_and_how_to_stop() {
    let text = runaway_warning(30, Some(Utc::now() + chrono::Duration::minutes(5)));
    assert!(text.contains("last 30 runs"), "{text}");
    assert!(text.contains("If 18 more do"), "{text}");
    assert!(text.contains("cancel_schedule"), "{text}");
    assert!(text.contains("longer interval"), "{text}");
}

struct Fixture {
    db: Arc<StorageBackend>,
    schedule: SessionScheduleRow,
    owner: Uuid,
    author: Uuid,
}

async fn fixture() -> Fixture {
    let db = Arc::new(StorageBackend::test_database());
    let owner = db.create_test_user(Uuid::now_v7()).await;
    let author = db.create_test_user(Uuid::now_v7()).await;
    let pool = db.database().pool();
    for user in [owner, author] {
        sqlx::query(
            "INSERT INTO organization_members (org_id, user_id, role) VALUES ($1, $2, 'member')",
        )
        .bind(DEFAULT_ORG_ID)
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    }
    let agent = db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await;
    sqlx::query(
        "INSERT INTO entity_changes (id, org_id, entity_kind, entity_ref, command, action,
            actor_kind, actor_user_id, surface)
         SELECT $1, $2, 'agent', public_id, 'update_agent', 'update', 'user', $3, 'ui'
         FROM agents WHERE id = $4",
    )
    .bind(Uuid::now_v7())
    .bind(DEFAULT_ORG_ID)
    .bind(author)
    .bind(agent)
    .execute(pool)
    .await
    .unwrap();
    let session = db.create_test_session().await;
    sqlx::query("UPDATE sessions SET agent_id = $1 WHERE id = $2")
        .bind(agent)
        .bind(session.uuid())
        .execute(pool)
        .await
        .unwrap();
    let schedule = db
        .create_session_schedule(CreateSessionScheduleRow {
            org_id: DEFAULT_ORG_ID,
            session_id: session,
            owner_principal_id: PrincipalId::from_seed(1),
            resolved_owner_user_id: Some(owner),
            description: "Poll the claim".to_string(),
            scheduled_at: None,
            cron_expression: Some("*/5 * * * *".to_string()),
            timezone: "UTC".to_string(),
            next_trigger_at: Some(Utc::now()),
            ends_at: None,
            max_runs: Some(96),
            stop_when: Some("the claim is approved".to_string()),
        })
        .await
        .unwrap();
    Fixture {
        db,
        schedule,
        owner,
        author,
    }
}

/// A fire of the schedule whose turn answered `answer`.
async fn finished_fire(f: &Fixture, sequence: i32, answer: &str) {
    let input = Uuid::now_v7();
    let pool = f.db.database().pool();
    sqlx::query(
        "INSERT INTO runtime_invocations (input_message_id, org_id, session_id, schedule_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(input)
    .bind(DEFAULT_ORG_ID)
    .bind(f.schedule.session_id.uuid())
    .bind(f.schedule.id.uuid())
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO events (session_id, sequence, event_type, data, context)
         VALUES ($1, $2, 'output.message.completed', $3, $4)",
    )
    .bind(f.schedule.session_id.uuid())
    .bind(sequence)
    .bind(json!({"message": {"content": [{"type": "text", "text": answer}]}}))
    .bind(json!({"input_message_id": input.to_string()}))
    .execute(pool)
    .await
    .unwrap();
}

async fn notified(f: &Fixture, user: Uuid) -> Vec<(String, bool)> {
    sqlx::query_as::<_, (String, bool)>(
        "SELECT title, viewed_at IS NULL FROM notifications
         WHERE org_id = $1 AND user_id = $2 AND kind = 'health.issue'",
    )
    .bind(DEFAULT_ORG_ID)
    .bind(user)
    .fetch_all(f.db.database().pool())
    .await
    .unwrap()
}

#[tokio::test]
async fn identical_outcomes_are_counted_once_each_and_a_different_one_resets() {
    let f = fixture().await;
    finished_fire(&f, 1, "Still pending at 10:00").await;
    assert_eq!(count_previous_outcome(&f.db, &f.schedule).await, 1);
    // The same fire seen again (a deferred fire) is not counted twice.
    assert_eq!(count_previous_outcome(&f.db, &f.schedule).await, 1);
    finished_fire(&f, 2, "Still pending at 10:05").await;
    assert_eq!(count_previous_outcome(&f.db, &f.schedule).await, 2);
    finished_fire(&f, 3, "Approved").await;
    assert_eq!(count_previous_outcome(&f.db, &f.schedule).await, 1);
}

#[tokio::test]
async fn a_runaway_schedule_tells_its_owner_and_authors_and_resolves_when_resumed() {
    let f = fixture().await;
    observe_runaway(&f.db, &f.schedule, RUNAWAY_WARN_RUNS, false).await;
    for user in [f.owner, f.author] {
        assert_eq!(
            notified(&f, user).await,
            vec![("Schedule keeps repeating".to_string(), true)]
        );
    }

    // People read it; then the schedule is stopped, which shows it again.
    sqlx::query("UPDATE notifications SET viewed_at = NOW() WHERE kind = 'health.issue'")
        .execute(f.db.database().pool())
        .await
        .unwrap();
    let service = SessionScheduleService::new(f.db.clone());
    service
        .stop(DEFAULT_ORG_ID, f.schedule.id, stop_reason::RUNAWAY)
        .await
        .unwrap();
    observe_runaway(&f.db, &f.schedule, RUNAWAY_STOP_RUNS, true).await;
    assert_eq!(
        notified(&f, f.owner).await,
        vec![("Schedule stopped: it kept repeating".to_string(), true)]
    );
    let issue =
        f.db.open_subject_health_issues(SCHEDULE_RUNAWAY)
            .await
            .unwrap()
            .into_iter()
            .find(|row| row.subject_id.as_deref() == Some(f.schedule.public_id.as_str()))
            .expect("the runaway issue is open");
    assert!(issue.agent_name.is_some());

    // A person resumes it: the count starts over and the issue resolves.
    service
        .update(
            DEFAULT_ORG_ID,
            f.schedule.id,
            ScheduleChange {
                enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        f.db.schedule_same_outcome_runs(f.schedule.id.uuid())
            .await
            .unwrap(),
        0
    );
    recheck(&f.db, &issue).await.unwrap();
    assert!(
        f.db.open_subject_health_issues(SCHEDULE_RUNAWAY)
            .await
            .unwrap()
            .iter()
            .all(|row| row.subject_id.as_deref() != Some(f.schedule.public_id.as_str()))
    );
}

#[tokio::test]
async fn a_spent_org_budget_tells_only_owners() {
    let f = fixture().await;
    let owner = f.db.create_test_user(Uuid::now_v7()).await;
    sqlx::query(
        "INSERT INTO organization_members (org_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(DEFAULT_ORG_ID)
    .bind(owner)
    .execute(f.db.database().pool())
    .await
    .unwrap();
    observe_org_unattended_spent(&f.db, DEFAULT_ORG_ID, 20.0).await;
    assert_eq!(
        notified(&f, owner).await,
        vec![("Daily unattended spend limit reached".to_string(), true)]
    );
    assert!(notified(&f, f.author).await.is_empty());
    // Seen again while still spent: nobody is told twice.
    observe_org_unattended_spent(&f.db, DEFAULT_ORG_ID, 20.0).await;
    assert_eq!(notified(&f, owner).await.len(), 1);
}

fn issue_row(code: &str) -> crate::storage::HealthIssueRow {
    crate::storage::HealthIssueRow {
        id: Uuid::now_v7(),
        org_id: DEFAULT_ORG_ID,
        channel_id: None,
        channel_public_id: None,
        agent_public_id: None,
        agent_name: None,
        code: code.to_string(),
        episode_id: Uuid::now_v7(),
        status: "open".to_string(),
        missing_scopes: Vec::new(),
        error_code: None,
        channel_revision: None,
        first_detected_at: Utc::now(),
        last_checked_at: Utc::now(),
        subject_id: None,
        session_id: None,
        evidence: None,
    }
}

#[tokio::test]
async fn a_session_resending_a_large_uncached_history_is_reported_to_the_agents_authors() {
    let f = fixture().await;
    for _ in 0..20 {
        sqlx::query(
            "INSERT INTO llm_generations (org_id, session_id, model, input_tokens, cache_read_tokens)
             VALUES ($1, $2, 'muse', 380000, 1000)",
        )
        .bind(DEFAULT_ORG_ID)
        .bind(f.schedule.session_id.uuid())
        .execute(f.db.database().pool())
        .await
        .unwrap();
    }
    recheck(&f.db, &issue_row(CONTEXT_BLOAT)).await.unwrap();
    let open =
        f.db.open_subject_health_issues(CONTEXT_BLOAT)
            .await
            .unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(
        open[0].subject_id.as_deref(),
        Some(f.schedule.session_id.to_string().as_str())
    );
    assert_eq!(
        notified(&f, f.author).await,
        vec![("Session sends a large, uncached history".to_string(), true)]
    );
    // The schedule's owner is not an author of the agent.
    assert!(notified(&f, f.owner).await.is_empty());

    // The spend and budget sweeps run on the same data without finding anything.
    recheck(&f.db, &issue_row(SPEND_SPIKE)).await.unwrap();
    recheck(&f.db, &issue_row(ORG_UNATTENDED_BUDGET))
        .await
        .unwrap();
    assert!(
        f.db.open_subject_health_issues(SPEND_SPIKE)
            .await
            .unwrap()
            .is_empty()
    );
}

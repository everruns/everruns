use super::background_work::*;
use crate::domains::session_schedules::{ScheduleChange, SessionScheduleService};
use crate::storage::{CreateSessionScheduleRow, SessionScheduleRow, StorageBackend};
use chrono::Utc;
use everruns_contracts::typed_id::{PrincipalId, SessionId};
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
    agent: Uuid,
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
        agent,
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

async fn spend_charge(f: &Fixture, amount: f64, days_ago: i32) -> Uuid {
    spend_charge_at(
        f,
        amount,
        Utc::now() - chrono::Duration::days(i64::from(days_ago)),
    )
    .await
}

async fn spend_charge_at(f: &Fixture, amount: f64, at: chrono::DateTime<Utc>) -> Uuid {
    let session = f.db.create_test_session().await;
    let agent = f.agent;
    sqlx::query("UPDATE sessions SET agent_id = $1 WHERE id = $2")
        .bind(agent)
        .bind(session.uuid())
        .execute(f.db.database().pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO budgets (org_id, subject_type, subject_id, currency, \"limit\", balance)
         VALUES ($1, 'org_unattended', $2, 'USD', 100, 100) ON CONFLICT DO NOTHING",
    )
    .bind(DEFAULT_ORG_ID)
    .bind(DEFAULT_ORG_ID.to_string())
    .execute(f.db.database().pool())
    .await
    .unwrap();
    let budget: Uuid = sqlx::query_scalar(
        "SELECT id FROM budgets WHERE org_id = $1 AND subject_type = 'org_unattended'",
    )
    .bind(DEFAULT_ORG_ID)
    .fetch_one(f.db.database().pool())
    .await
    .unwrap();
    let journal: Uuid = sqlx::query_scalar(
        "INSERT INTO usage_journal (org_id, kind) VALUES ($1, 'usage') RETURNING id",
    )
    .bind(DEFAULT_ORG_ID)
    .fetch_one(f.db.database().pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO usage_ledger (org_id, journal_id, budget_id, session_id, agent_id,
            currency, amount, meter_source, created_at)
         VALUES ($1, $2, $3, $4, $5, 'USD', $6, 'llm', $7)",
    )
    .bind(DEFAULT_ORG_ID)
    .bind(journal)
    .bind(budget)
    .bind(session.uuid())
    .bind(agent)
    .bind(amount)
    .bind(at)
    .execute(f.db.database().pool())
    .await
    .unwrap();
    session.uuid()
}

#[tokio::test]
async fn spend_alert_explains_same_window_and_dismissal_waits_for_a_new_spike() {
    let f = fixture().await;
    // Six dollars over the preceding six days means a $1/day baseline.
    spend_charge(&f, 6.0, 2).await;
    let top = spend_charge(&f, 8.0, 0).await;
    for _ in 0..6 {
        spend_charge(&f, 1.0, 0).await;
    }
    spend_charge(&f, 100.0, -1).await; // Future entries are not observed spend.
    sweep_spend_spikes(&f.db).await.unwrap();
    let row =
        f.db.open_subject_health_issues(SPEND_SPIKE)
            .await
            .unwrap()
            .remove(0);
    let evidence = row.evidence.as_ref().unwrap();
    assert_eq!(evidence["last_day_usd"], 14.0);
    assert_eq!(evidence["baseline_daily_usd"], 1.0);
    let sessions = evidence["top_sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 5);
    assert_eq!(
        sessions[0]["session_id"],
        SessionId::from_uuid(top).to_string()
    );
    assert_eq!(sessions[0]["amount_usd"], 8.0);
    assert!(
        f.db.unattended_spend_sessions(DEFAULT_ORG_ID + 1, f.agent, Utc::now())
            .await
            .unwrap()
            .is_empty()
    );
    f.db.dismiss_finding(
        row.id,
        row.episode_id,
        DEFAULT_ORG_ID,
        Some(f.author),
        "Expected audit",
    )
    .await
    .unwrap();
    resolve(&f.db, &row).await.unwrap();
    sweep_spend_spikes(&f.db).await.unwrap();
    assert!(
        f.db.open_subject_health_issues(SPEND_SPIKE)
            .await
            .unwrap()
            .is_empty()
    );
    let kept =
        f.db.finding_investigation(row.id, row.episode_id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(kept.dismiss_reason.as_deref(), Some("Expected audit"));
    // A genuinely healthy observation rearms the detector, without deleting
    // the acknowledgment or claiming the old episode was never dismissed.
    let recovered_at = Utc::now() + chrono::Duration::days(8);
    sweep_spend_spikes_at(&f.db, recovered_at).await.unwrap();
    spend_charge_at(&f, 10.0, recovered_at).await;
    sweep_spend_spikes_at(&f.db, recovered_at + chrono::Duration::seconds(1))
        .await
        .unwrap();
    let next =
        f.db.open_subject_health_issues(SPEND_SPIKE)
            .await
            .unwrap()
            .remove(0);
    assert_eq!(next.id, row.id);
    assert_ne!(next.episode_id, row.episode_id);
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

#[tokio::test]
async fn an_observer_under_its_alert_opens_an_issue_for_the_agents_authors() {
    use crate::domains::health_issues::quality::OBSERVER_PASS_RATE_LOW;
    use crate::domains::observers::record::{
        ObserverAlert, ObserverMatch, ObserverScope, ObserverScorerConfig, ScorerMethod,
    };
    let f = fixture().await;
    let session =
        f.db.get_session_unscoped(f.schedule.session_id)
            .await
            .unwrap()
            .unwrap();
    let agent = session.agent_id.unwrap();
    let scorer = ObserverScorerConfig {
        key: "grounded".into(),
        scope: ObserverScope::Turn,
        tools: None,
        method: ScorerMethod::Rule {
            rule: crate::domains::evals::record::Scorer::Contains {
                text: "source".into(),
                weight: 1.0,
            },
        },
    };
    let alert = ObserverAlert {
        min_pass_rate: 0.8,
        window_hours: 24,
        min_scored: 4,
    };
    let observer =
        f.db.create_observer(
            DEFAULT_ORG_ID,
            crate::storage::CreateObserverRow {
                system_kind: None,
                public_id: everruns_contracts::typed_id::ObserverId::from_uuid(Uuid::now_v7())
                    .to_string(),
                name: "Support quality".into(),
                description: None,
                match_config: serde_json::to_value(ObserverMatch {
                    agent_ids: Some(vec![agent]),
                    ..Default::default()
                })
                .unwrap(),
                sampling_rate: 1.0,
                scorers: serde_json::to_value(vec![scorer]).unwrap(),
                alert: Some(serde_json::to_value(&alert).unwrap()),
            },
        )
        .await
        .unwrap();
    let rows: Vec<_> = (0..4)
        .map(|i| crate::storage::CreateTraceScoreRow {
            evidence_ref: None,
            public_id: everruns_contracts::typed_id::TraceScoreId::from_uuid(Uuid::now_v7())
                .to_string(),
            observer_id: observer.id,
            scorer_key: "grounded".into(),
            session_id: session.id.uuid(),
            turn_id: format!("turn_{i}"),
            agent_id: Some(agent.uuid()),
            harness_id: None,
            scope: "turn".into(),
            not_before: None,
        })
        .collect();
    f.db.enqueue_trace_scores(DEFAULT_ORG_ID, &rows)
        .await
        .unwrap();
    let claimed = f.db.claim_trace_scores(10, 120, 3).await.unwrap();
    assert_eq!(claimed.len(), 4);
    // Two of four pass: 50%, under the 80% alert.
    for (i, score) in claimed.iter().enumerate() {
        f.db.complete_trace_score(
            score.id,
            crate::storage::CompleteTraceScoreRow {
                status: "completed".into(),
                pass: Some(i < 2),
                value: Some(if i < 2 { 1.0 } else { 0.0 }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    recheck(&f.db, &issue_row(OBSERVER_PASS_RATE_LOW))
        .await
        .unwrap();
    let open =
        f.db.open_subject_health_issues(OBSERVER_PASS_RATE_LOW)
            .await
            .unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(
        open[0].subject_id.as_deref(),
        Some(observer.public_id.as_str())
    );
    assert_eq!(
        notified(&f, f.author).await,
        vec![("Live quality dropped".to_string(), true)]
    );
    let (_, body) = copy(&open[0]);
    assert!(body.contains("50% (2 of 4 scores passed)"), "{body}");

    // Removing the alert resolves the issue on the next sweep.
    f.db.update_observer(
        DEFAULT_ORG_ID,
        observer.id,
        crate::storage::UpdateObserverRow {
            alert: Some(None),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    recheck(&f.db, &issue_row(OBSERVER_PASS_RATE_LOW))
        .await
        .unwrap();
    assert!(
        f.db.open_subject_health_issues(OBSERVER_PASS_RATE_LOW)
            .await
            .unwrap()
            .is_empty()
    );
}

use super::behavior::*;
use crate::storage::StorageBackend;
use everruns_core::DEFAULT_ORG_ID;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

#[test]
fn a_failing_call_is_a_loop_before_it_is_a_repeat() {
    assert_eq!(classify_tool_pattern(30, 9), Some(TOOL_ERROR_LOOP));
    assert_eq!(classify_tool_pattern(25, 0), Some(REPEATED_TOOL_CALLS));
    assert_eq!(classify_tool_pattern(10, 3), None);
}

#[test]
fn the_copy_names_the_tool_and_counts_never_the_target() {
    let evidence = json!({
        "tool": "web_fetch", "calls": 12, "failed": 12, "window_calls": 14,
        "target": "https://example.com/?token=secret",
    });
    let (title, body) = copy_for(TOOL_ERROR_LOOP, "open", &evidence, Some("Scout"));
    assert_eq!(title, "Tool keeps failing");
    assert!(
        body.contains("Scout made the same web_fetch call 12 times"),
        "{body}"
    );
    assert!(!body.contains("secret"), "{body}");
    let (title, _) = copy_for(SESSION_SPEND_SPIKE, "resolved", &evidence, None);
    assert_eq!(title, "Session spend back to normal");
    assert!(super::background_work::owns(REPEATED_TOOL_CALLS));
}

struct Fixture {
    db: Arc<StorageBackend>,
    author: Uuid,
    agent: Uuid,
}

async fn fixture() -> Fixture {
    let db = Arc::new(StorageBackend::test_database());
    let author = db.create_test_user(Uuid::now_v7()).await;
    let pool = db.database().pool();
    sqlx::query(
        "INSERT INTO organization_members (org_id, user_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(DEFAULT_ORG_ID)
    .bind(author)
    .execute(pool)
    .await
    .unwrap();
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
    Fixture { db, author, agent }
}

impl Fixture {
    async fn session(&self, source: &str) -> Uuid {
        let session = self.db.create_test_session().await.uuid();
        sqlx::query("UPDATE sessions SET agent_id = $1, source = $2 WHERE id = $3")
            .bind(self.agent)
            .bind(source)
            .bind(session)
            .execute(self.db.database().pool())
            .await
            .unwrap();
        session
    }

    /// One model call `hours_ago` hours back that cost `usd`.
    async fn generation(&self, session: Uuid, hours_ago: i64, usd: f64) {
        sqlx::query(
            "INSERT INTO llm_generations (org_id, session_id, model, input_tokens, estimated_cost_usd, created_at)
             VALUES ($1, $2, 'muse', 1000, $3, NOW() - make_interval(hours => $4::int))",
        )
        .bind(DEFAULT_ORG_ID)
        .bind(session)
        .bind(usd)
        .bind(hours_ago as i32)
        .execute(self.db.database().pool())
        .await
        .unwrap();
    }

    /// `count` calls of `tool` on `target` in one turn, ending with `status`.
    async fn tool_calls(&self, session: Uuid, count: i32, tool: &str, target: &str, status: &str) {
        let pool = self.db.database().pool();
        sqlx::query(
            "INSERT INTO session_trace_turns (session_id, turn_no, turn_id, start_sequence, started_at, status)
             VALUES ($1, 1, $2, 1, NOW(), 'running') ON CONFLICT DO NOTHING",
        )
        .bind(session)
        .bind(Uuid::now_v7())
        .execute(pool)
        .await
        .unwrap();
        for step in 0..count {
            sqlx::query(
                "INSERT INTO session_trace_steps (session_id, turn_no, step_no, kind, status, start_sequence, started_at, name, target)
                 VALUES ($1, 1, $2, 'tool', $3, $2, NOW(), $4, $5)",
            )
            .bind(session)
            .bind(step)
            .bind(status)
            .bind(tool)
            .bind(target)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    async fn open(&self, code: &str) -> Vec<String> {
        self.db
            .open_subject_health_issues(code)
            .await
            .unwrap()
            .into_iter()
            .filter_map(|row| row.subject_id)
            .collect()
    }

    async fn notified(&self) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT title FROM notifications WHERE org_id = $1 AND user_id = $2 AND kind = 'health.issue'",
        )
        .bind(DEFAULT_ORG_ID)
        .bind(self.author)
        .fetch_all(self.db.database().pool())
        .await
        .unwrap()
    }
}

fn subject(session: Uuid) -> String {
    everruns_contracts::typed_id::SessionId::from_uuid(session).to_string()
}

#[tokio::test]
async fn a_tool_call_that_keeps_failing_is_reported_and_resolves_when_it_stops() {
    let f = fixture().await;
    let stuck = f.session("api").await;
    f.generation(stuck, 0, 0.01).await;
    f.tool_calls(stuck, 12, "web_fetch", "https://example.com/down", "error")
        .await;
    // An eval probing the same failure is not reported.
    let eval = f.session("eval").await;
    f.generation(eval, 0, 0.01).await;
    f.tool_calls(eval, 12, "web_fetch", "https://example.com/down", "error")
        .await;

    sweep(&f.db).await.unwrap();
    assert_eq!(f.open(TOOL_ERROR_LOOP).await, vec![subject(stuck)]);
    assert_eq!(f.notified().await, vec!["Tool keeps failing".to_string()]);

    // The calls start working on different targets: back to normal.
    sqlx::query("UPDATE session_trace_steps SET status = 'ok', target = step_no::text WHERE session_id = $1")
        .bind(stuck)
        .execute(f.db.database().pool())
        .await
        .unwrap();
    sweep(&f.db).await.unwrap();
    assert!(f.open(TOOL_ERROR_LOOP).await.is_empty());
    assert!(f.open(REPEATED_TOOL_CALLS).await.is_empty());
}

#[tokio::test]
async fn a_session_far_above_its_agents_usual_hour_is_reported() {
    let f = fixture().await;
    // A week of ordinary sessions: about five cents an hour.
    let usual = f.session("api").await;
    for hour in 2..16 {
        f.generation(usual, hour, 0.05).await;
    }
    let spiking = f.session("api").await;
    f.generation(spiking, 0, 4.0).await;

    sweep(&f.db).await.unwrap();
    assert_eq!(f.open(SESSION_SPEND_SPIKE).await, vec![subject(spiking)]);
    assert_eq!(
        f.notified().await,
        vec!["Session spending far above normal".to_string()]
    );
}

#[tokio::test]
async fn an_agent_without_enough_history_is_not_judged() {
    let f = fixture().await;
    let usual = f.session("api").await;
    for hour in 2..5 {
        f.generation(usual, hour, 0.05).await;
    }
    let spiking = f.session("api").await;
    f.generation(spiking, 0, 4.0).await;
    sweep(&f.db).await.unwrap();
    assert!(f.open(SESSION_SPEND_SPIKE).await.is_empty());
}

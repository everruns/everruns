use super::*;
use crate::storage::{CreateEventRow, StorageBackend, test_database::test_session_row};
use everruns_contracts::typed_id::{AgentId, SessionId, TurnId};
use everruns_core::{Caller, DEFAULT_ORG_ID, Permission, PermissionResolver};
use serde_json::json;
use std::sync::Arc;

async fn session(db: &StorageBackend, agent: AgentId) -> SessionId {
    let mut input = test_session_row(DEFAULT_ORG_ID);
    input.agent_id = Some(agent);
    let row = db.create_session(input).await.unwrap();
    sqlx::query(
        "UPDATE sessions SET created_at = now() - interval '7 days', status = 'idle' WHERE id = $1",
    )
    .bind(row.id)
    .execute(db.pool())
    .await
    .unwrap();
    row.id
}

async fn event(
    db: &StorageBackend,
    session: SessionId,
    kind: &str,
    ts: DateTime<Utc>,
    error: &str,
) -> i32 {
    let turn = TurnId::new();
    db.create_event(CreateEventRow {
        session_id: session,
        event_type: kind.into(),
        ts,
        context: json!({"turn_id": turn}),
        data: json!({"turn_id": turn, "error": error}),
        metadata: None,
        tags: None,
    })
    .await
    .unwrap()
    .sequence
}

fn request(agent: AgentId, before: DateTime<Utc>) -> ListAgentFailures {
    ListAgentFailures {
        agent_id: agent.to_string(),
        after: Some(before - Duration::hours(24)),
        before: Some(before),
        offset: None,
        limit: None,
    }
}

#[tokio::test]
async fn history_keeps_old_recovered_and_archived_sessions_and_matches_activity() {
    let db = Arc::new(StorageBackend::test_database());
    crate::setup::org_init::initialize_org_harnesses(&db, DEFAULT_ORG_ID)
        .await
        .unwrap();
    let agent = AgentId::from_uuid(
        db.create_test_agent(DEFAULT_ORG_ID, uuid::Uuid::now_v7())
            .await,
    );
    let public_agent = AgentId::new();
    sqlx::query("UPDATE agents SET public_id = $2 WHERE id = $1")
        .bind(agent)
        .bind(public_agent.to_string())
        .execute(db.pool())
        .await
        .unwrap();
    let other_agent = AgentId::from_uuid(
        db.create_test_agent(DEFAULT_ORG_ID, uuid::Uuid::now_v7())
            .await,
    );
    let old = session(&db, agent).await;
    let recent = session(&db, agent).await;
    let irrelevant = session(&db, agent).await;
    let other = session(&db, other_agent).await;
    let now = Utc::now();
    event(
        &db,
        old,
        "turn.failed",
        now - Duration::hours(2),
        "earlier error",
    )
    .await;
    let sequence = event(
        &db,
        old,
        "turn.failed",
        now - Duration::minutes(30),
        "model timed out",
    )
    .await;
    event(&db, old, "turn.completed", now - Duration::minutes(20), "").await;
    sqlx::query("UPDATE sessions SET archived_at = now() WHERE id = $1")
        .bind(old)
        .execute(db.pool())
        .await
        .unwrap();
    event(&db, recent, "turn.failed", now, "latest error").await;
    // Failures outside the window, cancellations, and recoverable tool errors
    // must not turn a matching turn count into a different session population.
    event(
        &db,
        irrelevant,
        "turn.failed",
        now - Duration::hours(24),
        "lower boundary",
    )
    .await;
    event(
        &db,
        irrelevant,
        "turn.failed",
        now + Duration::seconds(1),
        "after snapshot",
    )
    .await;
    event(
        &db,
        irrelevant,
        "turn.cancelled",
        now - Duration::minutes(10),
        "stopped",
    )
    .await;
    event(
        &db,
        irrelevant,
        "tool.completed",
        now - Duration::minutes(5),
        "recoverable",
    )
    .await;
    event(&db, other, "turn.failed", now, "other agent").await;

    let ctx = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db.clone(), None);
    let result = request(public_agent, now).run(&ctx).await.unwrap();
    assert_eq!((result.failed_turns, result.total_sessions), (3, 2));
    assert_eq!(result.data[0].session.id, recent);
    let recovered = &result.data[1];
    assert_eq!(recovered.session.id, old);
    assert_eq!(recovered.session.agent_id, Some(public_agent));
    assert_eq!(recovered.failed_turns, 2);
    assert_eq!(recovered.failure_sequence, sequence);
    assert_eq!(recovered.error.as_deref(), Some("model timed out"));
    assert_eq!(
        recovered.session.last_turn_status.as_deref(),
        Some("completed")
    );
    assert!(recovered.session.archived_at.is_some());
    let activity = db.agent_activity(DEFAULT_ORG_ID, now).await.unwrap();
    assert_eq!(
        activity
            .run_buckets
            .iter()
            .filter(|b| b.agent_id == public_agent.to_string())
            .map(|b| b.failed)
            .sum::<i64>(),
        result.failed_turns
    );

    let mut paged = request(public_agent, now);
    paged.limit = Some(1);
    paged.offset = Some(1);
    let page = paged.run(&ctx).await.unwrap();
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].session.id, old);
    assert_eq!((page.failed_turns, page.total_sessions), (3, 2));
    let mut empty = request(public_agent, now);
    empty.offset = Some(100);
    let empty = empty.run(&ctx).await.unwrap();
    assert!(empty.data.is_empty());
    assert_eq!((empty.failed_turns, empty.total_sessions), (3, 2));

    let foreign = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID + 1), db.clone(), None);
    assert!(matches!(
        request(public_agent, now)
            .run(&foreign)
            .await
            .unwrap_err()
            .kind,
        CommandErrorKind::NotFound(_)
    ));
    let (rows, totals) = db
        .agent_failure_sessions(
            DEFAULT_ORG_ID + 1,
            agent,
            now - Duration::hours(24),
            now,
            0,
            20,
        )
        .await
        .unwrap();
    assert!(rows.is_empty());
    assert_eq!(totals.failed_turns, 0);
}

struct Without(Permission);
impl PermissionResolver for Without {
    fn has_permission(&self, _: &Caller, permission: &Permission) -> bool {
        permission != &self.0
    }
    fn caller_permissions(&self, caller: &Caller) -> Vec<Permission> {
        Permission::ALL
            .iter()
            .copied()
            .filter(|p| self.has_permission(caller, p))
            .collect()
    }
}

#[tokio::test]
async fn failure_history_requires_agent_and_session_permissions() {
    let db = Arc::new(StorageBackend::test_database());
    let mut caller = Caller::internal(DEFAULT_ORG_ID);
    caller.is_internal = false;
    for denied in [Permission::OrgAgentsManage, Permission::OrgSessionsManage] {
        let ctx = Ctx::minimal_for_test(caller.clone(), db.clone(), None)
            .with_permission_resolver(Arc::new(Without(denied)));
        assert!(matches!(
            request(AgentId::new(), Utc::now())
                .run(&ctx)
                .await
                .unwrap_err()
                .kind,
            CommandErrorKind::Forbidden(_)
        ));
    }
}

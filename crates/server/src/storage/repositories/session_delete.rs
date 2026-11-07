// PostgreSQL repository: Session deletion

use super::Database;
use anyhow::Result;
use everruns_contracts::typed_id::SessionId;
use tracing::warn;

/// Rows of a session in tables without a foreign key to `sessions`, deleted
/// with it. Trace steps before their turns.
const UNKEYED_SESSION_ROWS: [&str; 7] = [
    "DELETE FROM session_trace_steps WHERE session_id = ANY($1)",
    "DELETE FROM session_trace_turns WHERE session_id = ANY($1)",
    "DELETE FROM llm_generations WHERE session_id = ANY($1)",
    "DELETE FROM notification_turn_requests WHERE session_id = ANY($1)",
    "DELETE FROM runtime_invocations WHERE session_id = ANY($1)",
    "DELETE FROM session_actor_lease_history WHERE session_id = ANY($1)",
    "DELETE FROM events WHERE session_id = ANY($1)",
];

impl Database {
    /// Delete session by org and session id
    ///
    /// Runs in a transaction that sets `app.session_purge`, the flag the
    /// append-only guard on `events` recognises (migration 122). Without it the
    /// FK cascade into `events` trips the guard and the whole delete aborts —
    /// which is what made this endpoint answer 500 for any session that had
    /// taken a turn (EVE-919).
    pub async fn delete_session(&self, org_id: i64, id: SessionId) -> Result<bool> {
        // Defense in depth: the lock ordering below removes the known cycle
        // with event inserts, but a delete still races other writers (e.g. the
        // retention archiver, which locks events before event_sequences). A
        // deadlock aborts the whole transaction, so it is safe to retry.
        const ATTEMPTS: u32 = 3;
        let mut attempt = 1;
        loop {
            match self.delete_session_once(org_id, id).await {
                Err(e) if attempt < ATTEMPTS && is_deadlock(&e) => {
                    warn!(session_id = %id, attempt, "session delete deadlocked; retrying");
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    async fn delete_session_once(&self, org_id: i64, id: SessionId) -> Result<bool> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("SET LOCAL app.session_purge = 'true'")
            .execute(&mut *tx)
            .await?;

        // Lock order: event_sequences row before sessions row. An event insert
        // reserves its sequence on event_sequences first and only then, at the
        // end of its statement, takes a key-share lock on the sessions row for
        // the events FK. Deleting the session first and reaching
        // event_sequences through the FK cascade took the same two locks in the
        // opposite order and deadlocked against an in-flight insert. Waiting
        // here lets that insert commit first; an insert that arrives after
        // this lock fails its FK check once the session is gone.
        sqlx::query("SELECT 1 FROM event_sequences WHERE session_id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;

        // Sandboxes outlive their Session as history (migration 175): keep the
        // title for display and mark them deleted. The provider resource is
        // reclaimed by its lease cleanup.
        sqlx::query(
            r#"
            UPDATE sandboxes sb
            SET session_title = COALESCE(s.title, sb.session_title),
                agent_id = COALESCE(s.agent_id, sb.agent_id),
                desired_state = 'deleted', observed_state = 'deleted',
                current_instance_id = NULL, updated_at = NOW()
            FROM sessions s
            WHERE s.id = sb.session_id AND s.org_id = $1 AND s.id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            UPDATE sandbox_instances i SET retired_at = NOW(), updated_at = NOW()
            FROM sandboxes sb
            WHERE i.sandbox_id = sb.id AND sb.session_id = $1 AND i.retired_at IS NULL
            "#,
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;

        // These tables keep no key on their session (migration
        // 20261107040229_drop_hot_path_foreign_keys), so their rows go here,
        // for the session and every subagent session the delete cascades to.
        let doomed: Vec<uuid::Uuid> = sqlx::query_scalar(
            r#"
            WITH RECURSIVE doomed(id) AS (
                SELECT id FROM sessions WHERE org_id = $1 AND id = $2
                UNION
                SELECT s.id FROM sessions s
                JOIN doomed d ON s.parent_session_id = d.id OR s.root_session_id = d.id
            )
            SELECT id FROM doomed
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        if !doomed.is_empty() {
            for statement in UNKEYED_SESSION_ROWS {
                sqlx::query(statement)
                    .bind(&doomed)
                    .execute(&mut *tx)
                    .await?;
            }
        }

        let result = sqlx::query(
            r#"
            DELETE FROM sessions
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(result.rows_affected() > 0)
    }
}

/// SQLSTATE 40P01: PostgreSQL aborted the transaction to break a deadlock.
fn is_deadlock(err: &anyhow::Error) -> bool {
    err.downcast_ref::<sqlx::Error>()
        .and_then(sqlx::Error::as_database_error)
        .and_then(|db| db.code())
        .is_some_and(|code| code == "40P01")
}

#[cfg(test)]
mod tests {
    use crate::storage::test_database::test_session_row;
    use crate::storage::{CreateEventRow, StorageBackend};
    use everruns_contracts::typed_id::SessionId;
    use everruns_core::DEFAULT_ORG_ID;

    async fn count(db: &StorageBackend, sql: &'static str, ids: &[uuid::Uuid]) -> i64 {
        sqlx::query_scalar(sql)
            .bind(ids)
            .fetch_one(db.pool())
            .await
            .unwrap()
    }

    fn turn(session: SessionId, event_type: &str) -> CreateEventRow {
        CreateEventRow {
            session_id: session,
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context: serde_json::json!({ "turn_id": uuid::Uuid::now_v7().to_string() }),
            data: serde_json::json!({}),
            metadata: None,
            tags: None,
        }
    }

    /// Events and trace rows keep no key on their session, so deleting a
    /// session removes them itself, its subagent sessions' rows included.
    #[tokio::test]
    async fn deleting_a_session_removes_its_unkeyed_rows_and_its_subagents() {
        let db = StorageBackend::test_database();
        let parent = db.create_test_session().await;
        let child = db
            .create_session(crate::storage::CreateSessionRow {
                parent_session_id: Some(parent),
                ..test_session_row(DEFAULT_ORG_ID)
            })
            .await
            .unwrap()
            .id;
        let ids = [parent.uuid(), child.uuid()];
        for session in [parent, child] {
            db.create_events(vec![
                turn(session, "turn.started"),
                turn(session, "turn.completed"),
            ])
            .await
            .unwrap();
            db.database()
                .catch_up_session_trace_fully(session.uuid())
                .await
                .unwrap();
        }
        let events = "SELECT COUNT(*) FROM events WHERE session_id = ANY($1)";
        let turns = "SELECT COUNT(*) FROM session_trace_turns WHERE session_id = ANY($1)";
        assert_eq!(count(&db, events, &ids).await, 4);
        assert_eq!(count(&db, turns, &ids).await, 2);

        assert!(db.delete_session(DEFAULT_ORG_ID, parent).await.unwrap());

        assert_eq!(count(&db, events, &ids).await, 0);
        assert_eq!(count(&db, turns, &ids).await, 0);
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM sessions WHERE id = ANY($1)",
                &ids
            )
            .await,
            0
        );
    }
}

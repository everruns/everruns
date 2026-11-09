// PostgreSQL repository: Session deletion
//
// Decision: deleting a session is a soft delete. The session row, its events,
// trace, usage, files, secrets and every other row it owns stay in the
// database; `sessions.deleted_at` marks the session and its subagent sessions
// deleted, and the reads a person or an agent can reach leave marked rows out
// (`deleted_at IS NULL`). Records are permanent: only deleting a user or an
// organization erases sessions, through the foreign-key cascade.
//
// What stops with the session: its schedules are retired, its unfinished
// tasks are asked to cancel, its sandboxes are marked deleted, and a turn that
// is running or queued is cancelled. Events a worker still writes while the
// cancel lands are stored like any other; usage keeps counting.

use super::Database;
use anyhow::Result;
use everruns_contracts::typed_id::SessionId;
use tracing::warn;

impl Database {
    /// Mark a session, and every subagent session under it, deleted.
    ///
    /// `false` when the org has no such session or it is deleted already.
    pub async fn delete_session(&self, org_id: i64, id: SessionId) -> Result<bool> {
        // The marking transaction updates rows other writers also lock (the
        // session row, its sandboxes, schedules and tasks). A deadlock aborts
        // the whole transaction, so it is safe to retry.
        const ATTEMPTS: u32 = 3;
        let mut attempt = 1;
        let deleted = loop {
            match self.mark_session_deleted(org_id, id).await {
                Err(e) if attempt < ATTEMPTS && is_deadlock(&e) => {
                    warn!(session_id = %id, attempt, "session delete deadlocked; retrying");
                    attempt += 1;
                }
                result => break result?,
            }
        };

        // After the commit, so the cancel never holds the session rows' locks
        // together with an actor row's. A cancel that fails here leaves a turn
        // that runs to its end and is stored; no later turn starts, because
        // the turn context no longer loads for a deleted session.
        for session in &deleted {
            if let Err(error) = self
                .request_session_actor_cancel(SessionId::from_uuid(*session))
                .await
            {
                warn!(session_id = %session, %error, "could not cancel the deleted session's turn");
            }
        }

        Ok(!deleted.is_empty())
    }

    /// The sessions marked deleted: `id` and its subagent sessions.
    async fn mark_session_deleted(&self, org_id: i64, id: SessionId) -> Result<Vec<uuid::Uuid>> {
        let mut tx = self.pool.begin().await?;

        let deleted: Vec<uuid::Uuid> = sqlx::query_scalar(
            r#"
            WITH RECURSIVE family(id) AS (
                SELECT id FROM sessions
                WHERE org_id = $1 AND id = $2 AND deleted_at IS NULL
                UNION
                SELECT s.id FROM sessions s
                JOIN family f ON s.parent_session_id = f.id OR s.root_session_id = f.id
            )
            UPDATE sessions s SET deleted_at = NOW()
            FROM family f
            WHERE s.id = f.id AND s.deleted_at IS NULL
            RETURNING s.id
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        if deleted.is_empty() {
            return Ok(deleted);
        }

        // Sandboxes are history (migration 175): keep the title for display
        // and mark them deleted. The provider resource is reclaimed by its
        // lease cleanup.
        sqlx::query(
            r#"
            UPDATE sandboxes sb
            SET session_title = COALESCE(s.title, sb.session_title),
                agent_id = COALESCE(s.agent_id, sb.agent_id),
                desired_state = 'deleted', observed_state = 'deleted',
                current_instance_id = NULL, updated_at = NOW()
            FROM sessions s
            WHERE s.id = sb.session_id AND s.id = ANY($1)
            "#,
        )
        .bind(&deleted)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            UPDATE sandbox_instances i SET retired_at = NOW(), updated_at = NOW()
            FROM sandboxes sb
            WHERE i.sandbox_id = sb.id AND sb.session_id = ANY($1) AND i.retired_at IS NULL
            "#,
        )
        .bind(&deleted)
        .execute(&mut *tx)
        .await?;

        // Nothing wakes a deleted session: its schedules are retired the way
        // deleting one retires it, and unfinished tasks are asked to cancel.
        sqlx::query(
            r#"
            UPDATE session_schedules
            SET deleted_at = NOW(), enabled = false, next_trigger_at = NULL,
                claimed_by = NULL, claimed_at = NULL, updated_at = NOW()
            WHERE session_id = ANY($1) AND deleted_at IS NULL
            "#,
        )
        .bind(&deleted)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            UPDATE session_tasks
            SET cancel_requested_at = NOW(), updated_at = NOW()
            WHERE session_id = ANY($1)
              AND state IN ('queued', 'running', 'awaiting_input')
              AND cancel_requested_at IS NULL
            "#,
        )
        .bind(&deleted)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(deleted)
    }
}

#[cfg(test)]
impl Database {
    /// Erase a session row the way deleting its user or organization does:
    /// a real `DELETE`, cascading through the foreign keys. Tests use it for
    /// behaviour that depends on the row being gone, which an ordinary
    /// session delete no longer causes.
    pub(crate) async fn purge_session_for_test(&self, org_id: i64, id: SessionId) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL app.session_purge = 'true'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT 1 FROM event_sequences WHERE session_id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        let result = sqlx::query("DELETE FROM sessions WHERE org_id = $1 AND id = $2")
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

    async fn traced_family(db: &StorageBackend) -> (SessionId, [uuid::Uuid; 2]) {
        let parent = db.create_test_session().await;
        let child = db
            .create_session(crate::storage::CreateSessionRow {
                parent_session_id: Some(parent),
                ..test_session_row(DEFAULT_ORG_ID)
            })
            .await
            .unwrap()
            .id;
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
        (parent, [parent.uuid(), child.uuid()])
    }

    /// A real delete of the parent row, relying on the database's cascades
    /// and the cleanup trigger for the rest. The release before soft delete
    /// did this for every session delete; erasing an organization still does.
    async fn legacy_delete(db: &StorageBackend, parent: SessionId) {
        assert!(
            db.purge_session_for_test(DEFAULT_ORG_ID, parent)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn legacy_session_delete_cleans_unkeyed_rows_during_rolling_upgrade() {
        let db = StorageBackend::test_database();
        let (parent, ids) = traced_family(&db).await;
        let other = db.create_test_session().await;
        db.create_events(vec![turn(other, "turn.started")])
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM pg_constraint WHERE conname = 'events_session_id_fkey'"
            )
            .fetch_one(db.pool())
            .await
            .unwrap(),
            0,
            "keep the hot-path foreign key removed"
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM events WHERE session_id = ANY($1)",
                &ids
            )
            .await,
            4
        );

        legacy_delete(&db, parent).await;

        for (table, statement) in [
            (
                "sessions",
                "SELECT COUNT(*) FROM sessions WHERE id = ANY($1)",
            ),
            (
                "events",
                "SELECT COUNT(*) FROM events WHERE session_id = ANY($1)",
            ),
            (
                "session_trace_turns",
                "SELECT COUNT(*) FROM session_trace_turns WHERE session_id = ANY($1)",
            ),
            (
                "session_trace_steps",
                "SELECT COUNT(*) FROM session_trace_steps WHERE session_id = ANY($1)",
            ),
        ] {
            let remaining: i64 = sqlx::query_scalar(statement)
                .bind(ids.as_slice())
                .fetch_one(db.pool())
                .await
                .unwrap();
            assert_eq!(remaining, 0, "legacy delete left rows in {table}");
        }
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM events WHERE session_id = ANY($1)",
                &[other.uuid()]
            )
            .await,
            1
        );
    }

    #[tokio::test]
    async fn legacy_cleanup_keeps_the_append_only_guard() {
        let db = StorageBackend::test_database();
        let (parent, ids) = traced_family(&db).await;

        let error = sqlx::query("DELETE FROM sessions WHERE org_id = $1 AND id = $2")
            .bind(DEFAULT_ORG_ID)
            .bind(parent)
            .execute(db.pool())
            .await
            .expect_err("deleting events still requires the transaction purge flag");
        assert!(error.to_string().contains("append-only"), "{error}");
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM events WHERE session_id = ANY($1)",
                &ids
            )
            .await,
            4
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM sessions WHERE id = ANY($1)",
                &ids
            )
            .await,
            2
        );
    }

    #[tokio::test]
    async fn cleanup_prerequisite_applies_after_its_successor_without_rewriting_history() {
        const PREDECESSOR: i64 = 20261107040228;
        const SUCCESSOR: i64 = 20261107040229;
        let db = StorageBackend::test_database();
        let checksum: Vec<u8> = sqlx::query_scalar(
            "SELECT checksum FROM _sqlx_migrations WHERE version = $1 AND success",
        )
        .bind(SUCCESSOR)
        .fetch_one(db.pool())
        .await
        .unwrap();
        // This database is a private test copy. Recreate the already-upgraded
        // state that existed before the missing prerequisite was published.
        sqlx::query("DROP TRIGGER sessions_unkeyed_delete_compat ON sessions")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("DROP FUNCTION public.cleanup_session_rows_for_legacy_delete()")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version = $1")
            .bind(PREDECESSOR)
            .execute(db.pool())
            .await
            .unwrap();

        crate::setup::migrations::run(db.pool()).await.unwrap();

        let unchanged: Vec<u8> = sqlx::query_scalar(
            "SELECT checksum FROM _sqlx_migrations WHERE version = $1 AND success",
        )
        .bind(SUCCESSOR)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(unchanged, checksum);
        let applied: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM _sqlx_migrations WHERE version = $1 AND success",
        )
        .bind(PREDECESSOR)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(applied, 1);
        let (parent, ids) = traced_family(&db).await;
        legacy_delete(&db, parent).await;
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM events WHERE session_id = ANY($1)",
                &ids
            )
            .await,
            0
        );
    }

    /// Deleting keeps every row: the session, its subagent session, their
    /// events and trace. Both are marked deleted and stop being readable.
    #[tokio::test]
    async fn deleting_a_session_keeps_its_rows_and_hides_it_with_its_subagents() {
        let db = StorageBackend::test_database();
        let (parent, ids) = traced_family(&db).await;
        let child = SessionId::from_uuid(ids[1]);
        let other = db.create_test_session().await;
        let events = "SELECT COUNT(*) FROM events WHERE session_id = ANY($1)";
        let turns = "SELECT COUNT(*) FROM session_trace_turns WHERE session_id = ANY($1)";
        let marked = "SELECT COUNT(*) FROM sessions WHERE id = ANY($1) AND deleted_at IS NOT NULL";
        assert_eq!(count(&db, events, &ids).await, 4);
        assert_eq!(count(&db, turns, &ids).await, 2);

        assert!(db.delete_session(DEFAULT_ORG_ID, parent).await.unwrap());

        assert_eq!(count(&db, events, &ids).await, 4);
        assert_eq!(count(&db, turns, &ids).await, 2);
        assert_eq!(count(&db, marked, &ids).await, 2);
        assert_eq!(count(&db, marked, &[other.uuid()]).await, 0);
        for session in [parent, child] {
            assert!(
                db.get_session(DEFAULT_ORG_ID, session)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            db.get_session(DEFAULT_ORG_ID, other)
                .await
                .unwrap()
                .is_some()
        );
        // Deleting again finds nothing to delete.
        assert!(!db.delete_session(DEFAULT_ORG_ID, parent).await.unwrap());
        // Another organization cannot delete it.
        assert!(!db.delete_session(DEFAULT_ORG_ID + 1, other).await.unwrap());
    }

    /// A deleted session's schedules stop firing and its queued turn is
    /// withdrawn, so nothing wakes it.
    #[tokio::test]
    async fn deleting_a_session_stops_what_would_wake_it() {
        let db = StorageBackend::test_database();
        let session = db.create_test_session().await;
        db.mark_session_actor_runnable(session, DEFAULT_ORG_ID, 1)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO session_tasks (id, session_id, kind, state) \
             VALUES ('task_1', $1, 'test', 'running')",
        )
        .bind(session)
        .execute(db.pool())
        .await
        .unwrap();

        assert!(db.delete_session(DEFAULT_ORG_ID, session).await.unwrap());

        let actor = db.get_session_actor(session).await.unwrap().unwrap();
        assert!(actor.runnable_since.is_none());
        let asked: bool = sqlx::query_scalar(
            "SELECT cancel_requested_at IS NOT NULL FROM session_tasks WHERE session_id = $1",
        )
        .bind(session)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert!(asked);
    }
}

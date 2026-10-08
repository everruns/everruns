// PostgreSQL repository: Session deletion

use super::Database;
use anyhow::Result;
use everruns_contracts::typed_id::SessionId;
use tracing::warn;

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

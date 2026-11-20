// PostgreSQL repository: running v1 sessions moved onto their org's data
// plane (data plane v2, Phase C). See `domains::data_plane::moves` and the
// `dp_session_moves` migration.

use anyhow::Result;
use serde_json::Value;

use super::Database;

/// A session's move as it stands.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DpSessionMoveRow {
    pub session_id: String,
    pub org_id: i64,
    pub state: String,
    pub from_sequence: Option<i32>,
    pub report: Option<Value>,
}

/// Where a session's messages go while it moves: the summary that marks it
/// as the data plane's, with this status until the import is done.
pub const MOVING: &str = "moving";

impl Database {
    pub async fn dp_session_move(&self, session_id: &str) -> Result<Option<DpSessionMoveRow>> {
        Ok(sqlx::query_as(
            "SELECT session_id, org_id, state, from_sequence, report
               FROM dp_session_moves WHERE session_id = $1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Switch `session_id` to the data plane: record its move and mark it
    /// as the data plane's ([`MOVING`]) in one transaction. False when it
    /// already was (moved, moving, or created there).
    pub async fn dp_begin_session_move(
        &self,
        org_id: i64,
        session_id: &str,
        data_plane_id: &str,
        agent_key: &str,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let marked = sqlx::query(
            "INSERT INTO dp_session_summaries
                (session_id, org_id, data_plane_id, agent_id, session_version, title, status,
                 turns, cost_micros, created_at, last_activity_at)
             VALUES ($1, $2, $3, $4, 0, '', $5, 0, 0, now(), now())
             ON CONFLICT (session_id) DO NOTHING",
        )
        .bind(session_id)
        .bind(org_id)
        .bind(data_plane_id)
        .bind(agent_key)
        .bind(MOVING)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if marked == 0 {
            return Ok(false);
        }
        sqlx::query(
            "INSERT INTO dp_session_moves (session_id, org_id, state) VALUES ($1, $2, 'switched')",
        )
        .bind(session_id)
        .bind(org_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// The import of `session_id` is done: its messages are served from
    /// now on, and its move keeps `report`.
    pub async fn dp_finish_session_move(
        &self,
        session_id: &str,
        from_sequence: Option<i32>,
        report: &Value,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE dp_session_summaries SET status = 'started', updated_at = now()
              WHERE session_id = $1 AND status = $2",
        )
        .bind(session_id)
        .bind(MOVING)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE dp_session_moves
                SET state = 'done', from_sequence = $2, report = $3, finished_at = now()
              WHERE session_id = $1",
        )
        .bind(session_id)
        .bind(from_sequence)
        .bind(report)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Whether `session_id`'s messages are refused for now: it is moving.
    pub async fn dp_session_moving(&self, session_id: &str) -> Result<bool> {
        Ok(sqlx::query_scalar::<_, i32>(
            "SELECT 1 FROM dp_session_summaries WHERE session_id = $1 AND status = $2",
        )
        .bind(session_id)
        .bind(MOVING)
        .fetch_optional(&self.pool)
        .await?
        .is_some())
    }

    /// `org_id`'s sessions still on v1 (not deleted; archived ones only with
    /// `archived`), by id after `after`.
    pub async fn dp_v1_sessions(
        &self,
        org_id: i64,
        archived: bool,
        after: uuid::Uuid,
        limit: i64,
    ) -> Result<Vec<uuid::Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT s.id FROM sessions s
              WHERE s.org_id = $1 AND s.deleted_at IS NULL
                AND ($2 OR s.archived_at IS NULL)
                AND s.id > $3
                AND NOT EXISTS (
                    SELECT 1 FROM dp_session_summaries d
                     WHERE d.session_id = 'session_' || replace(s.id::text, '-', '')
                )
              ORDER BY s.id
              LIMIT $4",
        )
        .bind(org_id)
        .bind(archived)
        .bind(after)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }
}

use super::StorageBackend;
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::{AgentId, SessionId, TurnId};
use serde_json::Value;
use uuid::Uuid;

impl StorageBackend {
    pub(crate) async fn get_agent_view_settings(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> Result<Option<(i64, Option<Value>)>> {
        Ok(sqlx::query_as(
            "SELECT COALESCE(v.revision, 0), v.view
             FROM agents a
             LEFT JOIN agent_view_settings v ON v.agent_id = a.id
             WHERE a.org_id = $1 AND a.public_id = $2
             AND a.status <> 'deleted' AND NOT a.is_built_in",
        )
        .bind(org_id)
        .bind(agent_id.to_string())
        .fetch_optional(self.database().pool())
        .await?)
    }

    /// Lock the owning Agent so the write and an archive or delete have a defined order.
    pub(crate) async fn replace_agent_view_settings(
        &self,
        org_id: i64,
        agent_id: AgentId,
        expected_revision: i64,
        view: Option<Value>,
    ) -> Result<Option<i64>> {
        let mut tx = self.database().pool().begin().await?;
        let editable = sqlx::query_scalar::<_, bool>(
            "SELECT true FROM agents
             WHERE org_id = $1 AND id = $2 AND status = 'active' AND NOT is_built_in
             FOR UPDATE",
        )
        .bind(org_id)
        .bind(agent_id.uuid())
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(false);
        if !editable {
            return Ok(None);
        }

        let revision = sqlx::query_scalar::<_, i64>(
            "SELECT revision FROM agent_view_settings WHERE agent_id = $1",
        )
        .bind(agent_id.uuid())
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(0);
        if revision != expected_revision {
            return Ok(None);
        }
        if view.is_none() {
            for statement in [
                "DELETE FROM agent_view_snapshots WHERE agent_id = $1",
                "DELETE FROM agent_view_preferences WHERE agent_id = $1",
                "DELETE FROM agent_view_pending WHERE agent_id = $1",
            ] {
                sqlx::query(statement)
                    .bind(agent_id.uuid())
                    .execute(&mut *tx)
                    .await?;
            }
        }

        let revision = sqlx::query_scalar(
            "INSERT INTO agent_view_settings (agent_id, revision, view)
             VALUES ($1, 1, $2)
             ON CONFLICT (agent_id) DO UPDATE
             SET revision = agent_view_settings.revision + 1, view = EXCLUDED.view
             RETURNING revision",
        )
        .bind(agent_id.uuid())
        .bind(view)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(revision))
    }

    pub(crate) async fn has_agent_views(&self, org_id: i64) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (
              SELECT 1 FROM agent_view_settings v JOIN agents a ON a.id = v.agent_id
              WHERE a.org_id = $1 AND a.status = 'active' AND v.view IS NOT NULL
            )",
        )
        .bind(org_id)
        .fetch_one(self.database().pool())
        .await?)
    }

    pub(crate) async fn start_agent_view_turn(
        &self,
        session: SessionId,
        turn: TurnId,
        agent: AgentId,
        agent_scope: bool,
        started_at: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO agent_view_pending
                (session_id, turn_id, agent_id, settings_revision, agent_scope, started_at)
             SELECT s.id, $2, v.agent_id, v.revision, $4, $5
             FROM sessions s
             JOIN agents a ON a.id = s.agent_id AND a.org_id = s.org_id AND a.status = 'active'
             JOIN agent_view_settings v ON a.id = v.agent_id AND v.view IS NOT NULL
             WHERE s.id = $1 AND s.deleted_at IS NULL AND (a.id = $3 OR a.public_id = $6)
             ON CONFLICT DO NOTHING",
        )
        .bind(session.uuid())
        .bind(turn.uuid())
        .bind(agent.uuid())
        .bind(agent_scope)
        .bind(started_at)
        .bind(agent.to_string())
        .execute(self.database().pool())
        .await?;
        Ok(())
    }

    pub(crate) async fn stage_agent_view(
        &self,
        session: SessionId,
        turn: TurnId,
        sequence: i32,
        draft: Value,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE agent_view_pending SET draft = $4, draft_sequence = $3
             WHERE session_id = $1 AND turn_id = $2
             AND (draft_sequence IS NULL OR draft_sequence < $3)",
        )
        .bind(session.uuid())
        .bind(turn.uuid())
        .bind(sequence)
        .bind(draft)
        .execute(self.database().pool())
        .await?;
        Ok(())
    }

    pub(crate) async fn finish_agent_view_turn(
        &self,
        session: SessionId,
        turn: TurnId,
        publish: impl FnOnce(Value, Value) -> Option<Value>,
    ) -> Result<()> {
        let mut tx = self.database().pool().begin().await?;
        let pending: Option<(Uuid, i64, bool, DateTime<Utc>, Option<Value>)> = sqlx::query_as(
            "DELETE FROM agent_view_pending WHERE session_id = $1 AND turn_id = $2
             RETURNING agent_id, settings_revision, agent_scope, started_at, draft",
        )
        .bind(session.uuid())
        .bind(turn.uuid())
        .fetch_optional(&mut *tx)
        .await?;

        if let Some((agent, revision, agent_scope, started_at, Some(draft))) = pending {
            let settings: Option<Value> = sqlx::query_scalar(
                "SELECT view FROM agent_view_settings
                 WHERE agent_id = $1 AND revision = $2 AND view IS NOT NULL FOR SHARE",
            )
            .bind(agent)
            .bind(revision)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(publication) = settings.and_then(|settings| publish(settings, draft)) {
                sqlx::query(
                    "INSERT INTO agent_view_snapshots
                     (agent_id, session_id, settings_revision, started_at, turn_id, publication)
                     VALUES ($1, $2, $3, $4, $5, $6)
                     ON CONFLICT (agent_id, session_id) DO UPDATE
                     SET settings_revision = EXCLUDED.settings_revision,
                         started_at = EXCLUDED.started_at, turn_id = EXCLUDED.turn_id,
                         publication = EXCLUDED.publication, updated_at = now()
                     WHERE agent_view_snapshots.settings_revision < EXCLUDED.settings_revision
                        OR (agent_view_snapshots.started_at, agent_view_snapshots.turn_id)
                         < (EXCLUDED.started_at, EXCLUDED.turn_id)",
                )
                .bind(agent)
                .bind((!agent_scope).then(|| session.uuid()))
                .bind(revision)
                .bind(started_at)
                .bind(turn.uuid())
                .bind(publication)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn read_published_agent_view(
        &self,
        org_id: i64,
        agent: AgentId,
        session: Option<SessionId>,
        agent_scope: bool,
    ) -> Result<Option<(i64, Value, Option<Value>, Option<DateTime<Utc>>)>> {
        Ok(sqlx::query_as(
            "SELECT v.revision, v.view, snapshot.publication, snapshot.updated_at
             FROM agent_view_settings v
             JOIN agents a ON a.id = v.agent_id AND a.org_id = $1 AND a.status = 'active'
             LEFT JOIN LATERAL (
               SELECT publication, updated_at FROM agent_view_snapshots
               WHERE agent_id = a.id AND settings_revision = v.revision
               AND (session_id = $3 OR ($4 AND session_id IS NULL))
               ORDER BY session_id NULLS LAST LIMIT 1
             ) snapshot ON TRUE
             WHERE a.id = $2 AND v.view IS NOT NULL",
        )
        .bind(org_id)
        .bind(agent.uuid())
        .bind(session.map(|id| id.uuid()))
        .bind(agent_scope)
        .fetch_optional(self.database().pool())
        .await?)
    }

    pub(crate) async fn read_agent_view_preference(
        &self,
        agent: AgentId,
        audience: &str,
        viewer: Uuid,
        revision: i64,
    ) -> Result<Option<Value>> {
        Ok(sqlx::query_scalar::<_, Option<Value>>(
            "SELECT layout FROM agent_view_preferences
             WHERE agent_id = $1 AND audience = $2 AND viewer_id = $3 AND settings_revision = $4",
        )
        .bind(agent.uuid())
        .bind(audience)
        .bind(viewer)
        .bind(revision)
        .fetch_optional(self.database().pool())
        .await?
        .flatten())
    }

    pub(crate) async fn save_agent_view_preference(
        &self,
        agent: AgentId,
        audience: &str,
        viewer: Uuid,
        revision: i64,
        layout: Option<Value>,
    ) -> Result<bool> {
        Ok(sqlx::query(
            "INSERT INTO agent_view_preferences
             (agent_id, audience, viewer_id, settings_revision, layout)
             SELECT agent_id, $2, $3, revision, $5 FROM agent_view_settings
             WHERE agent_id = $1 AND revision = $4 AND view->'allow_personalization' = 'true'::jsonb
             ON CONFLICT (agent_id, audience, viewer_id) DO UPDATE
             SET settings_revision = EXCLUDED.settings_revision, layout = EXCLUDED.layout
             WHERE agent_view_preferences.settings_revision <= EXCLUDED.settings_revision",
        )
        .bind(agent.uuid())
        .bind(audience)
        .bind(viewer)
        .bind(revision)
        .bind(layout)
        .execute(self.database().pool())
        .await?
        .rows_affected()
            > 0)
    }
}

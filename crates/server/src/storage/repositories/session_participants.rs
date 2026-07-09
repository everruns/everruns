use super::super::models::*;
use super::Database;
use anyhow::Result;

impl Database {
    pub async fn create_session_participant(
        &self,
        input: CreateSessionParticipantRow,
    ) -> Result<SessionParticipantRow> {
        let joined_at = input.joined_at.unwrap_or_else(chrono::Utc::now);

        sqlx::query_as::<_, SessionParticipantRow>(
            r#"
            INSERT INTO session_participants (
                id, org_id, session_id, kind, agent_id, agent_version_id,
                principal_id, role, joined_at
            )
            VALUES (uuidv7(), $1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id, org_id, session_id, kind, agent_id, agent_version_id,
                      principal_id, role, joined_at, left_at, created_at, updated_at
            "#,
        )
        .bind(input.org_id)
        .bind(input.session_id.uuid())
        .bind(input.kind.to_string())
        .bind(input.agent_id.map(|id| id.uuid()))
        .bind(input.agent_version_id.map(|id| id.uuid()))
        .bind(input.principal_id)
        .bind(input.role.to_string())
        .bind(joined_at)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list_session_participants(
        &self,
        org_id: i64,
        session_id: everruns_core::SessionId,
    ) -> Result<Vec<SessionParticipantRow>> {
        sqlx::query_as::<_, SessionParticipantRow>(
            r#"
            SELECT id, org_id, session_id, kind, agent_id, agent_version_id,
                   principal_id, role, joined_at, left_at, created_at, updated_at
            FROM session_participants
            WHERE org_id = $1 AND session_id = $2
            ORDER BY joined_at ASC, created_at ASC, id ASC
            "#,
        )
        .bind(org_id)
        .bind(session_id.uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }
}

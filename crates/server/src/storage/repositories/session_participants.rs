use super::super::models::*;
use super::Database;
use crate::kernel_imports::{
    everruns_provider::typed_id::SessionId, everruns_provider::typed_id::SessionParticipantId,
};
use crate::storage::backend::MAX_SESSION_PARTICIPANT_HISTORY;
use anyhow::{Result, bail};
use everruns_platform::{SessionParticipantKind, SessionParticipantRole};

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
                principal_id, display_name, role, joined_at
            )
            VALUES (uuidv7(), $1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING id, org_id, session_id, kind, agent_id, agent_version_id,
                      principal_id, display_name, role, joined_at, left_at, created_at, updated_at
            "#,
        )
        .bind(input.org_id)
        .bind(input.session_id.uuid())
        .bind(input.kind.to_string())
        .bind(input.agent_id.map(|id| id.uuid()))
        .bind(input.agent_version_id.map(|id| id.uuid()))
        .bind(input.principal_id)
        .bind(input.display_name)
        .bind(input.role.to_string())
        .bind(joined_at)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn ensure_active_user_session_participant(
        &self,
        input: CreateSessionParticipantRow,
    ) -> Result<SessionParticipantRow> {
        if input.kind != SessionParticipantKind::User
            || input.role != SessionParticipantRole::Member
            || input.agent_id.is_some()
            || input.agent_version_id.is_some()
        {
            bail!("active user participant upsert requires a user member shape");
        }

        let joined_at = input.joined_at.unwrap_or_else(chrono::Utc::now);

        sqlx::query_as::<_, SessionParticipantRow>(
            r#"
            INSERT INTO session_participants (
                id, org_id, session_id, kind, agent_id, agent_version_id,
                principal_id, display_name, role, joined_at
            )
            VALUES (uuidv7(), $1, $2, 'user', NULL, NULL, $3, $4, 'member', $5)
            ON CONFLICT (session_id, principal_id)
                WHERE kind = 'user' AND left_at IS NULL
            DO UPDATE SET display_name = COALESCE(
                              NULLIF(EXCLUDED.display_name, ''),
                              session_participants.display_name
                          ),
                          updated_at = NOW()
            RETURNING id, org_id, session_id, kind, agent_id, agent_version_id,
                      principal_id, display_name, role, joined_at, left_at, created_at, updated_at
            "#,
        )
        .bind(input.org_id)
        .bind(input.session_id.uuid())
        .bind(input.principal_id)
        .bind(input.display_name)
        .bind(joined_at)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list_session_participants(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<Vec<SessionParticipantRow>> {
        sqlx::query_as::<_, SessionParticipantRow>(
            r#"
            SELECT id, org_id, session_id, kind, agent_id, agent_version_id,
                   principal_id, display_name, role, joined_at, left_at, created_at, updated_at
            FROM session_participants
            WHERE org_id = $1 AND session_id = $2
            ORDER BY joined_at ASC, created_at ASC, id ASC
            LIMIT $3
            "#,
        )
        .bind(org_id)
        .bind(session_id.uuid())
        .bind((MAX_SESSION_PARTICIPANT_HISTORY + 1) as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn leave_session_participant(
        &self,
        org_id: i64,
        session_id: SessionId,
        participant_id: SessionParticipantId,
    ) -> Result<Option<SessionParticipantRow>> {
        sqlx::query_as::<_, SessionParticipantRow>(
            r#"
            UPDATE session_participants
            SET left_at = COALESCE(left_at, NOW()),
                updated_at = NOW()
            WHERE org_id = $1 AND session_id = $2 AND id = $3
            RETURNING id, org_id, session_id, kind, agent_id, agent_version_id,
                      principal_id, display_name, role, joined_at, left_at, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(session_id.uuid())
        .bind(participant_id.uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }
}

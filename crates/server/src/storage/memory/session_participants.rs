use super::super::models::*;
use super::InMemoryDatabase;
use crate::kernel_imports::{
    everruns_provider::typed_id::SessionId, everruns_provider::typed_id::SessionParticipantId,
};
use crate::storage::backend::MAX_SESSION_PARTICIPANT_HISTORY;
use anyhow::{Result, bail};
use everruns_platform::{SessionParticipantKind, SessionParticipantRole};

impl InMemoryDatabase {
    pub(crate) async fn insert_initial_session_participants(
        &self,
        session: &SessionRow,
    ) -> Result<()> {
        if let Some(agent_id) = session.agent_id {
            self.insert_session_participant_unchecked(SessionParticipantRow {
                id: SessionParticipantId::new(),
                org_id: session.org_id,
                session_id: session.id,
                kind: SessionParticipantKind::Agent.to_string(),
                agent_id: Some(agent_id),
                agent_version_id: session.agent_version_id,
                principal_id: session.owner_principal_id,
                display_name: None,
                role: SessionParticipantRole::Host.to_string(),
                joined_at: session.created_at,
                left_at: None,
                created_at: session.created_at,
                updated_at: session.created_at,
            })?;
        }

        let display_name = session
            .resolved_owner_user_id
            .and_then(|user_id| {
                self.users
                    .read()
                    .get(&user_id)
                    .map(|user| user.name.trim().to_string())
            })
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "User".to_string());
        self.insert_session_participant_unchecked(SessionParticipantRow {
            id: SessionParticipantId::new(),
            org_id: session.org_id,
            session_id: session.id,
            kind: SessionParticipantKind::User.to_string(),
            agent_id: None,
            agent_version_id: None,
            principal_id: session.owner_principal_id,
            display_name: Some(display_name),
            role: SessionParticipantRole::Member.to_string(),
            joined_at: session.created_at,
            left_at: None,
            created_at: session.created_at,
            updated_at: session.created_at,
        })?;

        Ok(())
    }

    pub async fn create_session_participant(
        &self,
        input: CreateSessionParticipantRow,
    ) -> Result<SessionParticipantRow> {
        self.validate_session_participant(&input)?;

        let now = Self::now();
        let joined_at = input.joined_at.unwrap_or(now);
        let row = SessionParticipantRow {
            id: SessionParticipantId::new(),
            org_id: input.org_id,
            session_id: input.session_id,
            kind: input.kind.to_string(),
            agent_id: input.agent_id,
            agent_version_id: input.agent_version_id,
            principal_id: input.principal_id,
            display_name: input.display_name,
            role: input.role.to_string(),
            joined_at,
            left_at: None,
            created_at: now,
            updated_at: now,
        };

        self.insert_session_participant_unchecked(row.clone())?;
        Ok(row)
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

        let existing_id = {
            self.session_participants
                .read()
                .values()
                .find(|row| {
                    row.org_id == input.org_id
                        && row.session_id == input.session_id
                        && row.kind == SessionParticipantKind::User.to_string()
                        && row.principal_id == input.principal_id
                        && row.left_at.is_none()
                })
                .map(|row| row.id)
        };
        if let Some(existing_id) = existing_id {
            let mut participants = self.session_participants.write();
            let existing = participants
                .get_mut(&existing_id)
                .expect("participant exists");
            if input
                .display_name
                .as_ref()
                .is_some_and(|name| !name.is_empty())
            {
                existing.display_name = input.display_name;
                existing.updated_at = Self::now();
            }
            return Ok(existing.clone());
        }

        self.validate_session_participant(&input)?;

        let now = Self::now();
        let joined_at = input.joined_at.unwrap_or(now);
        let row = SessionParticipantRow {
            id: SessionParticipantId::new(),
            org_id: input.org_id,
            session_id: input.session_id,
            kind: SessionParticipantKind::User.to_string(),
            agent_id: None,
            agent_version_id: None,
            principal_id: input.principal_id,
            display_name: input.display_name,
            role: SessionParticipantRole::Member.to_string(),
            joined_at,
            left_at: None,
            created_at: now,
            updated_at: now,
        };

        self.insert_session_participant_unchecked(row.clone())?;
        Ok(row)
    }

    pub async fn list_session_participants(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<Vec<SessionParticipantRow>> {
        let mut rows: Vec<_> = self
            .session_participants
            .read()
            .values()
            .filter(|row| row.org_id == org_id && row.session_id == session_id)
            .cloned()
            .collect();
        rows.sort_by_key(|row| (row.joined_at, row.created_at, row.id.uuid()));
        rows.truncate(MAX_SESSION_PARTICIPANT_HISTORY + 1);
        Ok(rows)
    }

    pub async fn leave_session_participant(
        &self,
        org_id: i64,
        session_id: SessionId,
        participant_id: SessionParticipantId,
    ) -> Result<Option<SessionParticipantRow>> {
        let mut participants = self.session_participants.write();
        let Some(row) = participants.get_mut(&participant_id) else {
            return Ok(None);
        };
        if row.org_id != org_id || row.session_id != session_id {
            return Ok(None);
        }

        let now = Self::now();
        if row.left_at.is_none() {
            row.left_at = Some(now);
        }
        row.updated_at = now;
        Ok(Some(row.clone()))
    }

    fn validate_session_participant(&self, input: &CreateSessionParticipantRow) -> Result<()> {
        let session = self
            .sessions
            .read()
            .get(&input.session_id)
            .cloned()
            .filter(|session| session.org_id == input.org_id);
        if session.is_none() {
            bail!("session not found");
        }

        match input.kind {
            SessionParticipantKind::Agent if input.agent_id.is_none() => {
                bail!("agent participants require agent_id")
            }
            SessionParticipantKind::User
                if input.agent_id.is_some() || input.agent_version_id.is_some() =>
            {
                bail!("user participants cannot reference an agent")
            }
            _ => {}
        }

        if input.role == SessionParticipantRole::Host && input.kind != SessionParticipantKind::Agent
        {
            bail!("host participants must be agents");
        }

        if input.role == SessionParticipantRole::Host {
            let has_active_host = self.session_participants.read().values().any(|row| {
                row.session_id == input.session_id
                    && row.kind == "agent"
                    && row.role == "host"
                    && row.left_at.is_none()
            });
            if has_active_host {
                bail!("session already has an active host participant");
            }
        }

        if input.kind == SessionParticipantKind::User {
            let has_active_user = self.session_participants.read().values().any(|row| {
                row.session_id == input.session_id
                    && row.kind == "user"
                    && row.principal_id == input.principal_id
                    && row.left_at.is_none()
            });
            if has_active_user {
                bail!("session already has an active user participant for this principal");
            }
        }

        if input.kind == SessionParticipantKind::Agent
            && input.role == SessionParticipantRole::Member
        {
            let has_active_agent = self.session_participants.read().values().any(|row| {
                row.session_id == input.session_id
                    && row.kind == "agent"
                    && row.role == "member"
                    && row.agent_id == input.agent_id
                    && row.left_at.is_none()
            });
            if has_active_agent {
                bail!("session already has an active agent participant for this agent");
            }
        }

        Ok(())
    }

    fn insert_session_participant_unchecked(&self, row: SessionParticipantRow) -> Result<()> {
        if row.role == "host" {
            let has_active_host = self.session_participants.read().values().any(|existing| {
                existing.session_id == row.session_id
                    && existing.kind == "agent"
                    && existing.role == "host"
                    && existing.left_at.is_none()
            });
            if has_active_host {
                bail!("session already has an active host participant");
            }
        }
        if row.kind == "user" {
            let has_active_user = self.session_participants.read().values().any(|existing| {
                existing.session_id == row.session_id
                    && existing.kind == "user"
                    && existing.principal_id == row.principal_id
                    && existing.left_at.is_none()
            });
            if has_active_user {
                bail!("session already has an active user participant for this principal");
            }
        }
        if row.kind == "agent" && row.role == "member" {
            let has_active_agent = self.session_participants.read().values().any(|existing| {
                existing.session_id == row.session_id
                    && existing.kind == "agent"
                    && existing.role == "member"
                    && existing.agent_id == row.agent_id
                    && existing.left_at.is_none()
            });
            if has_active_agent {
                bail!("session already has an active agent participant for this agent");
            }
        }
        self.session_participants.write().insert(row.id, row);
        Ok(())
    }
}

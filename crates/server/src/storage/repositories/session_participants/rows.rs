// Session participant rows.

use crate::kernel_imports::contracts::typed_id::{
    AgentId, PrincipalId, SessionId, SessionParticipantId,
};
use crate::records::{SessionParticipant, SessionParticipantKind, SessionParticipantRole};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct SessionParticipantRow {
    pub id: SessionParticipantId,
    pub org_id: i64,
    pub session_id: SessionId,
    pub kind: String,
    pub agent_id: Option<AgentId>,
    pub principal_id: PrincipalId,
    pub display_name: Option<String>,
    pub role: String,
    pub joined_at: DateTime<Utc>,
    pub left_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SessionParticipantRow {
    pub fn to_core(&self) -> SessionParticipant {
        SessionParticipant {
            id: self.id,
            session_id: self.session_id,
            kind: SessionParticipantKind::from(self.kind.as_str()),
            agent_id: self.agent_id,
            principal_id: self.principal_id,
            display_name: self.display_name.clone(),
            role: SessionParticipantRole::from(self.role.as_str()),
            joined_at: self.joined_at,
            left_at: self.left_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreateSessionParticipantRow {
    pub org_id: i64,
    pub session_id: SessionId,
    pub kind: SessionParticipantKind,
    pub agent_id: Option<AgentId>,
    pub principal_id: PrincipalId,
    pub display_name: Option<String>,
    pub role: SessionParticipantRole,
    pub joined_at: Option<DateTime<Utc>>,
}

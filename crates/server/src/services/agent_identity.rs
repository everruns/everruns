// Agent identity service for business logic.
//
// Design Decision:
// - AgentIdentity is managed like a building block with archive/delete lifecycle.
// - Sessions and Apps validate identity assignment through this service layer so
//   background execution never binds archived/deleted identities.

use crate::storage::{
    StorageBackend,
    models::{AgentIdentityRow, CreateAgentIdentityRow, UpdateAgentIdentity},
};
use anyhow::Result;
use everruns_core::{
    AgentIdentity, AgentIdentityId, AgentIdentityStatus, Caller, Permission, Policy, Rule,
};
use everruns_macros::policy;
use std::sync::Arc;

use crate::api::agent_identities::{CreateAgentIdentityRequest, UpdateAgentIdentityRequest};

pub const AGENT_IDENTITY_VIEW: Policy = Policy {
    id: "agent_identity.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

pub const AGENT_IDENTITY_MANAGE: Policy = Policy {
    id: "agent_identity.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

pub const AGENT_IDENTITY_DANGEROUS: Policy = Policy {
    id: "agent_identity.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgAppsDangerous),
    ],
};

pub struct AgentIdentityService {
    db: Arc<StorageBackend>,
}

impl AgentIdentityService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    #[policy(AGENT_IDENTITY_MANAGE)]
    pub async fn create(
        &self,
        caller: &Caller,
        req: CreateAgentIdentityRequest,
    ) -> Result<AgentIdentity> {
        let row = self
            .db
            .create_agent_identity(CreateAgentIdentityRow {
                org_id: caller.org_id,
                id: AgentIdentityId::new(),
                name: req.name,
                description: req.description,
                avatar_url: req.avatar_url,
                locale: req.locale,
                timezone: req.timezone,
            })
            .await?;
        Ok(Self::row_to_identity(row))
    }

    #[policy(AGENT_IDENTITY_VIEW)]
    pub async fn get(&self, caller: &Caller, id: AgentIdentityId) -> Result<Option<AgentIdentity>> {
        Ok(self
            .db
            .get_agent_identity(caller.org_id, id)
            .await?
            .filter(|row| row.status != "deleted")
            .map(Self::row_to_identity))
    }

    #[policy(AGENT_IDENTITY_VIEW)]
    pub async fn list(
        &self,
        caller: &Caller,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<AgentIdentity>> {
        Ok(self
            .db
            .list_agent_identities(caller.org_id, search, include_archived)
            .await?
            .into_iter()
            .map(Self::row_to_identity)
            .collect())
    }

    #[policy(AGENT_IDENTITY_MANAGE)]
    pub async fn update(
        &self,
        caller: &Caller,
        id: AgentIdentityId,
        req: UpdateAgentIdentityRequest,
    ) -> Result<Option<AgentIdentity>> {
        let row = self
            .db
            .update_agent_identity(
                caller.org_id,
                id,
                UpdateAgentIdentity {
                    name: req.name,
                    description: req.description,
                    avatar_url: req.avatar_url,
                    locale: req.locale,
                    timezone: req.timezone,
                    status: req.status.map(|status| status.to_string()),
                },
            )
            .await?;
        Ok(row.map(Self::row_to_identity))
    }

    #[policy(AGENT_IDENTITY_MANAGE)]
    pub async fn delete(&self, caller: &Caller, id: AgentIdentityId) -> Result<bool> {
        self.db.delete_agent_identity(caller.org_id, id).await
    }

    #[policy(AGENT_IDENTITY_DANGEROUS)]
    pub async fn destroy(&self, caller: &Caller, id: AgentIdentityId) -> Result<bool> {
        self.db.destroy_agent_identity(caller.org_id, id).await
    }

    pub fn row_to_identity(row: AgentIdentityRow) -> AgentIdentity {
        AgentIdentity {
            id: row.id,
            name: row.name,
            description: row.description,
            avatar_url: row.avatar_url,
            locale: row.locale,
            timezone: row.timezone,
            status: AgentIdentityStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
            archived_at: row.archived_at,
            deleted_at: row.deleted_at,
        }
    }
}

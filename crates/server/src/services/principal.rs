// Principal service
//
// Encapsulates durable principal lifecycle, lineage validation, and default
// owner assignment for first-wave owned entities.

use crate::kernel_imports::{
    Caller, ExternalActor, PrincipalKind, PrincipalSummary,
    everruns_provider::typed_id::PrincipalId, org_public_id_from_internal,
};
use anyhow::{Result, anyhow};
use everruns_durable::UpdateField;
use everruns_platform::{ANONYMOUS_USER_ID, Principal, PrincipalStatus};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::storage::{
    StorageBackend,
    models::{CreatePrincipalRow, PrincipalRow, UpdatePrincipalRow},
};

const MAX_PRINCIPAL_DEPTH: usize = 8;

pub struct PrincipalService {
    db: Arc<StorageBackend>,
}

impl PrincipalService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    pub async fn get_summary(
        &self,
        org_id: i64,
        principal_id: PrincipalId,
    ) -> Result<Option<PrincipalSummary>> {
        Ok(self
            .db
            .get_principal(org_id, principal_id)
            .await?
            .filter(|row| row.status != "deleted")
            .map(row_to_principal)
            .map(|principal| principal.summary()))
    }

    pub async fn ensure_user_principal(&self, org_id: i64, user_id: Uuid) -> Result<PrincipalRow> {
        if let Some(row) = self
            .db
            .get_principal_by_subject(org_id, "user", user_id)
            .await?
        {
            return Ok(row);
        }

        let user = self
            .db
            .get_user(user_id)
            .await?
            .ok_or_else(|| anyhow!("User not found for principal"))?;

        self.db
            .create_principal(CreatePrincipalRow {
                id: PrincipalId::new(),
                org_id,
                kind: "user".to_string(),
                subject_id: Some(user.id),
                parent_principal_id: None,
                resolved_user_id: Some(user.id),
                metadata: json!({
                    "name": user.name,
                    "email": user.email,
                    "avatar_url": user.avatar_url,
                    "source": "user",
                }),
            })
            .await
    }

    pub async fn ensure_external_actor_principal(
        &self,
        org_id: i64,
        actor: &ExternalActor,
    ) -> Result<PrincipalRow> {
        let subject_key = format!("{}:{}", actor.source, actor.actor_id);
        let subject_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("everruns:external_actor:{subject_key}").as_bytes(),
        );

        self.db
            .create_principal(CreatePrincipalRow {
                id: PrincipalId::new(),
                org_id,
                kind: "user".to_string(),
                subject_id: Some(subject_id),
                parent_principal_id: None,
                resolved_user_id: None,
                metadata: json!({
                    "name": actor.display_label(),
                    "source": "external_actor",
                    "external_source": actor.source,
                    "external_actor_id": actor.actor_id,
                    "external_actor_name": actor.actor_name,
                    "external_actor_metadata": actor.metadata,
                }),
            })
            .await
    }

    pub async fn ensure_system_principal(&self, org_id: i64, name: &str) -> Result<PrincipalRow> {
        let subject_id = Uuid::from_u128(org_id as u128);
        if let Some(row) = self
            .db
            .get_principal_by_subject(org_id, "system", subject_id)
            .await?
        {
            return Ok(row);
        }

        self.db
            .create_principal(CreatePrincipalRow {
                id: PrincipalId::new(),
                org_id,
                kind: "system".to_string(),
                subject_id: Some(subject_id),
                parent_principal_id: None,
                resolved_user_id: None,
                metadata: json!({
                    "name": name,
                    "source": "system",
                }),
            })
            .await
    }

    pub async fn ensure_agent_identity_principal(
        &self,
        org_id: i64,
        agent_identity_id: everruns_provider::typed_id::AgentIdentityId,
        parent_principal_id: PrincipalId,
    ) -> Result<PrincipalRow> {
        let parent = self
            .db
            .get_principal(org_id, parent_principal_id)
            .await?
            .ok_or_else(|| anyhow!("Parent principal not found"))?;
        let resolved_user_id = self.resolve_user_from_lineage(org_id, &parent).await?;
        let identity = self
            .db
            .get_agent_identity(org_id, agent_identity_id)
            .await?
            .ok_or_else(|| anyhow!("Agent identity not found"))?;

        let metadata = json!({
            "name": identity.name,
            "description": identity.description,
            "avatar_url": identity.avatar_url,
            "locale": identity.locale,
            "timezone": identity.timezone,
            "source": "agent_identity",
        });

        if let Some(existing) = self
            .db
            .get_principal_by_subject(org_id, "agent_identity", agent_identity_id.uuid())
            .await?
        {
            return self
                .db
                .update_principal(
                    org_id,
                    existing.id,
                    UpdatePrincipalRow {
                        parent_principal_id: UpdateField::Unchanged,
                        resolved_user_id: UpdateField::Unchanged,
                        metadata: Some(metadata),
                        status: Some(identity.status.clone()),
                    },
                )
                .await?
                .ok_or_else(|| anyhow!("Failed to update principal"));
        }

        self.db
            .create_principal(CreatePrincipalRow {
                id: PrincipalId::new(),
                org_id,
                kind: "agent_identity".to_string(),
                subject_id: Some(agent_identity_id.uuid()),
                parent_principal_id: Some(parent.id),
                resolved_user_id,
                metadata,
            })
            .await
    }

    pub async fn default_owner_principal(
        &self,
        caller: &Caller,
        agent_identity_id: Option<everruns_provider::typed_id::AgentIdentityId>,
    ) -> Result<PrincipalRow> {
        let base_user_id = caller.user_id.or({
            if !caller.is_internal && caller.org_id == everruns_core::DEFAULT_ORG_ID {
                Some(ANONYMOUS_USER_ID)
            } else {
                None
            }
        });

        let human_owner = match base_user_id {
            Some(user_id) => self.ensure_user_principal(caller.org_id, user_id).await?,
            None => {
                self.ensure_system_principal(caller.org_id, "system-owner")
                    .await?
            }
        };

        if let Some(identity_id) = agent_identity_id {
            return self
                .ensure_agent_identity_principal(caller.org_id, identity_id, human_owner.id)
                .await;
        }

        Ok(human_owner)
    }

    pub async fn owner_for_entity(
        &self,
        org_id: i64,
        current_owner_principal_id: PrincipalId,
        current_resolved_owner_user_id: Option<Uuid>,
        agent_identity_id: Option<everruns_provider::typed_id::AgentIdentityId>,
    ) -> Result<PrincipalRow> {
        let current_owner = self
            .db
            .get_principal(org_id, current_owner_principal_id)
            .await?
            .ok_or_else(|| anyhow!("Current owner principal not found"))?;
        let base_owner = if let Some(user_id) = current_resolved_owner_user_id {
            self.ensure_user_principal(org_id, user_id).await?
        } else if current_owner.kind == "agent_identity" {
            match current_owner.parent_principal_id {
                Some(parent_id) => self
                    .db
                    .get_principal(org_id, parent_id)
                    .await?
                    .ok_or_else(|| anyhow!("Parent principal not found"))?,
                None => current_owner.clone(),
            }
        } else {
            current_owner.clone()
        };

        match agent_identity_id {
            Some(agent_identity_id) => {
                self.ensure_agent_identity_principal(org_id, agent_identity_id, base_owner.id)
                    .await
            }
            None => Ok(base_owner),
        }
    }

    pub async fn effective_owner_summary(
        &self,
        org_id: i64,
        resolved_user_id: Option<Uuid>,
    ) -> Result<Option<PrincipalSummary>> {
        let Some(user_id) = resolved_user_id else {
            return Ok(None);
        };
        Ok(self
            .db
            .get_principal_by_subject(org_id, "user", user_id)
            .await?
            .map(row_to_principal)
            .map(|principal| principal.summary()))
    }

    pub async fn sync_agent_identity_status(
        &self,
        org_id: i64,
        agent_identity_id: everruns_provider::typed_id::AgentIdentityId,
        status: PrincipalStatus,
    ) -> Result<()> {
        let Some(existing) = self
            .db
            .get_principal_by_subject(org_id, "agent_identity", agent_identity_id.uuid())
            .await?
        else {
            return Ok(());
        };
        self.db
            .update_principal(
                org_id,
                existing.id,
                UpdatePrincipalRow {
                    parent_principal_id: UpdateField::Unchanged,
                    resolved_user_id: UpdateField::Unchanged,
                    metadata: None,
                    status: Some(status.to_string()),
                },
            )
            .await?;
        Ok(())
    }

    /// Walk a principal's parent chain to the human user that ultimately owns
    /// it, if any. Returns `None` when the lineage terminates at the org's
    /// system principal: a system-owned (unattended) principal has no resolved
    /// human user. This lets an agent-identity principal be parented to the
    /// org system-owner — e.g. a lazily-created agent-trigger identity (EVE-758)
    /// — resolving to "no user" (system-owned) rather than erroring.
    async fn resolve_user_from_lineage(
        &self,
        org_id: i64,
        start: &PrincipalRow,
    ) -> Result<Option<Uuid>> {
        let mut current = start.clone();
        for _ in 0..MAX_PRINCIPAL_DEPTH {
            if current.org_id != org_id {
                anyhow::bail!("Principal parent must stay in the same org");
            }

            if current.kind == "user" {
                return current
                    .subject_id
                    .or(current.resolved_user_id)
                    .ok_or_else(|| anyhow!("User principal missing subject"))
                    .map(Some);
            }

            if let Some(user_id) = current.resolved_user_id {
                return Ok(Some(user_id));
            }

            // The org system-owner is a terminal root with no human behind it.
            if current.kind == "system" {
                return Ok(None);
            }

            let parent_id = current
                .parent_principal_id
                .ok_or_else(|| anyhow!("Owning principal must resolve to a user"))?;
            current = self
                .db
                .get_principal(org_id, parent_id)
                .await?
                .ok_or_else(|| anyhow!("Principal lineage parent not found"))?;
        }
        anyhow::bail!("Principal lineage exceeds depth limit");
    }
}

pub fn row_to_principal(row: PrincipalRow) -> Principal {
    Principal {
        id: row.id,
        organization_id: org_public_id_from_internal(row.org_id),
        kind: PrincipalKind::from(row.kind.as_str()),
        subject_id: row.subject_id,
        parent_principal_id: row.parent_principal_id,
        resolved_user_id: row.resolved_user_id,
        metadata: row.metadata,
        status: PrincipalStatus::from(row.status.as_str()),
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_imports::{AgentIdentityId, DEFAULT_ORG_ID};
    use crate::storage::{CreateAgentIdentityRow, CreateUserRow, StorageBackend};
    use everruns_platform::{ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID, ANONYMOUS_USER_NAME};

    async fn create_user_with_principal(
        db: &Arc<StorageBackend>,
        service: &PrincipalService,
        email: &str,
    ) -> PrincipalRow {
        let user = db
            .create_user(CreateUserRow {
                email: email.to_string(),
                name: email.to_string(),
                avatar_url: None,
                roles: vec!["member".to_string()],
                password_hash: None,
                email_verified: true,
                auth_provider: None,
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .unwrap();
        db.add_organization_member(DEFAULT_ORG_ID, user.id, "member")
            .await
            .unwrap();
        service
            .ensure_user_principal(DEFAULT_ORG_ID, user.id)
            .await
            .unwrap()
    }

    async fn create_agent_identity(db: &Arc<StorageBackend>, name: &str) -> AgentIdentityId {
        db.create_agent_identity(CreateAgentIdentityRow {
            org_id: DEFAULT_ORG_ID,
            id: AgentIdentityId::new(),
            name: name.to_string(),
            description: None,
            avatar_url: None,
            locale: None,
            timezone: None,
        })
        .await
        .unwrap()
        .id
    }

    async fn seed_anonymous_user(db: &Arc<StorageBackend>) {
        db.create_user_with_id(
            ANONYMOUS_USER_ID,
            CreateUserRow {
                email: ANONYMOUS_USER_EMAIL.to_string(),
                name: ANONYMOUS_USER_NAME.to_string(),
                avatar_url: None,
                roles: vec!["admin".to_string()],
                password_hash: None,
                email_verified: true,
                auth_provider: Some("none".to_string()),
                auth_provider_id: None,
                external_id: None,
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn ensure_agent_identity_principal_preserves_existing_parent() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = PrincipalService::new(db.clone());
        let owner_a = create_user_with_principal(&db, &service, "owner-a@example.com").await;
        let owner_b = create_user_with_principal(&db, &service, "owner-b@example.com").await;
        let identity_id = create_agent_identity(&db, "Ops Bot").await;

        let original = service
            .ensure_agent_identity_principal(DEFAULT_ORG_ID, identity_id, owner_a.id)
            .await
            .unwrap();
        let preserved = service
            .ensure_agent_identity_principal(DEFAULT_ORG_ID, identity_id, owner_b.id)
            .await
            .unwrap();

        assert_eq!(preserved.id, original.id);
        assert_eq!(preserved.parent_principal_id, Some(owner_a.id));
        assert_eq!(preserved.resolved_user_id, owner_a.resolved_user_id);
    }

    #[tokio::test]
    async fn owner_for_entity_clears_identity_back_to_human_owner() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = PrincipalService::new(db.clone());
        let user_principal =
            create_user_with_principal(&db, &service, "session-owner@example.com").await;
        let identity_id = create_agent_identity(&db, "Scheduler").await;
        let identity_principal = service
            .ensure_agent_identity_principal(DEFAULT_ORG_ID, identity_id, user_principal.id)
            .await
            .unwrap();

        let owner = service
            .owner_for_entity(
                DEFAULT_ORG_ID,
                identity_principal.id,
                identity_principal.resolved_user_id,
                None,
            )
            .await
            .unwrap();

        assert_eq!(owner.id, user_principal.id);
        assert_eq!(owner.kind, "user");
        assert_eq!(owner.resolved_user_id, user_principal.resolved_user_id);
    }

    #[tokio::test]
    async fn sync_agent_identity_status_updates_principal_row() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = PrincipalService::new(db.clone());
        let user_principal = create_user_with_principal(&db, &service, "status@example.com").await;
        let identity_id = create_agent_identity(&db, "Notifier").await;
        let principal = service
            .ensure_agent_identity_principal(DEFAULT_ORG_ID, identity_id, user_principal.id)
            .await
            .unwrap();

        service
            .sync_agent_identity_status(DEFAULT_ORG_ID, identity_id, PrincipalStatus::Archived)
            .await
            .unwrap();

        let stored = db
            .get_principal(DEFAULT_ORG_ID, principal.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, "archived");
        assert!(stored.archived_at.is_some());
    }

    #[tokio::test]
    async fn default_owner_principal_uses_anonymous_user_for_auth_none_requests() {
        let db = Arc::new(StorageBackend::in_memory());
        seed_anonymous_user(&db).await;
        let service = PrincipalService::new(db);

        let owner = service
            .default_owner_principal(
                &Caller {
                    org_id: DEFAULT_ORG_ID,
                    org_public_id: everruns_core::org_public_id_from_internal(DEFAULT_ORG_ID),
                    user_id: None,
                    role: everruns_core::organization::OrgRole::Owner,
                    is_platform_user: true,
                    is_internal: false,
                },
                None,
            )
            .await
            .unwrap();

        assert_eq!(owner.kind, "user");
        assert_eq!(owner.subject_id, Some(ANONYMOUS_USER_ID));
        assert_eq!(owner.resolved_user_id, Some(ANONYMOUS_USER_ID));
    }

    #[tokio::test]
    async fn default_owner_principal_keeps_internal_default_org_system_owned() {
        let db = Arc::new(StorageBackend::in_memory());
        seed_anonymous_user(&db).await;
        let service = PrincipalService::new(db);

        let owner = service
            .default_owner_principal(&Caller::internal(DEFAULT_ORG_ID), None)
            .await
            .unwrap();

        assert_eq!(owner.kind, "system");
        assert_eq!(
            owner.subject_id,
            Some(Uuid::from_u128(DEFAULT_ORG_ID as u128))
        );
        assert_eq!(owner.resolved_user_id, None);
    }
}

// In-memory storage: Principals

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_provider::typed_id::PrincipalId;
use uuid::Uuid;

impl InMemoryDatabase {
    pub async fn create_principal(&self, input: CreatePrincipalRow) -> Result<PrincipalRow> {
        if let Some(subject_id) = input.subject_id
            && let Some(existing) = self
                .principals
                .read()
                .values()
                .find(|row| {
                    row.org_id == input.org_id
                        && row.kind == input.kind
                        && row.subject_id == Some(subject_id)
                })
                .cloned()
        {
            let updated = PrincipalRow {
                parent_principal_id: input.parent_principal_id,
                resolved_user_id: input.resolved_user_id,
                metadata: input.metadata,
                status: "active".to_string(),
                archived_at: None,
                deleted_at: None,
                updated_at: Self::now(),
                ..existing
            };
            self.principals.write().insert(updated.id, updated.clone());
            return Ok(updated);
        }

        let row = PrincipalRow {
            id: input.id,
            public_id: input.id.to_string(),
            org_id: input.org_id,
            kind: input.kind,
            subject_id: input.subject_id,
            parent_principal_id: input.parent_principal_id,
            resolved_user_id: input.resolved_user_id,
            metadata: input.metadata,
            status: "active".to_string(),
            created_at: Self::now(),
            updated_at: Self::now(),
            archived_at: None,
            deleted_at: None,
        };
        self.principals.write().insert(row.id, row.clone());
        Ok(row)
    }

    pub async fn get_principal(
        &self,
        org_id: i64,
        id: PrincipalId,
    ) -> Result<Option<PrincipalRow>> {
        Ok(self
            .principals
            .read()
            .get(&id)
            .filter(|row| row.org_id == org_id)
            .cloned())
    }

    pub async fn get_principal_by_subject(
        &self,
        org_id: i64,
        kind: &str,
        subject_id: Uuid,
    ) -> Result<Option<PrincipalRow>> {
        Ok(self
            .principals
            .read()
            .values()
            .find(|row| {
                row.org_id == org_id
                    && row.kind == kind
                    && row.subject_id == Some(subject_id)
                    && row.status != "deleted"
            })
            .cloned())
    }

    pub async fn get_principals_for_session_list(
        &self,
        org_id: i64,
        principal_ids: &[PrincipalId],
        resolved_user_ids: &[Uuid],
    ) -> Result<Vec<PrincipalRow>> {
        Ok(self
            .principals
            .read()
            .values()
            .filter(|row| {
                row.org_id == org_id
                    && (principal_ids.contains(&row.id)
                        || (row.kind == "user"
                            && row
                                .subject_id
                                .is_some_and(|id| resolved_user_ids.contains(&id))))
            })
            .cloned()
            .collect())
    }

    pub async fn list_principals_by_resolved_user(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<Vec<PrincipalRow>> {
        let mut rows: Vec<_> = self
            .principals
            .read()
            .values()
            .filter(|row| {
                row.org_id == org_id
                    && row.resolved_user_id == Some(user_id)
                    && row.status != "deleted"
            })
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.created_at);
        Ok(rows)
    }

    pub async fn update_principal(
        &self,
        org_id: i64,
        id: PrincipalId,
        input: UpdatePrincipalRow,
    ) -> Result<Option<PrincipalRow>> {
        let mut principals = self.principals.write();
        let Some(principal) = principals.get_mut(&id) else {
            return Ok(None);
        };
        if principal.org_id != org_id {
            return Ok(None);
        }
        input
            .parent_principal_id
            .apply(&mut principal.parent_principal_id);
        input
            .resolved_user_id
            .apply(&mut principal.resolved_user_id);
        if let Some(metadata) = input.metadata {
            principal.metadata = metadata;
        }
        if let Some(status) = input.status {
            principal.status = status;
            match principal.status.as_str() {
                "archived" => {
                    principal.archived_at = Some(Self::now());
                    principal.deleted_at = None;
                }
                "deleted" => {
                    principal.deleted_at = Some(Self::now());
                }
                _ => {
                    principal.archived_at = None;
                    principal.deleted_at = None;
                }
            }
        }
        principal.updated_at = Self::now();
        Ok(Some(principal.clone()))
    }
}

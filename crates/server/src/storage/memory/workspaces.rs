// In-memory storage: Workspace CRUD.

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::{Result, bail};
use uuid::Uuid;

impl InMemoryDatabase {
    pub async fn create_workspace(
        &self,
        org_id: i64,
        input: CreateWorkspaceRow,
    ) -> Result<WorkspaceRow> {
        let now = Self::now();
        let mut workspaces = self.workspaces.write();

        if workspaces.values().any(|ws| {
            ws.org_id == org_id
                && ws.status != "deleted"
                && ws.name.eq_ignore_ascii_case(&input.name)
        }) {
            bail!("workspace name already exists");
        }

        let id = input.id.unwrap_or_else(Uuid::now_v7);
        let row = WorkspaceRow {
            id,
            org_id,
            public_id: input.public_id,
            name: input.name,
            description: input.description,
            owner_principal_id: input.owner_principal_id,
            resolved_owner_user_id: input.resolved_owner_user_id,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };
        workspaces.insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_workspace_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<WorkspaceRow>> {
        Ok(self
            .workspaces
            .read()
            .values()
            .find(|ws| ws.org_id == org_id && ws.public_id == public_id && ws.status != "deleted")
            .cloned())
    }

    pub async fn get_workspace_by_id(&self, org_id: i64, id: Uuid) -> Result<Option<WorkspaceRow>> {
        Ok(self
            .workspaces
            .read()
            .get(&id)
            .filter(|ws| ws.org_id == org_id && ws.status != "deleted")
            .cloned())
    }

    pub async fn get_workspace_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        Ok(self
            .workspaces
            .read()
            .values()
            .filter(|ws| ws.public_id == public_id && ws.status != "deleted")
            .max_by_key(|ws| ws.created_at)
            .map(|ws| ws.org_id))
    }

    pub async fn list_workspaces(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<WorkspaceRow>> {
        use super::matches_search_tokens;
        let mut rows: Vec<_> = self
            .workspaces
            .read()
            .values()
            .filter(|ws| {
                ws.org_id == org_id
                    && if include_archived {
                        ws.status != "deleted"
                    } else {
                        ws.status == "active"
                    }
            })
            .filter(|ws| {
                matches_search_tokens(search, &[&ws.name, ws.description.as_deref().unwrap_or("")])
            })
            .cloned()
            .collect();
        rows.sort_by_key(|ws| std::cmp::Reverse(ws.created_at));
        Ok(rows)
    }

    pub async fn update_workspace(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateWorkspace,
    ) -> Result<Option<WorkspaceRow>> {
        let mut workspaces = self.workspaces.write();
        if let Some(name) = input.name.as_ref()
            && workspaces.values().any(|ws| {
                ws.org_id == org_id
                    && ws.id != id
                    && ws.status != "deleted"
                    && ws.name.eq_ignore_ascii_case(name)
            })
        {
            bail!("workspace name already exists");
        }

        let Some(ws) = workspaces.get_mut(&id) else {
            return Ok(None);
        };
        if ws.org_id != org_id || ws.status == "deleted" {
            return Ok(None);
        }

        if let Some(name) = input.name {
            ws.name = name;
        }
        if let Some(description) = input.description {
            ws.description = description;
        }
        if let Some(status) = input.status {
            match status.as_str() {
                "active" => ws.archived_at = None,
                "archived" => ws.archived_at = Some(Self::now()),
                "deleted" => ws.deleted_at = Some(Self::now()),
                // Match the Postgres CHECK constraint so backends agree.
                other => bail!("invalid workspace status: {other}"),
            }
            ws.status = status;
        }
        ws.updated_at = Self::now();
        Ok(Some(ws.clone()))
    }

    pub async fn archive_workspace(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut workspaces = self.workspaces.write();
        let Some(ws) = workspaces.get_mut(&id) else {
            return Ok(false);
        };
        if ws.org_id != org_id || ws.status != "active" {
            return Ok(false);
        }
        ws.status = "archived".to_string();
        ws.archived_at = Some(Self::now());
        ws.updated_at = Self::now();
        Ok(true)
    }
}

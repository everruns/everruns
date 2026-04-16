// In-memory storage: Agent Identity CRUD

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_core::AgentIdentityId;

impl InMemoryDatabase {
    // ============================================
    // Agent Identity CRUD
    // ============================================

    pub async fn create_agent_identity(
        &self,
        input: CreateAgentIdentityRow,
    ) -> Result<AgentIdentityRow> {
        let row = AgentIdentityRow {
            id: input.id,
            org_id: input.org_id,
            name: input.name,
            description: input.description,
            avatar_url: input.avatar_url,
            locale: input.locale,
            timezone: input.timezone,
            status: "active".to_string(),
            created_at: Self::now(),
            updated_at: Self::now(),
            archived_at: None,
            deleted_at: None,
        };
        self.agent_identities.write().insert(row.id, row.clone());
        Ok(row)
    }

    pub async fn get_agent_identity(
        &self,
        org_id: i64,
        id: AgentIdentityId,
    ) -> Result<Option<AgentIdentityRow>> {
        Ok(self
            .agent_identities
            .read()
            .get(&id)
            .filter(|row| row.org_id == org_id)
            .cloned())
    }

    pub async fn list_agent_identities(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<AgentIdentityRow>> {
        let search = search.map(|s| s.to_lowercase());
        let mut rows: Vec<_> = self
            .agent_identities
            .read()
            .values()
            .filter(|row| row.org_id == org_id)
            .filter(|row| row.status != "deleted")
            .filter(|row| include_archived || row.status != "archived")
            .filter(|row| match &search {
                Some(search) => {
                    row.name.to_lowercase().contains(search)
                        || row
                            .description
                            .as_ref()
                            .map(|d: &String| d.to_lowercase().contains(search))
                            .unwrap_or(false)
                }
                None => true,
            })
            .cloned()
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.created_at));
        Ok(rows)
    }

    pub async fn update_agent_identity(
        &self,
        org_id: i64,
        id: AgentIdentityId,
        input: UpdateAgentIdentity,
    ) -> Result<Option<AgentIdentityRow>> {
        let mut rows = self.agent_identities.write();
        let Some(row) = rows.get_mut(&id) else {
            return Ok(None);
        };
        if row.org_id != org_id {
            return Ok(None);
        }
        if let Some(name) = input.name {
            row.name = name;
        }
        input.description.apply(&mut row.description);
        input.avatar_url.apply(&mut row.avatar_url);
        input.locale.apply(&mut row.locale);
        input.timezone.apply(&mut row.timezone);
        if let Some(status) = input.status {
            row.status = status;
        }
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn delete_agent_identity(&self, org_id: i64, id: AgentIdentityId) -> Result<bool> {
        let mut rows = self.agent_identities.write();
        let Some(row) = rows.get_mut(&id) else {
            return Ok(false);
        };
        if row.org_id != org_id || row.status != "active" {
            return Ok(false);
        }
        row.status = "archived".to_string();
        row.archived_at = Some(Self::now());
        row.updated_at = Self::now();
        Ok(true)
    }

    pub async fn destroy_agent_identity(&self, org_id: i64, id: AgentIdentityId) -> Result<bool> {
        let mut rows = self.agent_identities.write();
        let Some(row) = rows.get_mut(&id) else {
            return Ok(false);
        };
        if row.org_id != org_id || row.status != "archived" {
            return Ok(false);
        }
        row.status = "deleted".to_string();
        row.deleted_at = Some(Self::now());
        row.updated_at = Self::now();
        Ok(true)
    }
}

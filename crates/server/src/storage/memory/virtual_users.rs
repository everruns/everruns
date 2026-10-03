// In-memory storage: Virtual User CRUD

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_contracts::typed_id::VirtualUserId;

impl InMemoryDatabase {
    // ============================================
    // Virtual User CRUD
    // ============================================

    pub async fn create_virtual_user(&self, input: CreateVirtualUserRow) -> Result<VirtualUserRow> {
        let row = VirtualUserRow {
            usage: input.usage,
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
        let mut users = self.virtual_users.write();
        Ok(users.entry(row.id).or_insert(row).clone())
    }

    pub async fn get_virtual_user(
        &self,
        org_id: i64,
        id: VirtualUserId,
    ) -> Result<Option<VirtualUserRow>> {
        Ok(self
            .virtual_users
            .read()
            .get(&id)
            .filter(|row| row.org_id == org_id)
            .cloned())
    }

    /// Look up the owning org for a virtual user by its public id.
    pub async fn get_virtual_user_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        let Ok(id) = public_id.parse::<VirtualUserId>() else {
            return Ok(None);
        };
        Ok(self.virtual_users.read().get(&id).map(|r| r.org_id))
    }

    pub async fn list_virtual_users(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
        usage: Option<&str>,
        pagination: crate::api::common::Pagination,
    ) -> Result<(Vec<VirtualUserRow>, u32)> {
        let search = search.map(|s| s.to_lowercase());
        let mut rows: Vec<_> = self
            .virtual_users
            .read()
            .values()
            .filter(|row| row.org_id == org_id)
            .filter(|row| usage.is_none_or(|usage| row.usage == usage))
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
        rows.sort_by_key(|row| std::cmp::Reverse((row.created_at, row.id.uuid())));
        let total = rows.len() as u32;
        Ok((
            rows.into_iter()
                .skip(pagination.offset as usize)
                .take(pagination.limit as usize)
                .collect(),
            total,
        ))
    }

    pub async fn update_virtual_user(
        &self,
        org_id: i64,
        id: VirtualUserId,
        input: UpdateVirtualUser,
    ) -> Result<Option<VirtualUserRow>> {
        let mut rows = self.virtual_users.write();
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

    pub async fn delete_virtual_user(&self, org_id: i64, id: VirtualUserId) -> Result<bool> {
        let mut rows = self.virtual_users.write();
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

    pub async fn destroy_virtual_user(&self, org_id: i64, id: VirtualUserId) -> Result<bool> {
        let mut rows = self.virtual_users.write();
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

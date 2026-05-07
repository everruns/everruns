use super::super::models::*;
use super::{InMemoryDatabase, matches_search_tokens};
use anyhow::Result;
use uuid::Uuid;

impl InMemoryDatabase {
    pub async fn create_declarative_capability(
        &self,
        org_id: i64,
        input: CreateDeclarativeCapabilityRow,
    ) -> Result<DeclarativeCapabilityRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = DeclarativeCapabilityRow {
            id,
            org_id,
            public_id: input.public_id,
            name: input.name,
            display_name: input.display_name,
            description: input.description,
            status: "active".to_string(),
            definition: input.definition,
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };
        self.declarative_capabilities
            .write()
            .insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_declarative_capability(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        Ok(self
            .declarative_capabilities
            .read()
            .get(&id)
            .filter(|row| row.org_id == org_id)
            .cloned())
    }

    pub async fn get_declarative_capability_by_name(
        &self,
        org_id: i64,
        name: &str,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        Ok(self
            .declarative_capabilities
            .read()
            .values()
            .find(|row| row.org_id == org_id && row.name == name)
            .cloned())
    }

    pub async fn get_declarative_capability_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        Ok(self
            .declarative_capabilities
            .read()
            .values()
            .find(|row| row.org_id == org_id && row.public_id == public_id)
            .cloned())
    }

    pub async fn list_declarative_capabilities(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<DeclarativeCapabilityRow>> {
        let mut rows: Vec<_> = self
            .declarative_capabilities
            .read()
            .values()
            .filter(|row| row.org_id == org_id)
            .filter(|row| {
                if include_archived {
                    row.status != "deleted"
                } else {
                    row.status != "archived" && row.status != "deleted"
                }
            })
            .filter(|row| {
                matches_search_tokens(
                    search,
                    &[
                        &row.name,
                        row.display_name.as_deref().unwrap_or(""),
                        &row.description,
                    ],
                )
            })
            .cloned()
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.created_at));
        Ok(rows)
    }

    pub async fn update_declarative_capability(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateDeclarativeCapability,
    ) -> Result<Option<DeclarativeCapabilityRow>> {
        let mut rows = self.declarative_capabilities.write();
        let Some(row) = rows.get_mut(&id) else {
            return Ok(None);
        };
        if row.org_id != org_id {
            return Ok(None);
        }

        if let Some(name) = input.name {
            row.name = name;
        }
        let definition_changed = input.definition.is_some();
        if input.display_name.is_some() || definition_changed {
            row.display_name = input.display_name;
        }
        if let Some(description) = input.description {
            row.description = description;
        }
        if let Some(status) = input.status {
            if status == "archived" {
                row.archived_at = Some(Self::now());
            }
            row.status = status;
        }
        if let Some(definition) = input.definition {
            row.definition = definition;
        }
        row.updated_at = Self::now();
        Ok(Some(row.clone()))
    }

    pub async fn delete_declarative_capability(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut rows = self.declarative_capabilities.write();
        let Some(row) = rows.get_mut(&id) else {
            return Ok(false);
        };
        if row.org_id != org_id || !matches!(row.status.as_str(), "active" | "disabled") {
            return Ok(false);
        }
        row.status = "archived".to_string();
        row.archived_at = Some(Self::now());
        row.updated_at = Self::now();
        Ok(true)
    }

    pub async fn destroy_declarative_capability(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut rows = self.declarative_capabilities.write();
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

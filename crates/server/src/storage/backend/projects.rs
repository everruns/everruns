//! Projects: the nested work scope inside an organization.

use super::*;

impl StorageBackend {
    // ============================================
    // Projects
    // ============================================

    pub async fn create_project(&self, input: CreateProjectRow) -> Result<ProjectRow> {
        dispatch!(self, create_project, input)
    }

    pub async fn get_project(&self, org_id: i64, project_id: i64) -> Result<Option<ProjectRow>> {
        dispatch!(self, get_project, org_id, project_id)
    }

    pub async fn get_project_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<ProjectRow>> {
        dispatch!(self, get_project_by_public_id, org_id, public_id)
    }

    pub async fn get_project_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        dispatch!(self, get_project_organization_id, public_id)
    }

    pub async fn get_default_project(&self, org_id: i64) -> Result<Option<ProjectRow>> {
        dispatch!(self, get_default_project, org_id)
    }

    pub async fn list_projects(&self, org_id: i64) -> Result<Vec<ProjectRow>> {
        dispatch!(self, list_projects, org_id)
    }

    pub async fn update_project(
        &self,
        org_id: i64,
        project_id: i64,
        input: UpdateProject,
    ) -> Result<Option<ProjectRow>> {
        dispatch!(self, update_project, org_id, project_id, input)
    }

    pub async fn delete_project(&self, org_id: i64, project_id: i64) -> Result<bool> {
        dispatch!(self, delete_project, org_id, project_id)
    }

    /// Ensure an org has its default project, creating one if missing.
    ///
    /// Idempotent — used at org creation and on startup reconciliation so every
    /// org always has exactly one default project to fall back to.
    pub async fn ensure_default_project(&self, org_id: i64) -> Result<ProjectRow> {
        if let Some(existing) = self.get_default_project(org_id).await? {
            return Ok(existing);
        }
        self.create_project(CreateProjectRow {
            public_id: everruns_core::generate_project_public_id(),
            org_id,
            name: "Default".to_string(),
            description: None,
            is_default: true,
        })
        .await
    }
}

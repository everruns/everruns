// In-memory storage: Projects (grouping layer nested inside an org).

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;

impl InMemoryDatabase {
    pub async fn create_project(&self, input: CreateProjectRow) -> Result<ProjectRow> {
        let now = Self::now();
        let mut projects = self.projects.write();
        let project_id = projects.keys().max().unwrap_or(&0) + 1;
        let row = ProjectRow {
            project_id,
            public_id: input.public_id,
            org_id: input.org_id,
            name: input.name,
            description: input.description,
            is_default: input.is_default,
            created_at: now,
            updated_at: now,
        };
        projects.insert(project_id, row.clone());
        Ok(row)
    }

    pub async fn get_project(&self, org_id: i64, project_id: i64) -> Result<Option<ProjectRow>> {
        Ok(self
            .projects
            .read()
            .get(&project_id)
            .filter(|p| p.org_id == org_id)
            .cloned())
    }

    pub async fn get_project_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<ProjectRow>> {
        Ok(self
            .projects
            .read()
            .values()
            .find(|p| p.org_id == org_id && p.public_id == public_id)
            .cloned())
    }

    pub async fn get_project_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        Ok(self
            .projects
            .read()
            .values()
            .find(|p| p.public_id == public_id)
            .map(|p| p.org_id))
    }

    pub async fn get_default_project(&self, org_id: i64) -> Result<Option<ProjectRow>> {
        Ok(self
            .projects
            .read()
            .values()
            .find(|p| p.org_id == org_id && p.is_default)
            .cloned())
    }

    pub async fn list_projects(&self, org_id: i64) -> Result<Vec<ProjectRow>> {
        let projects = self.projects.read();
        let mut result: Vec<_> = projects
            .values()
            .filter(|p| p.org_id == org_id)
            .cloned()
            .collect();
        // Default first, then by name.
        result.sort_by(|a, b| {
            b.is_default
                .cmp(&a.is_default)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(result)
    }

    pub async fn update_project(
        &self,
        org_id: i64,
        project_id: i64,
        input: UpdateProject,
    ) -> Result<Option<ProjectRow>> {
        let mut projects = self.projects.write();
        if let Some(project) = projects.get_mut(&project_id).filter(|p| p.org_id == org_id) {
            if let Some(name) = input.name {
                project.name = name;
            }
            if let Some(description) = input.description {
                project.description = Some(description);
            }
            project.updated_at = Self::now();
            return Ok(Some(project.clone()));
        }
        Ok(None)
    }

    pub async fn delete_project(&self, org_id: i64, project_id: i64) -> Result<bool> {
        let mut projects = self.projects.write();
        let removable = projects
            .get(&project_id)
            .is_some_and(|p| p.org_id == org_id && !p.is_default);
        if removable {
            projects.remove(&project_id);
            return Ok(true);
        }
        Ok(false)
    }
}

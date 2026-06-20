// PostgreSQL repository: Projects (grouping layer nested inside an org).
//
// Project-scoped resources reference projects(project_id). Every org has exactly
// one `is_default` project, used as the reparent target and the fallback scope.

use super::super::models::*;
use super::Database;
use anyhow::Result;

impl Database {
    pub async fn create_project(&self, input: CreateProjectRow) -> Result<ProjectRow> {
        let row = sqlx::query_as::<_, ProjectRow>(
            r#"
            INSERT INTO projects (public_id, org_id, name, description, is_default)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING project_id, public_id, org_id, name, description, is_default, created_at, updated_at
            "#,
        )
        .bind(&input.public_id)
        .bind(input.org_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(input.is_default)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_project(&self, org_id: i64, project_id: i64) -> Result<Option<ProjectRow>> {
        let row = sqlx::query_as::<_, ProjectRow>(
            r#"
            SELECT project_id, public_id, org_id, name, description, is_default, created_at, updated_at
            FROM projects WHERE org_id = $1 AND project_id = $2
            "#,
        )
        .bind(org_id)
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_project_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<ProjectRow>> {
        let row = sqlx::query_as::<_, ProjectRow>(
            r#"
            SELECT project_id, public_id, org_id, name, description, is_default, created_at, updated_at
            FROM projects WHERE org_id = $1 AND public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Resolve a project's owning org from its public_id, regardless of caller.
    /// Used by the cross-org/project resolver; callers MUST gate on membership.
    pub async fn get_project_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT org_id FROM projects WHERE public_id = $1 LIMIT 1")
                .bind(public_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|r| r.0))
    }

    pub async fn get_default_project(&self, org_id: i64) -> Result<Option<ProjectRow>> {
        let row = sqlx::query_as::<_, ProjectRow>(
            r#"
            SELECT project_id, public_id, org_id, name, description, is_default, created_at, updated_at
            FROM projects WHERE org_id = $1 AND is_default LIMIT 1
            "#,
        )
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_projects(&self, org_id: i64) -> Result<Vec<ProjectRow>> {
        let rows = sqlx::query_as::<_, ProjectRow>(
            r#"
            SELECT project_id, public_id, org_id, name, description, is_default, created_at, updated_at
            FROM projects WHERE org_id = $1 ORDER BY is_default DESC, name
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn update_project(
        &self,
        org_id: i64,
        project_id: i64,
        input: UpdateProject,
    ) -> Result<Option<ProjectRow>> {
        let row = sqlx::query_as::<_, ProjectRow>(
            r#"
            UPDATE projects
            SET name = COALESCE($3, name),
                description = COALESCE($4, description),
                updated_at = NOW()
            WHERE org_id = $1 AND project_id = $2
            RETURNING project_id, public_id, org_id, name, description, is_default, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(project_id)
        .bind(&input.name)
        .bind(&input.description)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Delete a project. The default project cannot be deleted (guarded here and
    /// at the service layer); the DB FK from resources prevents deleting a
    /// non-empty project.
    pub async fn delete_project(&self, org_id: i64, project_id: i64) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM projects WHERE org_id = $1 AND project_id = $2 AND is_default = FALSE",
        )
        .bind(org_id)
        .bind(project_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

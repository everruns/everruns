// PostgreSQL repository: Principals

use super::super::models::*;
use super::Database;
use anyhow::Result;
use everruns_provider::typed_id::PrincipalId;
use uuid::Uuid;

impl Database {
    pub async fn create_principal(&self, input: CreatePrincipalRow) -> Result<PrincipalRow> {
        let row = if input.subject_id.is_some() {
            sqlx::query_as::<_, PrincipalRow>(
                r#"
                INSERT INTO principals (
                    id,
                    public_id,
                    org_id,
                    kind,
                    subject_id,
                    parent_principal_id,
                    resolved_user_id,
                    metadata,
                    status
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active')
                ON CONFLICT (org_id, kind, subject_id) DO UPDATE
                SET
                    parent_principal_id = EXCLUDED.parent_principal_id,
                    resolved_user_id = EXCLUDED.resolved_user_id,
                    metadata = EXCLUDED.metadata,
                    status = 'active',
                    archived_at = NULL,
                    deleted_at = NULL,
                    updated_at = NOW()
                RETURNING id, public_id, org_id, kind, subject_id, parent_principal_id, resolved_user_id, metadata, status, created_at, updated_at, archived_at, deleted_at
                "#,
            )
            .bind(input.id)
            .bind(input.id.to_string())
            .bind(input.org_id)
            .bind(&input.kind)
            .bind(input.subject_id)
            .bind(input.parent_principal_id)
            .bind(input.resolved_user_id)
            .bind(&input.metadata)
            .fetch_one(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PrincipalRow>(
                r#"
                INSERT INTO principals (
                    id,
                    public_id,
                    org_id,
                    kind,
                    subject_id,
                    parent_principal_id,
                    resolved_user_id,
                    metadata,
                    status
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active')
                RETURNING id, public_id, org_id, kind, subject_id, parent_principal_id, resolved_user_id, metadata, status, created_at, updated_at, archived_at, deleted_at
                "#,
            )
            .bind(input.id)
            .bind(input.id.to_string())
            .bind(input.org_id)
            .bind(&input.kind)
            .bind(input.subject_id)
            .bind(input.parent_principal_id)
            .bind(input.resolved_user_id)
            .bind(&input.metadata)
            .fetch_one(&self.pool)
            .await?
        };

        Ok(row)
    }

    pub async fn get_principal(
        &self,
        org_id: i64,
        id: PrincipalId,
    ) -> Result<Option<PrincipalRow>> {
        let row = sqlx::query_as::<_, PrincipalRow>(
            r#"
            SELECT id, public_id, org_id, kind, subject_id, parent_principal_id, resolved_user_id, metadata, status, created_at, updated_at, archived_at, deleted_at
            FROM principals
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_principal_by_subject(
        &self,
        org_id: i64,
        kind: &str,
        subject_id: Uuid,
    ) -> Result<Option<PrincipalRow>> {
        let row = sqlx::query_as::<_, PrincipalRow>(
            r#"
            SELECT id, public_id, org_id, kind, subject_id, parent_principal_id, resolved_user_id, metadata, status, created_at, updated_at, archived_at, deleted_at
            FROM principals
            WHERE org_id = $1 AND kind = $2 AND subject_id = $3
            "#,
        )
        .bind(org_id)
        .bind(kind)
        .bind(subject_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_principals_for_session_list(
        &self,
        org_id: i64,
        principal_ids: &[PrincipalId],
        resolved_user_ids: &[Uuid],
    ) -> Result<Vec<PrincipalRow>> {
        let principal_ids: Vec<Uuid> = principal_ids.iter().map(|id| id.uuid()).collect();
        let rows = sqlx::query_as::<_, PrincipalRow>(
            r#"
            SELECT id, public_id, org_id, kind, subject_id, parent_principal_id, resolved_user_id, metadata, status, created_at, updated_at, archived_at, deleted_at
            FROM principals
            WHERE org_id = $1
              AND (id = ANY($2) OR (kind = 'user' AND subject_id = ANY($3)))
            "#,
        )
        .bind(org_id)
        .bind(&principal_ids)
        .bind(resolved_user_ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn list_principals_by_resolved_user(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<Vec<PrincipalRow>> {
        let rows = sqlx::query_as::<_, PrincipalRow>(
            r#"
            SELECT id, public_id, org_id, kind, subject_id, parent_principal_id, resolved_user_id, metadata, status, created_at, updated_at, archived_at, deleted_at
            FROM principals
            WHERE org_id = $1 AND resolved_user_id = $2 AND status != 'deleted'
            ORDER BY created_at ASC
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_principal(
        &self,
        org_id: i64,
        id: PrincipalId,
        input: UpdatePrincipalRow,
    ) -> Result<Option<PrincipalRow>> {
        let row = sqlx::query_as::<_, PrincipalRow>(
            r#"
            UPDATE principals
            SET
                parent_principal_id = CASE WHEN $3 THEN $4 ELSE parent_principal_id END,
                resolved_user_id = CASE WHEN $5 THEN $6 ELSE resolved_user_id END,
                metadata = COALESCE($7, metadata),
                status = COALESCE($8, status),
                archived_at = CASE
                    WHEN $8 = 'archived' THEN COALESCE(archived_at, NOW())
                    WHEN $8 = 'active' THEN NULL
                    ELSE archived_at
                END,
                deleted_at = CASE
                    WHEN $8 = 'deleted' THEN COALESCE(deleted_at, NOW())
                    WHEN $8 = 'active' THEN NULL
                    ELSE deleted_at
                END,
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, public_id, org_id, kind, subject_id, parent_principal_id, resolved_user_id, metadata, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(input.parent_principal_id.is_changed())
        .bind(input.parent_principal_id.into_value())
        .bind(input.resolved_user_id.is_changed())
        .bind(input.resolved_user_id.into_value())
        .bind(&input.metadata)
        .bind(&input.status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }
}

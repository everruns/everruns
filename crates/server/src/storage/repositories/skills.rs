// PostgreSQL repository: Skills, Skill Files, Images

use super::super::models::*;
use super::Database;
use super::build_search_sql;
use anyhow::Result;
use uuid::Uuid;

impl Database {
    // ============================================
    // Skills
    // ============================================

    pub async fn create_skill(&self, org_id: i64, input: CreateSkillRow) -> Result<SkillRow> {
        let row = sqlx::query_as::<_, SkillRow>(
            r#"
            INSERT INTO skills (org_id, public_id, name, description, license, compatibility, metadata, allowed_tools, instructions, source_type, archive_data, version)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING id, public_id, org_id, name, description, license, compatibility, metadata, allowed_tools, instructions, source_type, archive_data, status, version, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.license)
        .bind(&input.compatibility)
        .bind(&input.metadata)
        .bind(&input.allowed_tools)
        .bind(&input.instructions)
        .bind(&input.source_type)
        .bind(&input.archive_data)
        .bind(&input.version)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_skill(&self, org_id: i64, id: Uuid) -> Result<Option<SkillRow>> {
        let row = sqlx::query_as::<_, SkillRow>(
            r#"
            SELECT id, public_id, org_id, name, description, license, compatibility, metadata, allowed_tools, instructions, source_type, archive_data, status, version, created_at, updated_at, archived_at, deleted_at
            FROM skills
            WHERE id = $1 AND org_id = $2
            "#,
        )
        .bind(id)
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_skill_by_name(&self, org_id: i64, name: &str) -> Result<Option<SkillRow>> {
        let row = sqlx::query_as::<_, SkillRow>(
            r#"
            SELECT id, public_id, org_id, name, description, license, compatibility, metadata, allowed_tools, instructions, source_type, archive_data, status, version, created_at, updated_at, archived_at, deleted_at
            FROM skills
            WHERE org_id = $1 AND name = $2
            "#,
        )
        .bind(org_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_skills(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<SkillRow>> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 2);
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status NOT IN ('archived', 'deleted')"
        };
        let sql = format!(
            r#"SELECT id, public_id, org_id, name, description, license, compatibility, metadata, allowed_tools, instructions, source_type, archive_data, status, version, created_at, updated_at, archived_at, deleted_at
                FROM skills
                WHERE org_id = $1{status_sql}{search_sql}
                ORDER BY created_at DESC"#
        );
        let mut query = sqlx::query_as::<_, SkillRow>(&sql).bind(org_id);
        for pat in &patterns {
            query = query.bind(pat);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn update_skill(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateSkill,
    ) -> Result<Option<SkillRow>> {
        let row = sqlx::query_as::<_, SkillRow>(
            r#"
            UPDATE skills
            SET
                name = COALESCE($3, name),
                description = COALESCE($4, description),
                license = COALESCE($5, license),
                compatibility = COALESCE($6, compatibility),
                metadata = COALESCE($7, metadata),
                allowed_tools = COALESCE($8, allowed_tools),
                instructions = COALESCE($9, instructions),
                status = COALESCE($10, status),
                version = COALESCE($11, version),
                archive_data = COALESCE($12, archive_data),
                source_type = COALESCE($13, source_type)
            WHERE id = $1 AND org_id = $2
            RETURNING id, public_id, org_id, name, description, license, compatibility, metadata, allowed_tools, instructions, source_type, archive_data, status, version, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(id)
        .bind(org_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.license)
        .bind(&input.compatibility)
        .bind(&input.metadata)
        .bind(&input.allowed_tools)
        .bind(&input.instructions)
        .bind(&input.status)
        .bind(&input.version)
        .bind(&input.archive_data)
        .bind(&input.source_type)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_skill(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE skills
            SET status = 'archived', archived_at = COALESCE(archived_at, NOW()), updated_at = NOW()
            WHERE id = $1 AND org_id = $2 AND status IN ('active', 'disabled')
            "#,
        )
        .bind(id)
        .bind(org_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn destroy_skill(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE skills
            SET status = 'deleted', deleted_at = COALESCE(deleted_at, NOW()), updated_at = NOW()
            WHERE id = $1 AND org_id = $2 AND status = 'archived'
            "#,
        )
        .bind(id)
        .bind(org_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Skill Files
    // ============================================

    pub async fn create_skill_file(&self, input: CreateSkillFileRow) -> Result<SkillFileRow> {
        let row = sqlx::query_as::<_, SkillFileRow>(
            r#"
            INSERT INTO skill_files (skill_id, path, content, content_binary, is_binary, size_bytes)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, skill_id, path, content, content_binary, is_binary, size_bytes, created_at
            "#,
        )
        .bind(input.skill_id)
        .bind(&input.path)
        .bind(&input.content)
        .bind(&input.content_binary)
        .bind(input.is_binary)
        .bind(input.size_bytes)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_skill_files(&self, skill_id: Uuid) -> Result<Vec<SkillFileRow>> {
        let rows = sqlx::query_as::<_, SkillFileRow>(
            r#"
            SELECT id, skill_id, path, content, content_binary, is_binary, size_bytes, created_at
            FROM skill_files
            WHERE skill_id = $1
            ORDER BY path ASC
            "#,
        )
        .bind(skill_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn delete_skill_files(&self, skill_id: Uuid) -> Result<u64> {
        let result = sqlx::query("DELETE FROM skill_files WHERE skill_id = $1")
            .bind(skill_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    // ============================================
    // Images
    // ============================================

    pub async fn create_image(&self, org_id: i64, input: CreateImageRow) -> Result<ImageRow> {
        let row = sqlx::query_as::<_, ImageRow>(
            r#"
            INSERT INTO images (org_id, filename, content_type, size_bytes, data, thumbnail_data, thumbnail_content_type, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id, org_id, filename, content_type, size_bytes, data, thumbnail_data, thumbnail_content_type, metadata, created_at
            "#,
        )
        .bind(org_id)
        .bind(&input.filename)
        .bind(&input.content_type)
        .bind(input.size_bytes)
        .bind(&input.data)
        .bind(&input.thumbnail_data)
        .bind(&input.thumbnail_content_type)
        .bind(&input.metadata)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_image(&self, org_id: i64, id: Uuid) -> Result<Option<ImageRow>> {
        let row = sqlx::query_as::<_, ImageRow>(
            r#"
            SELECT id, org_id, filename, content_type, size_bytes, data, thumbnail_data, thumbnail_content_type, metadata, created_at
            FROM images
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_image_info(&self, org_id: i64, id: Uuid) -> Result<Option<ImageInfoRow>> {
        let row = sqlx::query_as::<_, ImageInfoRow>(
            r#"
            SELECT id, org_id, filename, content_type, size_bytes, metadata, created_at
            FROM images
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_image(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM images WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn list_images(
        &self,
        org_id: i64,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ImageInfoRow>> {
        let rows = sqlx::query_as::<_, ImageInfoRow>(
            r#"
            SELECT id, org_id, filename, content_type, size_bytes, metadata, created_at
            FROM images
            WHERE org_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(org_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }
}

// Plugin Marketplace and Install storage — PostgreSQL implementation.

use super::super::models::*;
use super::{Database, build_search_sql};
use anyhow::Result;
use uuid::Uuid;

impl Database {
    // ========================================================================
    // Plugin Marketplaces
    // ========================================================================

    pub async fn create_plugin_marketplace(
        &self,
        org_id: i64,
        input: CreatePluginMarketplaceRow,
    ) -> Result<PluginMarketplaceRow> {
        Ok(sqlx::query_as::<_, PluginMarketplaceRow>(
            r#"
            INSERT INTO plugin_marketplaces (org_id, public_id, name, source_type, source)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, org_id, public_id, name, source_type, source, status, catalog,
                      last_synced_at, last_synced_sha, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.source_type)
        .bind(&input.source)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_plugin_marketplace(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PluginMarketplaceRow>> {
        Ok(sqlx::query_as::<_, PluginMarketplaceRow>(
            r#"
            SELECT id, org_id, public_id, name, source_type, source, status, catalog,
                   last_synced_at, last_synced_sha, created_at, updated_at
            FROM plugin_marketplaces
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn get_plugin_marketplace_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<PluginMarketplaceRow>> {
        Ok(sqlx::query_as::<_, PluginMarketplaceRow>(
            r#"
            SELECT id, org_id, public_id, name, source_type, source, status, catalog,
                   last_synced_at, last_synced_sha, created_at, updated_at
            FROM plugin_marketplaces
            WHERE org_id = $1 AND public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn list_plugin_marketplaces(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<PluginMarketplaceRow>> {
        let (search_sql, patterns) = build_search_sql(search, "LOWER(name)", 2);
        let sql = format!(
            r#"
            SELECT id, org_id, public_id, name, source_type, source, status, catalog,
                   last_synced_at, last_synced_sha, created_at, updated_at
            FROM plugin_marketplaces
            WHERE org_id = $1{search_sql}
            ORDER BY created_at DESC
            "#
        );
        let mut query =
            sqlx::query_as::<_, PluginMarketplaceRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(org_id);
        for pattern in &patterns {
            query = query.bind(pattern);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn update_plugin_marketplace(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePluginMarketplace,
    ) -> Result<Option<PluginMarketplaceRow>> {
        Ok(sqlx::query_as::<_, PluginMarketplaceRow>(
            r#"
            UPDATE plugin_marketplaces
            SET
                name = COALESCE($3, name),
                source_type = COALESCE($4, source_type),
                source = COALESCE($5, source),
                status = COALESCE($6, status),
                catalog = CASE WHEN $7::jsonb IS NOT NULL THEN $7 ELSE catalog END,
                last_synced_at = CASE WHEN $8::timestamptz IS NOT NULL THEN $8 ELSE last_synced_at END,
                last_synced_sha = CASE WHEN $9::bool THEN $10 ELSE last_synced_sha END
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, public_id, name, source_type, source, status, catalog,
                      last_synced_at, last_synced_sha, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.name)
        .bind(&input.source_type)
        .bind(&input.source)
        .bind(&input.status)
        .bind(&input.catalog)
        .bind(input.last_synced_at)
        .bind(input.last_synced_sha.is_some())
        .bind(input.last_synced_sha.flatten().as_deref())
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn delete_plugin_marketplace(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result =
            sqlx::query(r#"DELETE FROM plugin_marketplaces WHERE org_id = $1 AND id = $2"#)
                .bind(org_id)
                .bind(id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    // ========================================================================
    // Plugin Installs
    // ========================================================================

    pub async fn create_plugin_install(
        &self,
        org_id: i64,
        input: CreatePluginInstallRow,
    ) -> Result<PluginInstallRow> {
        Ok(sqlx::query_as::<_, PluginInstallRow>(
            r#"
            INSERT INTO plugin_installs
                (org_id, public_id, name, marketplace_id, source, version, pinned_sha,
                 manifest, definition, warnings)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id, org_id, public_id, name, marketplace_id, source, version, pinned_sha,
                      manifest, definition, warnings, status, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(input.marketplace_id)
        .bind(&input.source)
        .bind(&input.version)
        .bind(&input.pinned_sha)
        .bind(&input.manifest)
        .bind(&input.definition)
        .bind(&input.warnings)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_plugin_install(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PluginInstallRow>> {
        Ok(sqlx::query_as::<_, PluginInstallRow>(
            r#"
            SELECT id, org_id, public_id, name, marketplace_id, source, version, pinned_sha,
                   manifest, definition, warnings, status, created_at, updated_at
            FROM plugin_installs
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn get_plugin_install_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<PluginInstallRow>> {
        Ok(sqlx::query_as::<_, PluginInstallRow>(
            r#"
            SELECT id, org_id, public_id, name, marketplace_id, source, version, pinned_sha,
                   manifest, definition, warnings, status, created_at, updated_at
            FROM plugin_installs
            WHERE org_id = $1 AND public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn get_plugin_install_by_name(
        &self,
        org_id: i64,
        name: &str,
    ) -> Result<Option<PluginInstallRow>> {
        Ok(sqlx::query_as::<_, PluginInstallRow>(
            r#"
            SELECT id, org_id, public_id, name, marketplace_id, source, version, pinned_sha,
                   manifest, definition, warnings, status, created_at, updated_at
            FROM plugin_installs
            WHERE org_id = $1 AND name = $2
            "#,
        )
        .bind(org_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn list_plugin_installs(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<PluginInstallRow>> {
        let (search_sql, patterns) = build_search_sql(search, "LOWER(name)", 2);
        let sql = format!(
            r#"
            SELECT id, org_id, public_id, name, marketplace_id, source, version, pinned_sha,
                   manifest, definition, warnings, status, created_at, updated_at
            FROM plugin_installs
            WHERE org_id = $1{search_sql}
            ORDER BY created_at DESC
            "#
        );
        let mut query =
            sqlx::query_as::<_, PluginInstallRow>(sqlx::AssertSqlSafe(sql.as_str())).bind(org_id);
        for pattern in &patterns {
            query = query.bind(pattern);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn list_active_plugin_installs(&self, org_id: i64) -> Result<Vec<PluginInstallRow>> {
        Ok(sqlx::query_as::<_, PluginInstallRow>(
            r#"
            SELECT id, org_id, public_id, name, marketplace_id, source, version, pinned_sha,
                   manifest, definition, warnings, status, created_at, updated_at
            FROM plugin_installs
            WHERE org_id = $1 AND status = 'active'
            ORDER BY created_at DESC
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn update_plugin_install(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePluginInstall,
    ) -> Result<Option<PluginInstallRow>> {
        Ok(sqlx::query_as::<_, PluginInstallRow>(
            r#"
            UPDATE plugin_installs
            SET
                status = COALESCE($3, status),
                source = COALESCE($4, source),
                version = COALESCE($5, version),
                pinned_sha = COALESCE($6, pinned_sha),
                manifest = COALESCE($7, manifest),
                definition = COALESCE($8, definition),
                warnings = COALESCE($9, warnings)
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, public_id, name, marketplace_id, source, version, pinned_sha,
                      manifest, definition, warnings, status, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.status)
        .bind(&input.source)
        .bind(&input.version)
        .bind(&input.pinned_sha)
        .bind(&input.manifest)
        .bind(&input.definition)
        .bind(&input.warnings)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn delete_plugin_install(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(r#"DELETE FROM plugin_installs WHERE org_id = $1 AND id = $2"#)
            .bind(org_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

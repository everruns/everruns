// PostgreSQL repository: LLM Providers, LLM Models, LLM Generations (Usage Tracking)

use super::super::models::*;
use super::Database;
use anyhow::Result;

use uuid::Uuid;

impl Database {
    // ============================================
    // LLM Providers
    // ============================================

    pub async fn create_llm_provider(
        &self,
        org_id: i64,
        input: CreateLlmProviderRow,
    ) -> Result<LlmProviderRow> {
        let api_key_set = input.api_key_encrypted.is_some();
        let settings = input.settings.unwrap_or(serde_json::json!({}));

        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            INSERT INTO llm_providers (org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, settings)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, status, settings, last_synced_at, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(&input.name)
        .bind(&input.provider_type)
        .bind(&input.base_url)
        .bind(&input.api_key_encrypted)
        .bind(api_key_set)
        .bind(&settings)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create a provider with a specific ID (for seeding)
    /// Uses ON CONFLICT DO NOTHING for idempotency
    /// Returns None if provider already exists
    /// Create or update LLM provider with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_llm_provider_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateLlmProviderRow,
    ) -> Result<Option<LlmProviderRow>> {
        let api_key_set = input.api_key_encrypted.is_some();
        let settings = input.settings.unwrap_or(serde_json::json!({}));

        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            INSERT INTO llm_providers (id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, settings)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                provider_type = EXCLUDED.provider_type,
                updated_at = NOW()
            WHERE
                llm_providers.name IS DISTINCT FROM EXCLUDED.name
                OR llm_providers.provider_type IS DISTINCT FROM EXCLUDED.provider_type
            RETURNING id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, status, settings, last_synced_at, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(org_id)
        .bind(&input.name)
        .bind(&input.provider_type)
        .bind(&input.base_url)
        .bind(&input.api_key_encrypted)
        .bind(api_key_set)
        .bind(&settings)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_llm_provider(&self, org_id: i64, id: Uuid) -> Result<Option<LlmProviderRow>> {
        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            SELECT id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, status, settings, last_synced_at, created_at, updated_at
            FROM llm_providers
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_llm_providers(&self, org_id: i64) -> Result<Vec<LlmProviderRow>> {
        let rows = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            SELECT id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, status, settings, last_synced_at, created_at, updated_at
            FROM llm_providers
            WHERE org_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_llm_provider(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateLlmProvider,
    ) -> Result<Option<LlmProviderRow>> {
        // If updating api_key, also update api_key_set
        let api_key_set = input.api_key_encrypted.as_ref().map(|_| true);

        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            UPDATE llm_providers
            SET
                name = COALESCE($3, name),
                provider_type = COALESCE($4, provider_type),
                base_url = COALESCE($5, base_url),
                api_key_encrypted = COALESCE($6, api_key_encrypted),
                api_key_set = COALESCE($7, api_key_set),
                status = COALESCE($8, status),
                settings = COALESCE($9, settings),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, status, settings, last_synced_at, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.name)
        .bind(&input.provider_type)
        .bind(&input.base_url)
        .bind(&input.api_key_encrypted)
        .bind(api_key_set)
        .bind(&input.status)
        .bind(&input.settings)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_llm_provider(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM llm_providers WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Update provider's last_synced_at timestamp
    pub async fn update_provider_last_synced(
        &self,
        org_id: i64,
        id: Uuid,
        last_synced_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE llm_providers SET last_synced_at = $3, updated_at = NOW() WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(id)
        .bind(last_synced_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get the default LLM model with provider info.
    /// Reads from organization_settings.default_model_id.
    pub async fn get_default_llm_model(
        &self,
        org_id: i64,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let row = sqlx::query_as::<_, LlmModelWithProviderRow>(
            r#"
            SELECT m.id, m.org_id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_favorite, m.installed, m.status, m.source, m.last_seen_at, m.provider_metadata, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type
            FROM organization_settings os
            JOIN llm_models m ON m.id = os.default_model_id AND m.org_id = os.org_id
            JOIN llm_providers p ON m.provider_id = p.id AND p.org_id = m.org_id
            WHERE os.org_id = $1 AND m.status = 'active' AND p.status = 'active'
            "#,
        )
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // ============================================
    // LLM Models
    // ============================================

    pub async fn create_llm_model(
        &self,
        org_id: i64,
        input: CreateLlmModelRow,
    ) -> Result<LlmModelRow> {
        let capabilities_json = serde_json::to_value(&input.capabilities)?;

        let row = sqlx::query_as::<_, LlmModelRow>(
            r#"
            INSERT INTO llm_models (org_id, provider_id, model_id, display_name, capabilities, is_favorite, installed, source, provider_metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING id, org_id, provider_id, model_id, display_name, capabilities, is_favorite, installed, status, source, last_seen_at, provider_metadata, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(input.provider_id)
        .bind(&input.model_id)
        .bind(&input.display_name)
        .bind(&capabilities_json)
        .bind(input.is_favorite)
        .bind(input.installed)
        .bind(&input.source)
        .bind(&input.provider_metadata)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create or update a model with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_llm_model_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateLlmModelRow,
    ) -> Result<Option<LlmModelRow>> {
        let capabilities_json = serde_json::to_value(&input.capabilities)?;

        let row = sqlx::query_as::<_, LlmModelRow>(
            r#"
            INSERT INTO llm_models (id, org_id, provider_id, model_id, display_name, capabilities, is_favorite, installed, source, provider_metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO UPDATE SET
                display_name = EXCLUDED.display_name,
                is_favorite = EXCLUDED.is_favorite,
                installed = EXCLUDED.installed,
                updated_at = NOW()
            WHERE
                llm_models.display_name IS DISTINCT FROM EXCLUDED.display_name
                OR llm_models.is_favorite IS DISTINCT FROM EXCLUDED.is_favorite
                OR llm_models.installed IS DISTINCT FROM EXCLUDED.installed
            RETURNING id, org_id, provider_id, model_id, display_name, capabilities, is_favorite, installed, status, source, last_seen_at, provider_metadata, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(org_id)
        .bind(input.provider_id)
        .bind(&input.model_id)
        .bind(&input.display_name)
        .bind(&capabilities_json)
        .bind(input.is_favorite)
        .bind(input.installed)
        .bind(&input.source)
        .bind(&input.provider_metadata)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_llm_model(&self, org_id: i64, id: Uuid) -> Result<Option<LlmModelRow>> {
        let row = sqlx::query_as::<_, LlmModelRow>(
            r#"
            SELECT id, org_id, provider_id, model_id, display_name, capabilities, is_favorite, installed, status, source, last_seen_at, provider_metadata, created_at, updated_at
            FROM llm_models
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_llm_model_with_provider(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let row = sqlx::query_as::<_, LlmModelWithProviderRow>(
            r#"
            SELECT m.id, m.org_id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_favorite, m.installed, m.status, m.source, m.last_seen_at, m.provider_metadata, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type
            FROM llm_models m
            JOIN llm_providers p ON m.provider_id = p.id AND p.org_id = m.org_id
            WHERE m.org_id = $1 AND m.id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_llm_models_for_provider(
        &self,
        org_id: i64,
        provider_id: Uuid,
    ) -> Result<Vec<LlmModelRow>> {
        let rows = sqlx::query_as::<_, LlmModelRow>(
            r#"
            SELECT id, org_id, provider_id, model_id, display_name, capabilities, is_favorite, installed, status, source, last_seen_at, provider_metadata, created_at, updated_at
            FROM llm_models
            WHERE org_id = $1 AND provider_id = $2
            ORDER BY display_name ASC
            "#,
        )
        .bind(org_id)
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn list_all_llm_models(&self, org_id: i64) -> Result<Vec<LlmModelWithProviderRow>> {
        let rows = sqlx::query_as::<_, LlmModelWithProviderRow>(
            r#"
            SELECT m.id, m.org_id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_favorite, m.installed, m.status, m.source, m.last_seen_at, m.provider_metadata, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type
            FROM llm_models m
            JOIN llm_providers p ON m.provider_id = p.id AND p.org_id = m.org_id
            WHERE m.status = 'active' AND p.status = 'active' AND m.org_id = $1
            ORDER BY m.installed DESC, m.is_favorite DESC, p.name ASC, m.display_name ASC
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_llm_model(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateLlmModel,
    ) -> Result<Option<LlmModelRow>> {
        let capabilities_json = input
            .capabilities
            .map(|c| serde_json::to_value(&c))
            .transpose()?;

        let row = sqlx::query_as::<_, LlmModelRow>(
            r#"
            UPDATE llm_models
            SET
                model_id = COALESCE($3, model_id),
                display_name = COALESCE($4, display_name),
                capabilities = COALESCE($5, capabilities),
                is_favorite = COALESCE($6, is_favorite),
                installed = COALESCE($7, installed),
                status = COALESCE($8, status),
                last_seen_at = COALESCE($9, last_seen_at),
                provider_metadata = COALESCE($10, provider_metadata),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, provider_id, model_id, display_name, capabilities, is_favorite, installed, status, source, last_seen_at, provider_metadata, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.model_id)
        .bind(&input.display_name)
        .bind(&capabilities_json)
        .bind(input.is_favorite)
        .bind(input.installed)
        .bind(&input.status)
        .bind(input.last_seen_at)
        .bind(&input.provider_metadata)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_llm_model(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM llm_models WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Get model by model_id string (for resolving agent model references)
    pub async fn get_llm_model_by_model_id(
        &self,
        org_id: i64,
        model_id: &str,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let row = sqlx::query_as::<_, LlmModelWithProviderRow>(
            r#"
            SELECT m.id, m.org_id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_favorite, m.installed, m.status, m.source, m.last_seen_at, m.provider_metadata, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type
            FROM llm_models m
            JOIN llm_providers p ON m.provider_id = p.id AND p.org_id = m.org_id
            WHERE m.model_id = $1 AND m.status = 'active' AND p.status = 'active' AND m.org_id = $2
            "#,
        )
        .bind(model_id)
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // ============================================
    // LLM Generations (Usage Tracking)
    // ============================================

    #[allow(clippy::too_many_arguments)]
    pub async fn create_llm_generation(
        &self,
        org_id: i64,
        session_id: Uuid,
        turn_id: Option<Uuid>,
        event_id: Option<Uuid>,
        model: String,
        provider: Option<String>,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        duration_ms: Option<i32>,
        finish_reason: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO llm_generations (
                org_id, session_id, turn_id, event_id, model, provider,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                duration_ms, finish_reason, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            "#,
        )
        .bind(org_id)
        .bind(session_id)
        .bind(turn_id)
        .bind(event_id)
        .bind(&model)
        .bind(&provider)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cache_read_tokens)
        .bind(cache_creation_tokens)
        .bind(duration_ms)
        .bind(&finish_reason)
        .bind(created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn increment_session_usage(
        &self,
        session_id: Uuid,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE sessions
            SET
                total_input_tokens = total_input_tokens + $2,
                total_output_tokens = total_output_tokens + $3,
                total_cache_read_tokens = total_cache_read_tokens + $4,
                total_cache_creation_tokens = total_cache_creation_tokens + $5
            WHERE id = $1
            "#,
        )
        .bind(session_id)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cache_read_tokens)
        .bind(cache_creation_tokens)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn increment_agent_usage(
        &self,
        agent_id: Uuid,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE agents
            SET
                total_input_tokens = total_input_tokens + $2,
                total_output_tokens = total_output_tokens + $3,
                total_cache_read_tokens = total_cache_read_tokens + $4,
                total_cache_creation_tokens = total_cache_creation_tokens + $5
            WHERE id = $1
            "#,
        )
        .bind(agent_id)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cache_read_tokens)
        .bind(cache_creation_tokens)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

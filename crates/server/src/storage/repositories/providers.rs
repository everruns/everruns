// PostgreSQL repository: LLM Providers, LLM Models, LLM Generations (Usage Tracking)

use super::super::models::*;
use super::Database;
use anyhow::Result;
use tracing::warn;

use uuid::Uuid;

impl Database {
    // ============================================
    // LLM Providers
    // ============================================

    pub async fn create_provider(
        &self,
        org_id: i64,
        input: CreateProviderRow,
    ) -> Result<ProviderRow> {
        let api_key_set = input.api_key_encrypted.is_some();
        let settings = input.settings.unwrap_or(serde_json::json!({}));

        let row = sqlx::query_as::<_, ProviderRow>(
            r#"
            INSERT INTO providers (org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, settings)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, status, settings, managed, last_synced_at, created_at, updated_at
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
    pub async fn create_provider_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateProviderRow,
    ) -> Result<Option<ProviderRow>> {
        let api_key_set = input.api_key_encrypted.is_some();
        let settings = input.settings.unwrap_or(serde_json::json!({}));

        let row = sqlx::query_as::<_, ProviderRow>(
            r#"
            INSERT INTO providers (id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, settings)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                provider_type = EXCLUDED.provider_type,
                updated_at = NOW()
            WHERE
                providers.name IS DISTINCT FROM EXCLUDED.name
                OR providers.provider_type IS DISTINCT FROM EXCLUDED.provider_type
            RETURNING id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, status, settings, managed, last_synced_at, created_at, updated_at
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

    pub async fn get_provider(&self, org_id: i64, id: Uuid) -> Result<Option<ProviderRow>> {
        let row = sqlx::query_as::<_, ProviderRow>(
            r#"
            SELECT id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, status, settings, managed, last_synced_at, created_at, updated_at
            FROM providers
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_providers(&self, org_id: i64) -> Result<Vec<ProviderRow>> {
        let rows = sqlx::query_as::<_, ProviderRow>(
            r#"
            SELECT id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, status, settings, managed, last_synced_at, created_at, updated_at
            FROM providers
            WHERE org_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_provider(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateProvider,
    ) -> Result<Option<ProviderRow>> {
        // If updating api_key, also update api_key_set
        let api_key_set = input.api_key_encrypted.as_ref().map(|_| true);

        let row = sqlx::query_as::<_, ProviderRow>(
            r#"
            UPDATE providers
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
            RETURNING id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, status, settings, managed, last_synced_at, created_at, updated_at
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

    pub async fn delete_provider(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM providers WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Mark (or unmark) a provider as host-managed (EVE-810).
    ///
    /// Host-only write path: the OSS providers API never sets this flag (a
    /// managed provider is provisioned by an embedder directly through storage,
    /// e.g. from an `OrgInitializer`). Returns whether a row was updated.
    pub async fn set_provider_managed(&self, org_id: i64, id: Uuid, managed: bool) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE providers SET managed = $3, updated_at = NOW() WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(id)
        .bind(managed)
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
            "UPDATE providers SET last_synced_at = $3, updated_at = NOW() WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(id)
        .bind(last_synced_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get the default LLM model with provider info.
    /// Uses the organization selection when present, otherwise the platform fallback.
    ///
    /// Fail-closed: filters to providers that are administratively active *and*
    /// to models with `enabled = TRUE`, so disabling a model — even if it is
    /// the org default and its provider is still active — removes it from
    /// resolution. The provider's API-key state is *not* gated here; readiness
    /// is surfaced via the derived `healthy` field on `ModelWithProvider`,
    /// and downstream LLM calls fail loudly if the key is missing.
    pub async fn get_default_model(&self, org_id: i64) -> Result<Option<ModelWithProviderRow>> {
        let row = sqlx::query_as::<_, ModelWithProviderRow>(
            r#"
            SELECT m.id, m.org_id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_favorite, m.enabled, m.source, m.last_seen_at, m.provider_metadata, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type, p.api_key_set as provider_api_key_set, p.status as provider_status
            FROM models m
            LEFT JOIN organization_settings os ON os.org_id = m.org_id
            JOIN providers p ON m.provider_id = p.id AND p.org_id = m.org_id
            WHERE m.org_id = $1 AND m.id = COALESCE(os.default_model_id, $2)
              AND p.status = 'active' AND m.enabled = TRUE
            "#,
        )
        .bind(org_id)
        .bind(crate::platform::platform_default_model_id(org_id))
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // ============================================
    // LLM Models
    // ============================================

    pub async fn create_model(&self, org_id: i64, input: CreateModelRow) -> Result<ModelRow> {
        let capabilities_json = serde_json::to_value(&input.capabilities)?;

        let row = sqlx::query_as::<_, ModelRow>(
            r#"
            INSERT INTO models (org_id, provider_id, model_id, display_name, capabilities, is_favorite, enabled, source, provider_metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING id, org_id, provider_id, model_id, display_name, capabilities, is_favorite, enabled, source, last_seen_at, provider_metadata, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(input.provider_id)
        .bind(&input.model_id)
        .bind(&input.display_name)
        .bind(&capabilities_json)
        .bind(input.is_favorite)
        .bind(input.enabled)
        .bind(&input.source)
        .bind(&input.provider_metadata)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create or update a model with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_model_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateModelRow,
    ) -> Result<Option<ModelRow>> {
        let capabilities_json = serde_json::to_value(&input.capabilities)?;

        let row = sqlx::query_as::<_, ModelRow>(
            r#"
            INSERT INTO models (id, org_id, provider_id, model_id, display_name, capabilities, is_favorite, enabled, source, provider_metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO UPDATE SET
                display_name = EXCLUDED.display_name,
                is_favorite = EXCLUDED.is_favorite,
                enabled = EXCLUDED.enabled,
                updated_at = NOW()
            WHERE
                models.display_name IS DISTINCT FROM EXCLUDED.display_name
                OR models.is_favorite IS DISTINCT FROM EXCLUDED.is_favorite
                OR models.enabled IS DISTINCT FROM EXCLUDED.enabled
            RETURNING id, org_id, provider_id, model_id, display_name, capabilities, is_favorite, enabled, source, last_seen_at, provider_metadata, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(org_id)
        .bind(input.provider_id)
        .bind(&input.model_id)
        .bind(&input.display_name)
        .bind(&capabilities_json)
        .bind(input.is_favorite)
        .bind(input.enabled)
        .bind(&input.source)
        .bind(&input.provider_metadata)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Resolve an LLM model by UUID for use (e.g. agent execution, validation).
    ///
    /// Disabled models are filtered out: callers on this path must not be able
    /// to resolve a model that an administrator has disabled. Admin code that
    /// needs to read disabled rows (e.g. the management UI, update/delete by id)
    /// uses `get_model_with_provider` or operates via `update_model` /
    /// `delete_model` directly.
    pub async fn get_model(&self, org_id: i64, id: Uuid) -> Result<Option<ModelRow>> {
        let row = sqlx::query_as::<_, ModelRow>(
            r#"
            SELECT id, org_id, provider_id, model_id, display_name, capabilities, is_favorite, enabled, source, last_seen_at, provider_metadata, created_at, updated_at
            FROM models
            WHERE org_id = $1 AND id = $2 AND enabled = TRUE
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_model_with_provider(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<ModelWithProviderRow>> {
        let row = sqlx::query_as::<_, ModelWithProviderRow>(
            r#"
            SELECT m.id, m.org_id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_favorite, m.enabled, m.source, m.last_seen_at, m.provider_metadata, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type, p.api_key_set as provider_api_key_set, p.status as provider_status
            FROM models m
            JOIN providers p ON m.provider_id = p.id AND p.org_id = m.org_id
            WHERE m.org_id = $1 AND m.id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_models_for_provider(
        &self,
        org_id: i64,
        provider_id: Uuid,
    ) -> Result<Vec<ModelRow>> {
        let rows = sqlx::query_as::<_, ModelRow>(
            r#"
            SELECT id, org_id, provider_id, model_id, display_name, capabilities, is_favorite, enabled, source, last_seen_at, provider_metadata, created_at, updated_at
            FROM models
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

    /// List all LLM models for the org, including disabled ones.
    ///
    /// Admin/management surface. Used by the models management UI which must show
    /// disabled models so administrators can re-enable them. Resolution paths
    /// (`get_default_model`, `get_model_by_model_id`, `get_model`)
    /// enforce `enabled = TRUE`; this listing intentionally does not.
    pub async fn list_all_models(&self, org_id: i64) -> Result<Vec<ModelWithProviderRow>> {
        let rows = sqlx::query_as::<_, ModelWithProviderRow>(
            r#"
            SELECT m.id, m.org_id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_favorite, m.enabled, m.source, m.last_seen_at, m.provider_metadata, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type, p.api_key_set as provider_api_key_set, p.status as provider_status
            FROM models m
            JOIN providers p ON m.provider_id = p.id AND p.org_id = m.org_id
            WHERE p.status = 'active' AND m.org_id = $1
            ORDER BY m.enabled DESC, m.is_favorite DESC, p.name ASC, m.display_name ASC
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_model(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateModel,
    ) -> Result<Option<ModelRow>> {
        let capabilities_json = input
            .capabilities
            .map(|c| serde_json::to_value(&c))
            .transpose()?;

        let row = sqlx::query_as::<_, ModelRow>(
            r#"
            UPDATE models
            SET
                provider_id = COALESCE($3, provider_id),
                model_id = COALESCE($4, model_id),
                display_name = COALESCE($5, display_name),
                capabilities = COALESCE($6, capabilities),
                is_favorite = COALESCE($7, is_favorite),
                enabled = COALESCE($8, enabled),
                last_seen_at = COALESCE($9, last_seen_at),
                provider_metadata = COALESCE($10, provider_metadata),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
              AND (
                  $3::uuid IS NULL
                  OR EXISTS (
                      SELECT 1 FROM providers p
                      WHERE p.org_id = $1 AND p.id = $3
                  )
              )
            RETURNING id, org_id, provider_id, model_id, display_name, capabilities, is_favorite, enabled, source, last_seen_at, provider_metadata, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(input.provider_id)
        .bind(&input.model_id)
        .bind(&input.display_name)
        .bind(&capabilities_json)
        .bind(input.is_favorite)
        .bind(input.enabled)
        .bind(input.last_seen_at)
        .bind(&input.provider_metadata)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_model(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM models WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Get model by model_id string (for resolving agent model references)
    pub async fn get_model_by_model_id(
        &self,
        org_id: i64,
        model_id: &str,
    ) -> Result<Option<ModelWithProviderRow>> {
        let row = sqlx::query_as::<_, ModelWithProviderRow>(
            r#"
            SELECT m.id, m.org_id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_favorite, m.enabled, m.source, m.last_seen_at, m.provider_metadata, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type, p.api_key_set as provider_api_key_set, p.status as provider_status
            FROM models m
            JOIN providers p ON m.provider_id = p.id AND p.org_id = m.org_id
            WHERE m.model_id = $1 AND p.status = 'active' AND m.org_id = $2 AND m.enabled = TRUE
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
        actual_cost_usd: Option<f64>,
        estimated_cost_usd: Option<f64>,
        duration_ms: Option<i32>,
        finish_reason: Option<String>,
        provider_response_id: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        let (id,): (uuid::Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO llm_generations (
                org_id, session_id, turn_id, event_id, model, provider,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                actual_cost_usd, estimated_cost_usd, duration_ms, finish_reason,
                provider_response_id, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
            RETURNING id
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
        .bind(actual_cost_usd)
        .bind(estimated_cost_usd)
        .bind(duration_ms)
        .bind(&finish_reason)
        .bind(&provider_response_id)
        .bind(created_at)
        .fetch_one(&self.pool)
        .await?;

        // Reporting outbox enqueue is best-effort (see knowledge/evaluation/reporting.md).
        // The canonical llm_generations row is already durable; no reconciler
        // exists yet (tracked separately), so on failure the corresponding
        // fact will remain stale until reconciliation lands.
        if let Err(e) = self
            .enqueue_reporting_outbox(
                org_id,
                "llm_generation",
                &id.to_string(),
                Some(&id.to_string()),
                "llm_generation_projection",
            )
            .await
        {
            warn!(
                generation_id = %id,
                org_id,
                error = %e,
                "reporting outbox enqueue failed for llm_generation; projection may remain stale until reconciliation lands"
            );
        }

        Ok(())
    }

    /// Returns rows that have a `provider_response_id` but no `reconciled_at`
    /// and are ready to retry, ordered oldest-first. Failed rows are delayed by
    /// `mark_llm_generation_reconciliation_failed` so they cannot monopolize
    /// every batch.
    pub async fn list_unreconciled_llm_generations(
        &self,
        provider: &str,
        limit: i64,
    ) -> Result<Vec<UnreconciledGeneration>> {
        let rows = sqlx::query_as::<_, UnreconciledGeneration>(
            r#"
            SELECT id, org_id, provider_response_id
            FROM   llm_generations
            WHERE  provider = $1
              AND  provider_response_id IS NOT NULL
              AND  reconciled_at IS NULL
              AND  (reconcile_after IS NULL OR reconcile_after <= NOW())
            ORDER  BY created_at ASC
            LIMIT  $2
            "#,
        )
        .bind(provider)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Records a failed reconciliation attempt and delays the row before it is
    /// eligible for another batch. Idempotent with successful reconciliation:
    /// a row already reconciled (`reconciled_at IS NOT NULL`) is not modified.
    pub async fn mark_llm_generation_reconciliation_failed(
        &self,
        id: Uuid,
        retry_after_seconds: i32,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE llm_generations
            SET
                reconciliation_attempts = reconciliation_attempts + 1,
                reconcile_after = NOW() + ($2 * INTERVAL '1 second')
            WHERE id = $1
              AND reconciled_at IS NULL
            "#,
        )
        .bind(id)
        .bind(retry_after_seconds)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Updates a generation row with authoritative data from the provider and
    /// stamps `reconciled_at`. Idempotent: a row already reconciled (`reconciled_at
    /// IS NOT NULL`) is not modified.
    pub async fn reconcile_llm_generation(
        &self,
        id: Uuid,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        actual_cost_usd: Option<f64>,
        reconciled_provider: Option<&str>,
        reconciled_model: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE llm_generations
            SET
                input_tokens          = COALESCE($2, input_tokens),
                output_tokens         = COALESCE($3, output_tokens),
                actual_cost_usd       = COALESCE($4, actual_cost_usd),
                provider              = COALESCE($5, provider),
                model                 = COALESCE($6, model),
                reconcile_after       = NULL,
                reconciled_at         = NOW()
            WHERE id = $1
              AND reconciled_at IS NULL
            "#,
        )
        .bind(id)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(actual_cost_usd)
        .bind(reconciled_provider)
        .bind(reconciled_model)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn increment_session_usage(
        &self,
        session_id: Uuid,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        actual_cost_usd: f64,
        estimated_cost_usd: f64,
        cost_usd: f64,
    ) -> Result<()> {
        let row: (i64, uuid::Uuid, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
            r#"
            UPDATE sessions
            SET
                total_input_tokens = total_input_tokens + $2,
                total_output_tokens = total_output_tokens + $3,
                total_cache_read_tokens = total_cache_read_tokens + $4,
                total_cache_creation_tokens = total_cache_creation_tokens + $5,
                total_actual_cost_usd = total_actual_cost_usd + $6,
                total_estimated_cost_usd = total_estimated_cost_usd + $7,
                total_cost_usd = total_cost_usd + $8,
                updated_at = NOW()
            WHERE id = $1
            RETURNING org_id, id, updated_at
            "#,
        )
        .bind(session_id)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cache_read_tokens)
        .bind(cache_creation_tokens)
        .bind(actual_cost_usd)
        .bind(estimated_cost_usd)
        .bind(cost_usd)
        .fetch_one(&self.pool)
        .await?;

        // Reporting outbox enqueue is best-effort (see knowledge/evaluation/reporting.md).
        // The canonical session totals are already committed; no reconciler
        // exists yet (tracked separately), so on failure the corresponding
        // fact will remain stale until reconciliation lands or the session
        // totals are updated again and the next enqueue succeeds.
        if let Err(e) = self
            .enqueue_reporting_outbox(
                row.0,
                "session",
                &row.1.to_string(),
                Some(&row.2.to_rfc3339()),
                "session_snapshot",
            )
            .await
        {
            warn!(
                session_id = %row.1,
                org_id = row.0,
                error = %e,
                "reporting outbox enqueue failed for session snapshot; projection may remain stale until reconciliation lands or the source row is updated again"
            );
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn increment_agent_usage(
        &self,
        agent_id: Uuid,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_creation_tokens: i64,
        actual_cost_usd: f64,
        estimated_cost_usd: f64,
        cost_usd: f64,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE agents
            SET
                total_input_tokens = total_input_tokens + $2,
                total_output_tokens = total_output_tokens + $3,
                total_cache_read_tokens = total_cache_read_tokens + $4,
                total_cache_creation_tokens = total_cache_creation_tokens + $5,
                total_actual_cost_usd = total_actual_cost_usd + $6,
                total_estimated_cost_usd = total_estimated_cost_usd + $7,
                total_cost_usd = total_cost_usd + $8
            WHERE id = $1
            "#,
        )
        .bind(agent_id)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cache_read_tokens)
        .bind(cache_creation_tokens)
        .bind(actual_cost_usd)
        .bind(estimated_cost_usd)
        .bind(cost_usd)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

// Repository layer for database operations
// Note: Full implementations will be added incrementally across milestones

use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::*;

#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create database connection from URL
    pub async fn from_url(database_url: &str) -> Result<Self> {
        let pool = PgPool::connect(database_url).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    // Users (for future auth implementation)
    pub async fn create_user(&self, _input: CreateUser) -> Result<UserRow> {
        todo!("Implement when auth is added")
    }

    pub async fn get_user_by_email(&self, _email: &str) -> Result<Option<UserRow>> {
        todo!("Implement when auth is added")
    }

    pub async fn get_user(&self, _id: Uuid) -> Result<Option<UserRow>> {
        todo!("Implement when auth is added")
    }

    // Sessions (for future auth implementation)
    pub async fn create_session(&self, _input: CreateSession) -> Result<SessionRow> {
        todo!("Implement when auth is added")
    }

    pub async fn get_session_by_token(&self, _token: &str) -> Result<Option<SessionRow>> {
        todo!("Implement when auth is added")
    }

    pub async fn delete_session(&self, _token: &str) -> Result<()> {
        todo!("Implement when auth is added")
    }

    // Agents
    pub async fn create_agent(&self, input: CreateAgent) -> Result<AgentRow> {
        let definition_json = serde_json::to_value(&input.definition)?;

        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            INSERT INTO agents (name, description, default_model_id, definition, status)
            VALUES ($1, $2, $3, $4, 'active')
            RETURNING id, name, description, default_model_id, definition, status, created_at, updated_at
            "#,
        )
        .bind(input.name)
        .bind(input.description)
        .bind(input.default_model_id)
        .bind(definition_json)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent(&self, id: Uuid) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, name, description, default_model_id, definition, status, created_at, updated_at
            FROM agents
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentRow>> {
        let rows = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, name, description, default_model_id, definition, status, created_at, updated_at
            FROM agents
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_agent(&self, id: Uuid, input: UpdateAgent) -> Result<Option<AgentRow>> {
        let definition_json = input
            .definition
            .map(|d| serde_json::to_value(&d))
            .transpose()?;

        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            UPDATE agents
            SET
                name = COALESCE($2, name),
                description = COALESCE($3, description),
                default_model_id = COALESCE($4, default_model_id),
                definition = COALESCE($5, definition),
                status = COALESCE($6, status),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, name, description, default_model_id, definition, status, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(input.name)
        .bind(input.description)
        .bind(input.default_model_id)
        .bind(definition_json)
        .bind(input.status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // Threads
    pub async fn create_thread(&self, _input: CreateThread) -> Result<ThreadRow> {
        let row = sqlx::query_as::<_, ThreadRow>(
            r#"
            INSERT INTO threads DEFAULT VALUES
            RETURNING id, created_at
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_thread(&self, id: Uuid) -> Result<Option<ThreadRow>> {
        let row = sqlx::query_as::<_, ThreadRow>(
            r#"
            SELECT id, created_at
            FROM threads
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // Messages
    pub async fn create_message(&self, input: CreateMessage) -> Result<MessageRow> {
        let metadata_json = input.metadata.map(serde_json::to_value).transpose()?;

        let row = sqlx::query_as::<_, MessageRow>(
            r#"
            INSERT INTO messages (thread_id, role, content, metadata)
            VALUES ($1, $2, $3, $4)
            RETURNING id, thread_id, role, content, metadata, created_at
            "#,
        )
        .bind(input.thread_id)
        .bind(input.role)
        .bind(input.content)
        .bind(metadata_json)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_messages(&self, thread_id: Uuid) -> Result<Vec<MessageRow>> {
        let rows = sqlx::query_as::<_, MessageRow>(
            r#"
            SELECT id, thread_id, role, content, metadata, created_at
            FROM messages
            WHERE thread_id = $1
            ORDER BY created_at ASC
            "#,
        )
        .bind(thread_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // Runs
    pub async fn create_run(&self, input: CreateRun) -> Result<RunRow> {
        let row = sqlx::query_as::<_, RunRow>(
            r#"
            INSERT INTO runs (agent_id, thread_id, status)
            VALUES ($1, $2, 'pending')
            RETURNING id, agent_id, thread_id, status, temporal_workflow_id, temporal_run_id, created_at, started_at, finished_at
            "#,
        )
        .bind(input.agent_id)
        .bind(input.thread_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_run(&self, id: Uuid) -> Result<Option<RunRow>> {
        let row = sqlx::query_as::<_, RunRow>(
            r#"
            SELECT id, agent_id, thread_id, status, temporal_workflow_id, temporal_run_id, created_at, started_at, finished_at
            FROM runs
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List runs with optional filtering
    pub async fn list_runs(
        &self,
        status: Option<&str>,
        agent_id: Option<Uuid>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<RunRow>> {
        let rows = sqlx::query_as::<_, RunRow>(
            r#"
            SELECT id, agent_id, thread_id, status, temporal_workflow_id, temporal_run_id, created_at, started_at, finished_at
            FROM runs
            WHERE ($1::text IS NULL OR status = $1)
              AND ($2::uuid IS NULL OR agent_id = $2)
            ORDER BY created_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(status)
        .bind(agent_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_run(&self, id: Uuid, input: UpdateRun) -> Result<Option<RunRow>> {
        let row = sqlx::query_as::<_, RunRow>(
            r#"
            UPDATE runs
            SET
                status = COALESCE($2, status),
                temporal_workflow_id = COALESCE($3, temporal_workflow_id),
                temporal_run_id = COALESCE($4, temporal_run_id),
                started_at = COALESCE($5, started_at),
                finished_at = COALESCE($6, finished_at)
            WHERE id = $1
            RETURNING id, agent_id, thread_id, status, temporal_workflow_id, temporal_run_id, created_at, started_at, finished_at
            "#,
        )
        .bind(id)
        .bind(input.status)
        .bind(input.temporal_workflow_id)
        .bind(input.temporal_run_id)
        .bind(input.started_at)
        .bind(input.finished_at)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // Actions
    pub async fn create_action(&self, _input: CreateAction) -> Result<ActionRow> {
        todo!("Implement in M7")
    }

    pub async fn list_actions(&self, _run_id: Uuid) -> Result<Vec<ActionRow>> {
        todo!("Implement in M7")
    }

    // Run events
    pub async fn create_run_event(&self, _input: CreateRunEvent) -> Result<RunEventRow> {
        todo!("Implement in M5")
    }

    /// List run events, optionally filtering by sequence number
    /// Returns events ordered by sequence_number ASC
    pub async fn list_run_events(
        &self,
        run_id: Uuid,
        since_sequence: Option<i64>,
    ) -> Result<Vec<RunEventRow>> {
        let rows = if let Some(seq) = since_sequence {
            sqlx::query_as::<_, RunEventRow>(
                r#"
                SELECT id, run_id, sequence_number, event_type, event_data, created_at
                FROM run_events
                WHERE run_id = $1 AND sequence_number > $2
                ORDER BY sequence_number ASC
                "#,
            )
            .bind(run_id)
            .bind(seq)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, RunEventRow>(
                r#"
                SELECT id, run_id, sequence_number, event_type, event_data, created_at
                FROM run_events
                WHERE run_id = $1
                ORDER BY sequence_number ASC
                "#,
            )
            .bind(run_id)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }

    // LLM Providers

    pub async fn create_llm_provider(&self, input: CreateLlmProvider) -> Result<LlmProviderRow> {
        let api_key_set = input.api_key_encrypted.is_some();

        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            INSERT INTO llm_providers (name, provider_type, base_url, api_key_encrypted, api_key_set, is_default)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, name, provider_type, base_url, api_key_encrypted, api_key_set, is_default, status, created_at, updated_at
            "#,
        )
        .bind(&input.name)
        .bind(&input.provider_type)
        .bind(&input.base_url)
        .bind(&input.api_key_encrypted)
        .bind(api_key_set)
        .bind(input.is_default)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_llm_provider(&self, id: Uuid) -> Result<Option<LlmProviderRow>> {
        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            SELECT id, name, provider_type, base_url, api_key_encrypted, api_key_set, is_default, status, created_at, updated_at
            FROM llm_providers
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_llm_providers(&self) -> Result<Vec<LlmProviderRow>> {
        let rows = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            SELECT id, name, provider_type, base_url, api_key_encrypted, api_key_set, is_default, status, created_at, updated_at
            FROM llm_providers
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_llm_provider(
        &self,
        id: Uuid,
        input: UpdateLlmProvider,
    ) -> Result<Option<LlmProviderRow>> {
        // If updating api_key, also update api_key_set
        let api_key_set = input.api_key_encrypted.as_ref().map(|_| true);

        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            UPDATE llm_providers
            SET
                name = COALESCE($2, name),
                provider_type = COALESCE($3, provider_type),
                base_url = COALESCE($4, base_url),
                api_key_encrypted = COALESCE($5, api_key_encrypted),
                api_key_set = COALESCE($6, api_key_set),
                is_default = COALESCE($7, is_default),
                status = COALESCE($8, status),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, name, provider_type, base_url, api_key_encrypted, api_key_set, is_default, status, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&input.name)
        .bind(&input.provider_type)
        .bind(&input.base_url)
        .bind(&input.api_key_encrypted)
        .bind(api_key_set)
        .bind(input.is_default)
        .bind(&input.status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_llm_provider(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM llm_providers WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn get_default_llm_provider(&self) -> Result<Option<LlmProviderRow>> {
        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            SELECT id, name, provider_type, base_url, api_key_encrypted, api_key_set, is_default, status, created_at, updated_at
            FROM llm_providers
            WHERE is_default = TRUE AND status = 'active'
            "#,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    // LLM Models

    pub async fn create_llm_model(&self, input: CreateLlmModel) -> Result<LlmModelRow> {
        let capabilities_json = serde_json::to_value(&input.capabilities)?;

        let row = sqlx::query_as::<_, LlmModelRow>(
            r#"
            INSERT INTO llm_models (provider_id, model_id, display_name, capabilities, context_window, is_default)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, provider_id, model_id, display_name, capabilities, context_window, is_default, status, created_at, updated_at
            "#,
        )
        .bind(input.provider_id)
        .bind(&input.model_id)
        .bind(&input.display_name)
        .bind(&capabilities_json)
        .bind(input.context_window)
        .bind(input.is_default)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_llm_model(&self, id: Uuid) -> Result<Option<LlmModelRow>> {
        let row = sqlx::query_as::<_, LlmModelRow>(
            r#"
            SELECT id, provider_id, model_id, display_name, capabilities, context_window, is_default, status, created_at, updated_at
            FROM llm_models
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_llm_models_for_provider(
        &self,
        provider_id: Uuid,
    ) -> Result<Vec<LlmModelRow>> {
        let rows = sqlx::query_as::<_, LlmModelRow>(
            r#"
            SELECT id, provider_id, model_id, display_name, capabilities, context_window, is_default, status, created_at, updated_at
            FROM llm_models
            WHERE provider_id = $1
            ORDER BY display_name ASC
            "#,
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn list_all_llm_models(&self) -> Result<Vec<LlmModelWithProviderRow>> {
        let rows = sqlx::query_as::<_, LlmModelWithProviderRow>(
            r#"
            SELECT m.id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.context_window, m.is_default, m.status, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type
            FROM llm_models m
            JOIN llm_providers p ON m.provider_id = p.id
            WHERE m.status = 'active' AND p.status = 'active'
            ORDER BY p.name ASC, m.display_name ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_llm_model(
        &self,
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
                model_id = COALESCE($2, model_id),
                display_name = COALESCE($3, display_name),
                capabilities = COALESCE($4, capabilities),
                context_window = COALESCE($5, context_window),
                is_default = COALESCE($6, is_default),
                status = COALESCE($7, status),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, provider_id, model_id, display_name, capabilities, context_window, is_default, status, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&input.model_id)
        .bind(&input.display_name)
        .bind(&capabilities_json)
        .bind(input.context_window)
        .bind(input.is_default)
        .bind(&input.status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_llm_model(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM llm_models WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Get model by model_id string (for resolving agent model references)
    pub async fn get_llm_model_by_model_id(
        &self,
        model_id: &str,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let row = sqlx::query_as::<_, LlmModelWithProviderRow>(
            r#"
            SELECT m.id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.context_window, m.is_default, m.status, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type
            FROM llm_models m
            JOIN llm_providers p ON m.provider_id = p.id
            WHERE m.model_id = $1 AND m.status = 'active' AND p.status = 'active'
            "#,
        )
        .bind(model_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }
}

// Repository layer for database operations
// M2 Revised: Agent/Session/Messages/Events model

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

    // ============================================
    // Users (for future auth implementation)
    // ============================================

    pub async fn create_user(&self, _input: CreateUser) -> Result<UserRow> {
        todo!("Implement when auth is added")
    }

    pub async fn get_user_by_email(&self, _email: &str) -> Result<Option<UserRow>> {
        todo!("Implement when auth is added")
    }

    pub async fn get_user(&self, _id: Uuid) -> Result<Option<UserRow>> {
        todo!("Implement when auth is added")
    }

    // ============================================
    // Agents (configuration for agentic loop)
    // ============================================

    pub async fn create_agent(&self, input: CreateAgent) -> Result<AgentRow> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            INSERT INTO agents (slug, name, description, system_prompt, default_model_id, tags, status)
            VALUES ($1, $2, $3, $4, $5, $6, 'active')
            RETURNING id, slug, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at
            "#,
        )
        .bind(&input.slug)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id)
        .bind(&input.tags)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent(&self, id: Uuid) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, slug, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at
            FROM agents
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent_by_slug(&self, slug: &str) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, slug, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at
            FROM agents
            WHERE slug = $1
            "#,
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentRow>> {
        let rows = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, slug, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at
            FROM agents
            WHERE status = 'active'
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_agent(&self, id: Uuid, input: UpdateAgent) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            UPDATE agents
            SET
                slug = COALESCE($2, slug),
                name = COALESCE($3, name),
                description = COALESCE($4, description),
                system_prompt = COALESCE($5, system_prompt),
                default_model_id = COALESCE($6, default_model_id),
                tags = COALESCE($7, tags),
                status = COALESCE($8, status),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, slug, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&input.slug)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id)
        .bind(&input.tags)
        .bind(&input.status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_agent(&self, id: Uuid) -> Result<bool> {
        // Archive instead of hard delete
        let result = sqlx::query(
            r#"
            UPDATE agents
            SET status = 'archived', updated_at = NOW()
            WHERE id = $1 AND status = 'active'
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Sessions (instance of agentic loop)
    // ============================================

    pub async fn create_session(&self, input: CreateSession) -> Result<SessionRow> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            INSERT INTO sessions (agent_id, title, tags, model_id, status)
            VALUES ($1, $2, $3, $4, 'pending')
            RETURNING id, agent_id, title, tags, model_id, status, created_at, started_at, finished_at
            "#,
        )
        .bind(input.agent_id)
        .bind(&input.title)
        .bind(&input.tags)
        .bind(input.model_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_session(&self, id: Uuid) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, agent_id, title, tags, model_id, status, created_at, started_at, finished_at
            FROM sessions
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_sessions(&self, agent_id: Uuid) -> Result<Vec<SessionRow>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, agent_id, title, tags, model_id, status, created_at, started_at, finished_at
            FROM sessions
            WHERE agent_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_session(
        &self,
        id: Uuid,
        input: UpdateSession,
    ) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            UPDATE sessions
            SET
                title = COALESCE($2, title),
                tags = COALESCE($3, tags),
                model_id = COALESCE($4, model_id),
                status = COALESCE($5, status),
                started_at = COALESCE($6, started_at),
                finished_at = COALESCE($7, finished_at)
            WHERE id = $1
            RETURNING id, agent_id, title, tags, model_id, status, created_at, started_at, finished_at
            "#,
        )
        .bind(id)
        .bind(&input.title)
        .bind(&input.tags)
        .bind(input.model_id)
        .bind(&input.status)
        .bind(input.started_at)
        .bind(input.finished_at)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_session(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM sessions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Messages (PRIMARY conversation data)
    // ============================================

    pub async fn create_message(&self, input: CreateMessage) -> Result<MessageRow> {
        // Get next sequence number for this session
        let row = sqlx::query_as::<_, MessageRow>(
            r#"
            INSERT INTO messages (session_id, sequence, role, content, tool_call_id)
            VALUES ($1, COALESCE((SELECT MAX(sequence) + 1 FROM messages WHERE session_id = $1), 1), $2, $3, $4)
            RETURNING id, session_id, sequence, role, content, tool_call_id, created_at
            "#,
        )
        .bind(input.session_id)
        .bind(&input.role)
        .bind(&input.content)
        .bind(&input.tool_call_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_message(&self, id: Uuid) -> Result<Option<MessageRow>> {
        let row = sqlx::query_as::<_, MessageRow>(
            r#"
            SELECT id, session_id, sequence, role, content, tool_call_id, created_at
            FROM messages
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_messages(&self, session_id: Uuid) -> Result<Vec<MessageRow>> {
        let rows = sqlx::query_as::<_, MessageRow>(
            r#"
            SELECT id, session_id, sequence, role, content, tool_call_id, created_at
            FROM messages
            WHERE session_id = $1
            ORDER BY sequence ASC
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// List messages by role
    pub async fn list_messages_by_role(
        &self,
        session_id: Uuid,
        role: &str,
    ) -> Result<Vec<MessageRow>> {
        let rows = sqlx::query_as::<_, MessageRow>(
            r#"
            SELECT id, session_id, sequence, role, content, tool_call_id, created_at
            FROM messages
            WHERE session_id = $1 AND role = $2
            ORDER BY sequence ASC
            "#,
        )
        .bind(session_id)
        .bind(role)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // ============================================
    // Events (SSE notification stream for UI)
    // ============================================

    pub async fn create_event(&self, input: CreateEvent) -> Result<EventRow> {
        // Get next sequence number for this session
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            INSERT INTO events (session_id, sequence, event_type, data)
            VALUES ($1, COALESCE((SELECT MAX(sequence) + 1 FROM events WHERE session_id = $1), 1), $2, $3)
            RETURNING id, session_id, sequence, event_type, data, created_at
            "#,
        )
        .bind(input.session_id)
        .bind(&input.event_type)
        .bind(&input.data)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_events(
        &self,
        session_id: Uuid,
        since_sequence: Option<i32>,
    ) -> Result<Vec<EventRow>> {
        let rows = if let Some(seq) = since_sequence {
            sqlx::query_as::<_, EventRow>(
                r#"
                SELECT id, session_id, sequence, event_type, data, created_at
                FROM events
                WHERE session_id = $1 AND sequence > $2
                ORDER BY sequence ASC
                "#,
            )
            .bind(session_id)
            .bind(seq)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, EventRow>(
                r#"
                SELECT id, session_id, sequence, event_type, data, created_at
                FROM events
                WHERE session_id = $1
                ORDER BY sequence ASC
                "#,
            )
            .bind(session_id)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }

    // ============================================
    // LLM Providers
    // ============================================

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

    // ============================================
    // LLM Models
    // ============================================

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

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
    // Users
    // ============================================

    pub async fn create_user(&self, input: CreateUser) -> Result<UserRow> {
        let roles_json = serde_json::to_value(&input.roles)?;

        let row = sqlx::query_as::<_, UserRow>(
            r#"
            INSERT INTO users (email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at
            "#,
        )
        .bind(&input.email)
        .bind(&input.name)
        .bind(&input.avatar_url)
        .bind(&roles_json)
        .bind(&input.password_hash)
        .bind(input.email_verified)
        .bind(&input.auth_provider)
        .bind(&input.auth_provider_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<UserRow>> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at
            FROM users
            WHERE email = $1
            "#,
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_user(&self, id: Uuid) -> Result<Option<UserRow>> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at
            FROM users
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_user_by_oauth(
        &self,
        provider: &str,
        provider_id: &str,
    ) -> Result<Option<UserRow>> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at
            FROM users
            WHERE auth_provider = $1 AND auth_provider_id = $2
            "#,
        )
        .bind(provider)
        .bind(provider_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn update_user(&self, id: Uuid, input: UpdateUser) -> Result<Option<UserRow>> {
        let roles_json = input.roles.map(|r| serde_json::to_value(&r)).transpose()?;

        let row = sqlx::query_as::<_, UserRow>(
            r#"
            UPDATE users
            SET
                name = COALESCE($2, name),
                avatar_url = COALESCE($3, avatar_url),
                roles = COALESCE($4, roles),
                password_hash = COALESCE($5, password_hash),
                email_verified = COALESCE($6, email_verified),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&input.name)
        .bind(&input.avatar_url)
        .bind(&roles_json)
        .bind(&input.password_hash)
        .bind(input.email_verified)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List all users with optional search query
    /// Search matches name or email (case-insensitive, partial match)
    pub async fn list_users(&self, search: Option<&str>) -> Result<Vec<UserRow>> {
        let rows = match search {
            Some(query) if !query.trim().is_empty() => {
                let search_pattern = format!("%{}%", query.trim().to_lowercase());
                sqlx::query_as::<_, UserRow>(
                    r#"
                    SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at
                    FROM users
                    WHERE LOWER(name) LIKE $1 OR LOWER(email) LIKE $1
                    ORDER BY created_at DESC
                    "#,
                )
                .bind(&search_pattern)
                .fetch_all(&self.pool)
                .await?
            }
            _ => {
                sqlx::query_as::<_, UserRow>(
                    r#"
                    SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at
                    FROM users
                    ORDER BY created_at DESC
                    "#,
                )
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(rows)
    }

    // ============================================
    // API Keys
    // ============================================

    pub async fn create_api_key(&self, input: CreateApiKey) -> Result<ApiKeyRow> {
        let scopes_json = serde_json::to_value(&input.scopes)?;

        let row = sqlx::query_as::<_, ApiKeyRow>(
            r#"
            INSERT INTO api_keys (user_id, name, key_hash, key_prefix, scopes, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, user_id, name, key_hash, key_prefix, scopes, expires_at, last_used_at, created_at
            "#,
        )
        .bind(input.user_id)
        .bind(&input.name)
        .bind(&input.key_hash)
        .bind(&input.key_prefix)
        .bind(&scopes_json)
        .bind(input.expires_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_api_key_by_hash(&self, key_hash: &str) -> Result<Option<ApiKeyRow>> {
        let row = sqlx::query_as::<_, ApiKeyRow>(
            r#"
            SELECT id, user_id, name, key_hash, key_prefix, scopes, expires_at, last_used_at, created_at
            FROM api_keys
            WHERE key_hash = $1
            "#,
        )
        .bind(key_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_api_keys_for_user(&self, user_id: Uuid) -> Result<Vec<ApiKeyRow>> {
        let rows = sqlx::query_as::<_, ApiKeyRow>(
            r#"
            SELECT id, user_id, name, key_hash, key_prefix, scopes, expires_at, last_used_at, created_at
            FROM api_keys
            WHERE user_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_api_key_last_used(&self, id: Uuid) -> Result<()> {
        sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn delete_api_key(&self, id: Uuid, user_id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM api_keys WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Refresh Tokens
    // ============================================

    pub async fn create_refresh_token(&self, input: CreateRefreshToken) -> Result<RefreshTokenRow> {
        let row = sqlx::query_as::<_, RefreshTokenRow>(
            r#"
            INSERT INTO refresh_tokens (user_id, token_hash, expires_at)
            VALUES ($1, $2, $3)
            RETURNING id, user_id, token_hash, expires_at, created_at
            "#,
        )
        .bind(input.user_id)
        .bind(&input.token_hash)
        .bind(input.expires_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RefreshTokenRow>> {
        let row = sqlx::query_as::<_, RefreshTokenRow>(
            r#"
            SELECT id, user_id, token_hash, expires_at, created_at
            FROM refresh_tokens
            WHERE token_hash = $1
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_refresh_token(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM refresh_tokens WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_expired_refresh_tokens(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM refresh_tokens WHERE expires_at < NOW()")
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    pub async fn delete_user_refresh_tokens(&self, user_id: Uuid) -> Result<u64> {
        let result = sqlx::query("DELETE FROM refresh_tokens WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    // ============================================
    // Agents (configuration for agentic loop)
    // ============================================

    pub async fn create_agent(&self, input: CreateAgent) -> Result<AgentRow> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            INSERT INTO agents (name, description, system_prompt, default_model_id, tags, status)
            VALUES ($1, $2, $3, $4, $5, 'active')
            RETURNING id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at
            "#,
        )
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
            SELECT id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at
            FROM agents
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent_by_name(&self, name: &str) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at
            FROM agents
            WHERE name = $1
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_agents(&self) -> Result<Vec<AgentRow>> {
        let rows = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at
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
                name = COALESCE($2, name),
                description = COALESCE($3, description),
                system_prompt = COALESCE($4, system_prompt),
                default_model_id = COALESCE($5, default_model_id),
                tags = COALESCE($6, tags),
                status = COALESCE($7, status),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at
            "#,
        )
        .bind(id)
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

    // ============================================
    // Agent Capabilities
    // ============================================

    /// Get capabilities for an agent, ordered by position
    pub async fn get_agent_capabilities(&self, agent_id: Uuid) -> Result<Vec<AgentCapabilityRow>> {
        let rows = sqlx::query_as::<_, AgentCapabilityRow>(
            r#"
            SELECT id, agent_id, capability_id, position, created_at
            FROM agent_capabilities
            WHERE agent_id = $1
            ORDER BY position ASC
            "#,
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Set capabilities for an agent (replaces existing capabilities)
    /// capabilities: list of (capability_id, position) tuples
    pub async fn set_agent_capabilities(
        &self,
        agent_id: Uuid,
        capabilities: Vec<(String, i32)>,
    ) -> Result<Vec<AgentCapabilityRow>> {
        // Start a transaction
        let mut tx = self.pool.begin().await?;

        // Delete existing capabilities for this agent
        sqlx::query("DELETE FROM agent_capabilities WHERE agent_id = $1")
            .bind(agent_id)
            .execute(&mut *tx)
            .await?;

        // Insert new capabilities
        for (capability_id, position) in &capabilities {
            sqlx::query(
                r#"
                INSERT INTO agent_capabilities (agent_id, capability_id, position)
                VALUES ($1, $2, $3)
                "#,
            )
            .bind(agent_id)
            .bind(capability_id)
            .bind(position)
            .execute(&mut *tx)
            .await?;
        }

        // Commit transaction
        tx.commit().await?;

        // Return the new capabilities
        self.get_agent_capabilities(agent_id).await
    }

    /// Add a single capability to an agent
    pub async fn add_agent_capability(
        &self,
        input: CreateAgentCapability,
    ) -> Result<AgentCapabilityRow> {
        let row = sqlx::query_as::<_, AgentCapabilityRow>(
            r#"
            INSERT INTO agent_capabilities (agent_id, capability_id, position)
            VALUES ($1, $2, $3)
            RETURNING id, agent_id, capability_id, position, created_at
            "#,
        )
        .bind(input.agent_id)
        .bind(&input.capability_id)
        .bind(input.position)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Remove a capability from an agent
    pub async fn remove_agent_capability(
        &self,
        agent_id: Uuid,
        capability_id: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM agent_capabilities WHERE agent_id = $1 AND capability_id = $2",
        )
        .bind(agent_id)
        .bind(capability_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}

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

    pub async fn create_user(&self, input: CreateUserRow) -> Result<UserRow> {
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

    pub async fn create_api_key(&self, input: CreateApiKeyRow) -> Result<ApiKeyRow> {
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

    pub async fn create_refresh_token(
        &self,
        input: CreateRefreshTokenRow,
    ) -> Result<RefreshTokenRow> {
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

    pub async fn create_agent(&self, input: CreateAgentRow) -> Result<AgentRow> {
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

    pub async fn create_session(&self, input: CreateSessionRow) -> Result<SessionRow> {
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

    pub async fn create_message(&self, input: CreateMessageRow) -> Result<MessageRow> {
        // Serialize content, controls, and metadata to JSON for storage
        let content_json = serde_json::to_value(&input.content)?;
        let controls_json = input.controls.map(serde_json::to_value).transpose()?;
        let metadata_json = input.metadata.map(serde_json::to_value).transpose()?;

        // Get next sequence number for this session
        let row = sqlx::query_as::<_, MessageRow>(
            r#"
            INSERT INTO messages (session_id, sequence, role, content, controls, metadata, tags)
            VALUES ($1, COALESCE((SELECT MAX(sequence) + 1 FROM messages WHERE session_id = $1), 1), $2, $3, $4, $5, $6)
            RETURNING id, session_id, sequence, role, content, controls, metadata, tags, created_at
            "#,
        )
        .bind(input.session_id)
        .bind(&input.role)
        .bind(&content_json)
        .bind(&controls_json)
        .bind(&metadata_json)
        .bind(&input.tags)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_message(&self, id: Uuid) -> Result<Option<MessageRow>> {
        let row = sqlx::query_as::<_, MessageRow>(
            r#"
            SELECT id, session_id, sequence, role, content, controls, metadata, tags, created_at
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
            SELECT id, session_id, sequence, role, content, controls, metadata, tags, created_at
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
            SELECT id, session_id, sequence, role, content, controls, metadata, tags, created_at
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

    pub async fn create_event(&self, input: CreateEventRow) -> Result<EventRow> {
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

    pub async fn create_llm_provider(&self, input: CreateLlmProviderRow) -> Result<LlmProviderRow> {
        let api_key_set = input.api_key_encrypted.is_some();
        let settings = input.settings.unwrap_or(serde_json::json!({}));

        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            INSERT INTO llm_providers (name, provider_type, base_url, api_key_encrypted, api_key_set, is_default, settings)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, name, provider_type, base_url, api_key_encrypted, api_key_set, is_default, status, settings, created_at, updated_at
            "#,
        )
        .bind(&input.name)
        .bind(&input.provider_type)
        .bind(&input.base_url)
        .bind(&input.api_key_encrypted)
        .bind(api_key_set)
        .bind(input.is_default)
        .bind(&settings)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_llm_provider(&self, id: Uuid) -> Result<Option<LlmProviderRow>> {
        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            SELECT id, name, provider_type, base_url, api_key_encrypted, api_key_set, is_default, status, settings, created_at, updated_at
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
            SELECT id, name, provider_type, base_url, api_key_encrypted, api_key_set, is_default, status, settings, created_at, updated_at
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
                settings = COALESCE($9, settings),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, name, provider_type, base_url, api_key_encrypted, api_key_set, is_default, status, settings, created_at, updated_at
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
        .bind(&input.settings)
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
            SELECT id, name, provider_type, base_url, api_key_encrypted, api_key_set, is_default, status, settings, created_at, updated_at
            FROM llm_providers
            WHERE is_default = TRUE AND status = 'active'
            "#,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get a provider with its decrypted API key for use in LLM calls.
    /// This should only be called by worker activities that need to make LLM requests.
    pub fn get_provider_with_api_key(
        &self,
        provider: &LlmProviderRow,
        encryption: &crate::EncryptionService,
    ) -> Result<LlmProviderWithApiKey> {
        let api_key = if let Some(ref encrypted) = provider.api_key_encrypted {
            Some(encryption.decrypt_to_string(encrypted)?)
        } else {
            None
        };

        // Convert settings from sqlx JsonValue to serde_json::Value
        let settings: serde_json::Value =
            serde_json::from_str(&provider.settings.to_string()).unwrap_or_default();

        Ok(LlmProviderWithApiKey {
            id: provider.id,
            name: provider.name.clone(),
            provider_type: provider.provider_type.clone(),
            base_url: provider.base_url.clone(),
            api_key,
            settings,
        })
    }

    // ============================================
    // LLM Models
    // ============================================

    pub async fn create_llm_model(&self, input: CreateLlmModelRow) -> Result<LlmModelRow> {
        let capabilities_json = serde_json::to_value(&input.capabilities)?;

        let row = sqlx::query_as::<_, LlmModelRow>(
            r#"
            INSERT INTO llm_models (provider_id, model_id, display_name, capabilities, is_default)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, provider_id, model_id, display_name, capabilities, is_default, status, created_at, updated_at
            "#,
        )
        .bind(input.provider_id)
        .bind(&input.model_id)
        .bind(&input.display_name)
        .bind(&capabilities_json)
        .bind(input.is_default)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_llm_model(&self, id: Uuid) -> Result<Option<LlmModelRow>> {
        let row = sqlx::query_as::<_, LlmModelRow>(
            r#"
            SELECT id, provider_id, model_id, display_name, capabilities, is_default, status, created_at, updated_at
            FROM llm_models
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_llm_model_with_provider(
        &self,
        id: Uuid,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let row = sqlx::query_as::<_, LlmModelWithProviderRow>(
            r#"
            SELECT m.id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.context_window, m.is_default, m.status, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type
            FROM llm_models m
            JOIN llm_providers p ON m.provider_id = p.id
            WHERE m.id = $1
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
            SELECT id, provider_id, model_id, display_name, capabilities, is_default, status, created_at, updated_at
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
            SELECT m.id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_default, m.status, m.created_at, m.updated_at,
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
                is_default = COALESCE($5, is_default),
                status = COALESCE($6, status),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, provider_id, model_id, display_name, capabilities, is_default, status, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&input.model_id)
        .bind(&input.display_name)
        .bind(&capabilities_json)
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
            SELECT m.id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_default, m.status, m.created_at, m.updated_at,
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
        input: CreateAgentCapabilityRow,
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

    // ============================================
    // Session Files (virtual filesystem)
    // ============================================

    /// Create a new file or directory in the session virtual filesystem
    pub async fn create_session_file(&self, input: CreateSessionFileRow) -> Result<SessionFileRow> {
        let size_bytes = input.content.as_ref().map(|c| c.len() as i64).unwrap_or(0);

        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            INSERT INTO session_files (session_id, path, content, is_directory, is_readonly, size_bytes)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            "#,
        )
        .bind(input.session_id)
        .bind(&input.path)
        .bind(&input.content)
        .bind(input.is_directory)
        .bind(input.is_readonly)
        .bind(size_bytes)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get a file by session and path
    pub async fn get_session_file(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<SessionFileRow>> {
        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            SELECT id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM session_files
            WHERE session_id = $1 AND path = $2
            "#,
        )
        .bind(session_id)
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get a file by ID
    pub async fn get_session_file_by_id(&self, id: Uuid) -> Result<Option<SessionFileRow>> {
        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            SELECT id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM session_files
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List files in a directory (immediate children only, no content)
    pub async fn list_session_files(
        &self,
        session_id: Uuid,
        parent_path: &str,
    ) -> Result<Vec<SessionFileInfoRow>> {
        // Root directory case
        let pattern = if parent_path == "/" {
            "^/[^/]+$".to_string()
        } else {
            format!("^{}/[^/]+$", regex::escape(parent_path))
        };

        let rows = sqlx::query_as::<_, SessionFileInfoRow>(
            r#"
            SELECT id, session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM session_files
            WHERE session_id = $1 AND path ~ $2
            ORDER BY is_directory DESC, path ASC
            "#,
        )
        .bind(session_id)
        .bind(&pattern)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// List all files in a session (recursive, no content)
    pub async fn list_all_session_files(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionFileInfoRow>> {
        let rows = sqlx::query_as::<_, SessionFileInfoRow>(
            r#"
            SELECT id, session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM session_files
            WHERE session_id = $1
            ORDER BY path ASC
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Update a session file (content and/or metadata)
    pub async fn update_session_file(
        &self,
        session_id: Uuid,
        path: &str,
        input: UpdateSessionFile,
    ) -> Result<Option<SessionFileRow>> {
        // Calculate new size if content is being updated
        let size_bytes = input.content.as_ref().map(|c| c.len() as i64);

        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            UPDATE session_files
            SET
                content = COALESCE($3, content),
                is_readonly = COALESCE($4, is_readonly),
                size_bytes = COALESCE($5, size_bytes)
            WHERE session_id = $1 AND path = $2 AND is_directory = FALSE
            RETURNING id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            "#,
        )
        .bind(session_id)
        .bind(path)
        .bind(&input.content)
        .bind(input.is_readonly)
        .bind(size_bytes)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Delete a file or directory (directories must be empty)
    pub async fn delete_session_file(&self, session_id: Uuid, path: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM session_files WHERE session_id = $1 AND path = $2")
            .bind(session_id)
            .bind(path)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete a directory and all its contents recursively
    pub async fn delete_session_file_recursive(&self, session_id: Uuid, path: &str) -> Result<u64> {
        // Delete the directory and all paths that start with it
        let pattern = if path == "/" {
            // Delete all files in session
            "^/".to_string()
        } else {
            format!("^{}(/|$)", regex::escape(path))
        };

        let result = sqlx::query("DELETE FROM session_files WHERE session_id = $1 AND path ~ $2")
            .bind(session_id)
            .bind(&pattern)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    /// Move/rename a file or directory
    pub async fn move_session_file(
        &self,
        session_id: Uuid,
        old_path: &str,
        new_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        // For directories, we need to move all children as well
        let mut tx = self.pool.begin().await?;

        // First, check if source exists and is a directory
        let source = sqlx::query_as::<_, SessionFileRow>(
            r#"
            SELECT id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM session_files
            WHERE session_id = $1 AND path = $2
            "#,
        )
        .bind(session_id)
        .bind(old_path)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(source) = source else {
            return Ok(None);
        };

        if source.is_directory {
            // Move all children by replacing the prefix
            let old_prefix = format!("{}/", old_path);
            let new_prefix = format!("{}/", new_path);

            sqlx::query(
                r#"
                UPDATE session_files
                SET path = $3 || substring(path from $4)
                WHERE session_id = $1 AND path LIKE $2
                "#,
            )
            .bind(session_id)
            .bind(format!("{}%", old_prefix))
            .bind(&new_prefix)
            .bind((old_prefix.len() + 1) as i32)
            .execute(&mut *tx)
            .await?;
        }

        // Move the file/directory itself
        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            UPDATE session_files
            SET path = $3
            WHERE session_id = $1 AND path = $2
            RETURNING id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            "#,
        )
        .bind(session_id)
        .bind(old_path)
        .bind(new_path)
        .fetch_optional(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(row)
    }

    /// Copy a file (directories not supported yet)
    pub async fn copy_session_file(
        &self,
        session_id: Uuid,
        src_path: &str,
        dst_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            INSERT INTO session_files (session_id, path, content, is_directory, is_readonly, size_bytes)
            SELECT session_id, $3, content, is_directory, is_readonly, size_bytes
            FROM session_files
            WHERE session_id = $1 AND path = $2 AND is_directory = FALSE
            RETURNING id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            "#,
        )
        .bind(session_id)
        .bind(src_path)
        .bind(dst_path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Search file contents using regex pattern (grep-like)
    pub async fn grep_session_files(
        &self,
        session_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<SessionFileInfoRow>> {
        // Search text files for content matching the pattern
        let rows = if let Some(path_pat) = path_pattern {
            sqlx::query_as::<_, SessionFileInfoRow>(
                r#"
                SELECT id, session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
                FROM session_files
                WHERE session_id = $1
                    AND is_directory = FALSE
                    AND path ~ $2
                    AND convert_from(content, 'UTF8') ~ $3
                ORDER BY path ASC
                "#,
            )
            .bind(session_id)
            .bind(path_pat)
            .bind(pattern)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, SessionFileInfoRow>(
                r#"
                SELECT id, session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
                FROM session_files
                WHERE session_id = $1
                    AND is_directory = FALSE
                    AND convert_from(content, 'UTF8') ~ $2
                ORDER BY path ASC
                "#,
            )
            .bind(session_id)
            .bind(pattern)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }

    /// Check if a path exists
    pub async fn session_file_exists(&self, session_id: Uuid, path: &str) -> Result<bool> {
        let result: Option<(bool,)> =
            sqlx::query_as("SELECT TRUE FROM session_files WHERE session_id = $1 AND path = $2")
                .bind(session_id)
                .bind(path)
                .fetch_optional(&self.pool)
                .await?;

        Ok(result.is_some())
    }

    /// Check if a directory has any children
    pub async fn session_directory_has_children(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<bool> {
        let pattern = if path == "/" {
            "^/[^/]+".to_string()
        } else {
            format!("^{}/[^/]+", regex::escape(path))
        };

        let result: Option<(bool,)> = sqlx::query_as(
            "SELECT TRUE FROM session_files WHERE session_id = $1 AND path ~ $2 LIMIT 1",
        )
        .bind(session_id)
        .bind(&pattern)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }
}

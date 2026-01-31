// Repository layer for database operations
// M2 Revised: Agent/Session/Messages/Events model

use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::message_filter::{MessageFilter, MessageQuery};
use everruns_core::typed_id::{AgentId, EventId, SessionId};
use sqlx::PgPool;
use uuid::Uuid;

use super::models::*;

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
            SELECT id, org_id, user_id, name, key_hash, key_prefix, scopes, expires_at, last_used_at, created_at
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
            SELECT id, org_id, user_id, name, key_hash, key_prefix, scopes, expires_at, last_used_at, created_at
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

    pub async fn create_agent(&self, org_id: i64, input: CreateAgentRow) -> Result<AgentRow> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            INSERT INTO agents (org_id, name, description, system_prompt, default_model_id, tags, status)
            VALUES ($1, $2, $3, $4, $5, $6, 'active')
            RETURNING id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            "#,
        )
        .bind(org_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id)
        .bind(&input.tags)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create agent with a specific ID, idempotent (ON CONFLICT DO NOTHING)
    /// Returns None if agent already exists with this ID
    pub async fn create_agent_with_id(
        &self,
        org_id: i64,
        id: AgentId,
        input: CreateAgentRow,
    ) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            INSERT INTO agents (id, org_id, name, description, system_prompt, default_model_id, tags, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'active')
            ON CONFLICT (id) DO NOTHING
            RETURNING id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            "#,
        )
        .bind(id.uuid())
        .bind(org_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id.map(|m| m.uuid()))
        .bind(&input.tags)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent(&self, org_id: i64, id: AgentId) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            FROM agents
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id.uuid())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_agents(&self, org_id: i64) -> Result<Vec<AgentRow>> {
        let rows = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            FROM agents
            WHERE org_id = $1 AND status = 'active'
            ORDER BY created_at DESC
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn get_agent_by_name(&self, org_id: i64, name: &str) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            FROM agents
            WHERE org_id = $1 AND name = $2 AND status = 'active'
            "#,
        )
        .bind(org_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn update_agent(
        &self,
        org_id: i64,
        id: AgentId,
        input: UpdateAgent,
    ) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            UPDATE agents
            SET
                name = COALESCE($3, name),
                description = COALESCE($4, description),
                system_prompt = COALESCE($5, system_prompt),
                default_model_id = COALESCE($6, default_model_id),
                tags = COALESCE($7, tags),
                status = COALESCE($8, status),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            "#,
        )
        .bind(org_id)
        .bind(id.uuid())
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id.map(|m| m.uuid()))
        .bind(&input.tags)
        .bind(&input.status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_agent(&self, org_id: i64, id: AgentId) -> Result<bool> {
        // Archive instead of hard delete
        let result = sqlx::query(
            r#"
            UPDATE agents
            SET status = 'archived', updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status = 'active'
            "#,
        )
        .bind(org_id)
        .bind(id.uuid())
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
            INSERT INTO sessions (org_id, agent_id, title, tags, model_id, capabilities, status)
            VALUES ($1, $2, $3, $4, $5, $6, 'started')
            RETURNING id, org_id, agent_id, title, tags, model_id, capabilities, status, created_at, updated_at, started_at, finished_at,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            "#,
        )
        .bind(input.org_id)
        .bind(input.agent_id)
        .bind(&input.title)
        .bind(&input.tags)
        .bind(input.model_id)
        .bind(&input.capabilities)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get session by org and session id
    pub async fn get_session(&self, org_id: i64, id: SessionId) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, agent_id, title, tags, model_id, capabilities, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            FROM sessions
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List sessions for an organization with optional agent filter.
    /// Returns (sessions, total_count).
    pub async fn list_sessions(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
        pagination: crate::api::common::Pagination,
    ) -> Result<(Vec<SessionRow>, u32)> {
        // Get total count
        let total: (i64,) = if let Some(aid) = agent_id {
            sqlx::query_as(
                r#"
                SELECT COUNT(*) as count
                FROM sessions
                WHERE org_id = $1 AND agent_id = $2
                "#,
            )
            .bind(org_id)
            .bind(aid)
            .fetch_one(&self.pool)
            .await?
        } else {
            sqlx::query_as(
                r#"
                SELECT COUNT(*) as count
                FROM sessions
                WHERE org_id = $1
                "#,
            )
            .bind(org_id)
            .fetch_one(&self.pool)
            .await?
        };

        // Get paginated results
        let rows = if let Some(aid) = agent_id {
            sqlx::query_as::<_, SessionRow>(
                r#"
                SELECT id, org_id, agent_id, title, tags, model_id, capabilities, status, created_at, updated_at, started_at, finished_at,
                       total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
                FROM sessions
                WHERE org_id = $1 AND agent_id = $2
                ORDER BY created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(org_id)
            .bind(aid)
            .bind(pagination.limit as i64)
            .bind(pagination.offset as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, SessionRow>(
                r#"
                SELECT id, org_id, agent_id, title, tags, model_id, capabilities, status, created_at, updated_at, started_at, finished_at,
                       total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
                FROM sessions
                WHERE org_id = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(org_id)
            .bind(pagination.limit as i64)
            .bind(pagination.offset as i64)
            .fetch_all(&self.pool)
            .await?
        };

        Ok((rows, total.0 as u32))
    }

    /// Update session by org and session id
    pub async fn update_session(
        &self,
        org_id: i64,
        id: SessionId,
        input: UpdateSession,
    ) -> Result<Option<SessionRow>> {
        // Note: updated_at is automatically set by the update_sessions_updated_at trigger
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            UPDATE sessions
            SET
                title = COALESCE($3, title),
                tags = COALESCE($4, tags),
                model_id = COALESCE($5, model_id),
                status = COALESCE($6, status),
                started_at = COALESCE($7, started_at),
                finished_at = COALESCE($8, finished_at)
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, agent_id, title, tags, model_id, status, created_at, updated_at, started_at, finished_at,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.title)
        .bind(&input.tags)
        .bind(input.model_id.map(|m| m.uuid()))
        .bind(&input.status)
        .bind(input.started_at)
        .bind(input.finished_at)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Delete session by org and session id
    pub async fn delete_session(&self, org_id: i64, id: SessionId) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM sessions
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Events (source of truth for messages)
    // ============================================
    //
    // Messages are stored as events with type "message.*"
    // Use list_message_events() to load conversation messages.

    pub async fn create_event(&self, input: CreateEventRow) -> Result<EventRow> {
        // Atomically allocate next sequence number for this session
        // ID is generated by database using uuidv7() for monotonically increasing UUIDs
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            INSERT INTO events (session_id, sequence, event_type, ts, context, data, metadata, tags)
            VALUES ($1, allocate_event_sequence($1), $2, $3, $4, $5, $6, $7)
            RETURNING id, session_id, sequence, event_type, ts, context, data, metadata, tags, created_at
            "#,
        )
        .bind(input.session_id)
        .bind(&input.event_type)
        .bind(input.ts)
        .bind(&input.context)
        .bind(&input.data)
        .bind(&input.metadata)
        .bind(&input.tags)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_events(
        &self,
        session_id: SessionId,
        since_sequence: Option<i32>,
        since_id: Option<EventId>,
        exclude_types: &[String],
    ) -> Result<Vec<EventRow>> {
        // Prefer since_id (UUID v7 monotonically increasing) over sequence for filtering
        // exclude_types filters out unwanted event types (e.g., delta events)
        let rows = match (since_id, since_sequence) {
            (Some(id), _) => {
                sqlx::query_as::<_, EventRow>(
                    r#"
                    SELECT id, session_id, sequence, event_type, ts, context, data, metadata, tags, created_at
                    FROM events
                    WHERE session_id = $1 AND id > $2
                      AND (cardinality($3::text[]) = 0 OR event_type <> ALL($3))
                    ORDER BY id ASC
                    "#,
                )
                .bind(session_id.uuid())
                .bind(id.uuid())
                .bind(exclude_types)
                .fetch_all(&self.pool)
                .await?
            }
            (None, Some(seq)) => {
                sqlx::query_as::<_, EventRow>(
                    r#"
                    SELECT id, session_id, sequence, event_type, ts, context, data, metadata, tags, created_at
                    FROM events
                    WHERE session_id = $1 AND sequence > $2
                      AND (cardinality($3::text[]) = 0 OR event_type <> ALL($3))
                    ORDER BY sequence ASC
                    "#,
                )
                .bind(session_id.uuid())
                .bind(seq)
                .bind(exclude_types)
                .fetch_all(&self.pool)
                .await?
            }
            (None, None) => {
                sqlx::query_as::<_, EventRow>(
                    r#"
                    SELECT id, session_id, sequence, event_type, ts, context, data, metadata, tags, created_at
                    FROM events
                    WHERE session_id = $1
                      AND (cardinality($2::text[]) = 0 OR event_type <> ALL($2))
                    ORDER BY sequence ASC
                    "#,
                )
                .bind(session_id.uuid())
                .bind(exclude_types)
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(rows)
    }

    /// List only message events for a session (for MessageStore implementation)
    ///
    /// Returns events with types: input.message, output.message.completed, tool.completed
    /// Ordered by sequence for conversation reconstruction.
    /// Note: Tool calls are embedded in output.message.completed events via ContentPart::ToolCall.
    /// Note: Tool results come from tool.completed events (not message.tool_result).
    pub async fn list_message_events(&self, session_id: SessionId) -> Result<Vec<EventRow>> {
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, session_id, sequence, event_type, ts, context, data, metadata, tags, created_at
            FROM events
            WHERE session_id = $1
              AND event_type IN ('input.message', 'output.message.completed', 'tool.completed')
            ORDER BY sequence ASC
            "#,
        )
        .bind(session_id.uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// List message events for a session with filters applied.
    ///
    /// This method builds a dynamic SQL query based on the provided filters.
    /// DB-mappable filters (TimeRange, EventTypes, ToolName, Search, etc.) are
    /// pushed to the database for efficient filtering. Custom filters are applied
    /// in-memory after loading.
    ///
    /// Note: Injections are NOT applied here - they should be applied at the
    /// MessageRetriever layer after converting events to messages.
    pub async fn list_message_events_filtered(
        &self,
        query: &MessageQuery,
    ) -> Result<Vec<EventRow>> {
        // Build dynamic SQL query
        let mut sql = String::from(
            "SELECT id, session_id, sequence, event_type, ts, context, data, metadata, tags, created_at FROM events WHERE session_id = $1",
        );

        // Collect bound values for later binding
        // We track types separately since sqlx needs typed bindings
        let mut time_from: Option<DateTime<Utc>> = None;
        let mut time_to: Option<DateTime<Utc>> = None;
        let mut event_types: Option<Vec<String>> = None;
        let mut tool_name: Option<String> = None;
        let mut search_query: Option<String> = None;
        let mut exclude_ids: Option<Vec<Uuid>> = None;
        let mut include_ids: Option<Vec<Uuid>> = None;

        // Check for EventTypes filter, else use defaults
        for filter in &query.filters {
            if let MessageFilter::EventTypes(types) = filter {
                event_types = Some(types.clone());
                break;
            }
        }

        // Default event types if not specified
        let types = event_types.unwrap_or_else(|| {
            vec![
                "input.message".to_string(),
                "output.message.completed".to_string(),
                "tool.completed".to_string(),
            ]
        });
        sql.push_str(" AND event_type = ANY($2)");

        // Build parameter index tracker (starting at 3 since $1=session_id, $2=event_types)
        let mut param_idx = 3;

        // Process filters
        for filter in &query.filters {
            match filter {
                MessageFilter::EventTypes(_) => {
                    // Already handled above
                }
                MessageFilter::TimeRange { from, to } => {
                    if let Some(f) = from {
                        sql.push_str(&format!(" AND created_at >= ${}", param_idx));
                        time_from = Some(*f);
                        param_idx += 1;
                    }
                    if let Some(t) = to {
                        sql.push_str(&format!(" AND created_at <= ${}", param_idx));
                        time_to = Some(*t);
                        param_idx += 1;
                    }
                }
                MessageFilter::ToolName(name) => {
                    sql.push_str(&format!(
                        " AND (event_type = 'tool.completed' AND data->>'tool_name' = ${})",
                        param_idx
                    ));
                    tool_name = Some(name.clone());
                    param_idx += 1;
                }
                MessageFilter::Search(q) => {
                    sql.push_str(&format!(
                        " AND data::text ILIKE '%' || ${} || '%'",
                        param_idx
                    ));
                    search_query = Some(q.clone());
                    param_idx += 1;
                }
                MessageFilter::ExcludeIds(ids) => {
                    sql.push_str(&format!(" AND id != ALL(${})", param_idx));
                    exclude_ids = Some(ids.iter().map(|id| id.uuid()).collect());
                    param_idx += 1;
                }
                MessageFilter::IncludeIds(ids) => {
                    sql.push_str(&format!(" AND id = ANY(${})", param_idx));
                    include_ids = Some(ids.iter().map(|id| id.uuid()).collect());
                    param_idx += 1;
                }
                MessageFilter::Custom(_) => {
                    // Custom filters are applied in-memory, not in SQL
                }
            }
        }

        sql.push_str(" ORDER BY sequence ASC");

        // Apply limit/offset
        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }
        if let Some(offset) = query.offset {
            sql.push_str(&format!(" OFFSET {}", offset));
        }

        // Build and execute query with dynamic bindings
        // sqlx doesn't support truly dynamic queries, so we need to use raw SQL
        // with all possible parameters, binding None for unused ones
        let mut db_query = sqlx::query_as::<_, EventRow>(&sql)
            .bind(query.session_id.uuid())
            .bind(&types);

        // Bind parameters in order they were added to SQL
        if time_from.is_some() {
            db_query = db_query.bind(time_from);
        }
        if time_to.is_some() {
            db_query = db_query.bind(time_to);
        }
        if tool_name.is_some() {
            db_query = db_query.bind(tool_name);
        }
        if search_query.is_some() {
            db_query = db_query.bind(search_query);
        }
        if exclude_ids.is_some() {
            db_query = db_query.bind(exclude_ids);
        }
        if include_ids.is_some() {
            db_query = db_query.bind(include_ids);
        }

        let rows = db_query.fetch_all(&self.pool).await?;

        Ok(rows)
    }

    /// Get preview text for multiple sessions
    ///
    /// Returns a map of session_id -> preview text (first 200 chars of first user message)
    /// This is an efficient batch query that gets previews for all provided session IDs.
    pub async fn get_session_previews(
        &self,
        session_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, String>> {
        if session_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        // Use DISTINCT ON to get the first user message for each session
        // Extract text content from the JSON data structure
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            r#"
            WITH first_messages AS (
                SELECT DISTINCT ON (session_id)
                    session_id,
                    data
                FROM events
                WHERE session_id = ANY($1)
                  AND event_type = 'input.message'
                ORDER BY session_id, sequence ASC
            )
            SELECT
                session_id,
                LEFT(
                    COALESCE(
                        -- Extract text from content array: [{"type": "text", "text": "..."}]
                        data->'message'->'content'->0->>'text',
                        -- Fallback: try direct text field
                        data->'message'->>'text',
                        ''
                    ),
                    200
                ) as preview
            FROM first_messages
            WHERE data->'message'->'content'->0->>'text' IS NOT NULL
               OR data->'message'->>'text' IS NOT NULL
            "#,
        )
        .bind(session_ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().collect())
    }

    /// Get output preview text for multiple sessions
    ///
    /// Returns a map of session_id -> preview text (first 200 chars of last agent message)
    /// This is an efficient batch query that gets output previews for all provided session IDs.
    pub async fn get_session_output_previews(
        &self,
        session_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, String>> {
        if session_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        // Use DISTINCT ON with DESC order to get the last agent message for each session
        // Extract text content from the JSON data structure
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            r#"
            WITH last_messages AS (
                SELECT DISTINCT ON (session_id)
                    session_id,
                    data
                FROM events
                WHERE session_id = ANY($1)
                  AND event_type = 'output.message.completed'
                ORDER BY session_id, sequence DESC
            )
            SELECT
                session_id,
                LEFT(
                    COALESCE(
                        -- Extract text from content array: [{"type": "text", "text": "..."}]
                        data->'message'->'content'->0->>'text',
                        -- Fallback: try direct text field
                        data->'message'->>'text',
                        ''
                    ),
                    200
                ) as preview
            FROM last_messages
            WHERE data->'message'->'content'->0->>'text' IS NOT NULL
               OR data->'message'->>'text' IS NOT NULL
            "#,
        )
        .bind(session_ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().collect())
    }

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
            ON CONFLICT (id) DO NOTHING
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

    pub async fn get_llm_provider(&self, id: Uuid) -> Result<Option<LlmProviderRow>> {
        let row = sqlx::query_as::<_, LlmProviderRow>(
            r#"
            SELECT id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, status, settings, last_synced_at, created_at, updated_at
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
            SELECT id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, status, settings, last_synced_at, created_at, updated_at
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
                status = COALESCE($7, status),
                settings = COALESCE($8, settings),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, org_id, name, provider_type, base_url, api_key_encrypted, api_key_set, status, settings, last_synced_at, created_at, updated_at
            "#,
        )
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

    pub async fn delete_llm_provider(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM llm_providers WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Update provider's last_synced_at timestamp
    pub async fn update_provider_last_synced(
        &self,
        id: Uuid,
        last_synced_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE llm_providers SET last_synced_at = $2, updated_at = NOW() WHERE id = $1",
        )
        .bind(id)
        .bind(last_synced_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get the default LLM model with provider info.
    /// Returns the model marked as default (is_default = true) with its provider info.
    pub async fn get_default_llm_model(
        &self,
        org_id: i64,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let row = sqlx::query_as::<_, LlmModelWithProviderRow>(
            r#"
            SELECT m.id, m.org_id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_default, m.is_favorite, m.status, m.source, m.last_seen_at, m.provider_metadata, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type
            FROM llm_models m
            JOIN llm_providers p ON m.provider_id = p.id
            WHERE m.is_default = TRUE AND m.status = 'active' AND p.status = 'active' AND m.org_id = $1
            "#,
        )
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Clear all model defaults (set is_default = false for all models in org).
    /// Used to implement "last wins" default logic.
    pub async fn clear_all_model_defaults(&self, org_id: i64) -> Result<()> {
        sqlx::query(
            "UPDATE llm_models SET is_default = FALSE WHERE is_default = TRUE AND org_id = $1",
        )
        .bind(org_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get a provider with its decrypted API key from database.
    ///
    /// Note: Environment variable fallback (DEFAULT_*_API_KEY) is handled
    /// at the service layer (LlmResolverService), not here.
    pub fn get_provider_with_api_key(
        &self,
        provider: &LlmProviderRow,
        encryption: &super::EncryptionService,
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

    pub async fn create_llm_model(
        &self,
        org_id: i64,
        input: CreateLlmModelRow,
    ) -> Result<LlmModelRow> {
        let capabilities_json = serde_json::to_value(&input.capabilities)?;

        let row = sqlx::query_as::<_, LlmModelRow>(
            r#"
            INSERT INTO llm_models (org_id, provider_id, model_id, display_name, capabilities, is_default, is_favorite, source, provider_metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING id, org_id, provider_id, model_id, display_name, capabilities, is_default, is_favorite, status, source, last_seen_at, provider_metadata, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(input.provider_id)
        .bind(&input.model_id)
        .bind(&input.display_name)
        .bind(&capabilities_json)
        .bind(input.is_default)
        .bind(input.is_favorite)
        .bind(&input.source)
        .bind(&input.provider_metadata)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create or update a model with a specific ID (for seeding)
    /// Uses upsert to update display_name, is_default, is_favorite if model exists
    pub async fn create_llm_model_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateLlmModelRow,
    ) -> Result<Option<LlmModelRow>> {
        let capabilities_json = serde_json::to_value(&input.capabilities)?;

        let row = sqlx::query_as::<_, LlmModelRow>(
            r#"
            INSERT INTO llm_models (id, org_id, provider_id, model_id, display_name, capabilities, is_default, is_favorite, source, provider_metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO UPDATE SET
                display_name = EXCLUDED.display_name,
                is_default = EXCLUDED.is_default,
                is_favorite = EXCLUDED.is_favorite,
                updated_at = NOW()
            RETURNING id, org_id, provider_id, model_id, display_name, capabilities, is_default, is_favorite, status, source, last_seen_at, provider_metadata, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(org_id)
        .bind(input.provider_id)
        .bind(&input.model_id)
        .bind(&input.display_name)
        .bind(&capabilities_json)
        .bind(input.is_default)
        .bind(input.is_favorite)
        .bind(&input.source)
        .bind(&input.provider_metadata)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_llm_model(&self, id: Uuid) -> Result<Option<LlmModelRow>> {
        let row = sqlx::query_as::<_, LlmModelRow>(
            r#"
            SELECT id, org_id, provider_id, model_id, display_name, capabilities, is_default, is_favorite, status, source, last_seen_at, provider_metadata, created_at, updated_at
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
            SELECT m.id, m.org_id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_default, m.is_favorite, m.status, m.source, m.last_seen_at, m.provider_metadata, m.created_at, m.updated_at,
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
            SELECT id, org_id, provider_id, model_id, display_name, capabilities, is_default, is_favorite, status, source, last_seen_at, provider_metadata, created_at, updated_at
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

    pub async fn list_all_llm_models(&self, org_id: i64) -> Result<Vec<LlmModelWithProviderRow>> {
        let rows = sqlx::query_as::<_, LlmModelWithProviderRow>(
            r#"
            SELECT m.id, m.org_id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_default, m.is_favorite, m.status, m.source, m.last_seen_at, m.provider_metadata, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type
            FROM llm_models m
            JOIN llm_providers p ON m.provider_id = p.id
            WHERE m.status = 'active' AND p.status = 'active' AND m.org_id = $1
            ORDER BY m.is_favorite DESC, p.name ASC, m.display_name ASC
            "#,
        )
        .bind(org_id)
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
                is_favorite = COALESCE($6, is_favorite),
                status = COALESCE($7, status),
                last_seen_at = COALESCE($8, last_seen_at),
                provider_metadata = COALESCE($9, provider_metadata),
                updated_at = NOW()
            WHERE id = $1
            RETURNING id, org_id, provider_id, model_id, display_name, capabilities, is_default, is_favorite, status, source, last_seen_at, provider_metadata, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&input.model_id)
        .bind(&input.display_name)
        .bind(&capabilities_json)
        .bind(input.is_default)
        .bind(input.is_favorite)
        .bind(&input.status)
        .bind(input.last_seen_at)
        .bind(&input.provider_metadata)
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
        org_id: i64,
        model_id: &str,
    ) -> Result<Option<LlmModelWithProviderRow>> {
        let row = sqlx::query_as::<_, LlmModelWithProviderRow>(
            r#"
            SELECT m.id, m.org_id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_default, m.is_favorite, m.status, m.source, m.last_seen_at, m.provider_metadata, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type
            FROM llm_models m
            JOIN llm_providers p ON m.provider_id = p.id
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
    // Agent Capabilities
    // ============================================

    /// Get capabilities for an agent, ordered by position
    pub async fn get_agent_capabilities(&self, agent_id: Uuid) -> Result<Vec<AgentCapabilityRow>> {
        let rows = sqlx::query_as::<_, AgentCapabilityRow>(
            r#"
            SELECT id, agent_id, capability_id, position, config, created_at
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
    /// capabilities: list of (capability_id, position, config) tuples
    pub async fn set_agent_capabilities(
        &self,
        agent_id: Uuid,
        capabilities: Vec<(String, i32, serde_json::Value)>,
    ) -> Result<Vec<AgentCapabilityRow>> {
        // Start a transaction
        let mut tx = self.pool.begin().await?;

        // Delete existing capabilities for this agent
        sqlx::query("DELETE FROM agent_capabilities WHERE agent_id = $1")
            .bind(agent_id)
            .execute(&mut *tx)
            .await?;

        // Insert new capabilities
        for (capability_id, position, config) in &capabilities {
            sqlx::query(
                r#"
                INSERT INTO agent_capabilities (agent_id, capability_id, position, config)
                VALUES ($1, $2, $3, $4)
                "#,
            )
            .bind(agent_id)
            .bind(capability_id)
            .bind(position)
            .bind(config)
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
            INSERT INTO agent_capabilities (agent_id, capability_id, position, config)
            VALUES ($1, $2, $3, $4)
            RETURNING id, agent_id, capability_id, position, config, created_at
            "#,
        )
        .bind(input.agent_id)
        .bind(&input.capability_id)
        .bind(input.position)
        .bind(&input.config)
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

    // ============================================
    // MCP Servers
    // ============================================

    pub async fn create_mcp_server(
        &self,
        org_id: i64,
        input: CreateMcpServerRow,
    ) -> Result<McpServerRow> {
        let headers = input.headers.unwrap_or(serde_json::json!({}));
        let settings = input.settings.unwrap_or(serde_json::json!({}));
        let api_key_set = input.api_key_encrypted.is_some();

        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            INSERT INTO mcp_servers (org_id, name, description, url, transport_type, api_key_encrypted, api_key_set, headers, settings)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.url)
        .bind(&input.transport_type)
        .bind(&input.api_key_encrypted)
        .bind(api_key_set)
        .bind(&headers)
        .bind(&settings)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create MCP server with a specific ID (for seeding)
    /// Returns None if server already exists with this ID
    pub async fn create_mcp_server_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateMcpServerRow,
    ) -> Result<Option<McpServerRow>> {
        let headers = input.headers.unwrap_or(serde_json::json!({}));
        let settings = input.settings.unwrap_or(serde_json::json!({}));
        let api_key_set = input.api_key_encrypted.is_some();

        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            INSERT INTO mcp_servers (id, org_id, name, description, url, transport_type, api_key_encrypted, api_key_set, headers, settings)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO NOTHING
            RETURNING id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(org_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.url)
        .bind(&input.transport_type)
        .bind(&input.api_key_encrypted)
        .bind(api_key_set)
        .bind(&headers)
        .bind(&settings)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_mcp_server(&self, id: Uuid) -> Result<Option<McpServerRow>> {
        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
            FROM mcp_servers
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_mcp_server_by_name(&self, name: &str) -> Result<Option<McpServerRow>> {
        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
            FROM mcp_servers
            WHERE name = $1
            "#,
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_mcp_servers(&self) -> Result<Vec<McpServerRow>> {
        let rows = sqlx::query_as::<_, McpServerRow>(
            r#"
            SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
            FROM mcp_servers
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// List only active MCP servers (for capability listing)
    pub async fn list_active_mcp_servers(&self) -> Result<Vec<McpServerRow>> {
        let rows = sqlx::query_as::<_, McpServerRow>(
            r#"
            SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
            FROM mcp_servers
            WHERE status = 'active'
            ORDER BY name ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_mcp_server(
        &self,
        id: Uuid,
        input: UpdateMcpServer,
    ) -> Result<Option<McpServerRow>> {
        // Handle api_key_set: if we're updating the encrypted key, also update the flag
        let api_key_set = input.api_key_encrypted.as_ref().map(|_| true);

        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            UPDATE mcp_servers
            SET
                name = COALESCE($2, name),
                description = COALESCE($3, description),
                url = COALESCE($4, url),
                transport_type = COALESCE($5, transport_type),
                status = COALESCE($6, status),
                api_key_encrypted = COALESCE($7, api_key_encrypted),
                api_key_set = COALESCE($8, api_key_set),
                headers = COALESCE($9, headers),
                settings = COALESCE($10, settings)
            WHERE id = $1
            RETURNING id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.url)
        .bind(&input.transport_type)
        .bind(&input.status)
        .bind(&input.api_key_encrypted)
        .bind(api_key_set)
        .bind(&input.headers)
        .bind(&input.settings)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Update cached tools for an MCP server
    pub async fn update_mcp_server_tools(
        &self,
        id: Uuid,
        input: UpdateMcpServerTools,
    ) -> Result<Option<McpServerRow>> {
        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            UPDATE mcp_servers
            SET
                cached_tools = $2,
                tools_cached_at = NOW()
            WHERE id = $1
            RETURNING id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&input.cached_tools)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_mcp_server(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM mcp_servers WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
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

    // ============================================
    // Images
    // ============================================

    pub async fn create_image(&self, input: CreateImageRow) -> Result<ImageRow> {
        let row = sqlx::query_as::<_, ImageRow>(
            r#"
            INSERT INTO images (filename, content_type, size_bytes, data, thumbnail_data, thumbnail_content_type, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id, filename, content_type, size_bytes, data, thumbnail_data, thumbnail_content_type, metadata, created_at
            "#,
        )
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

    pub async fn get_image(&self, id: Uuid) -> Result<Option<ImageRow>> {
        let row = sqlx::query_as::<_, ImageRow>(
            r#"
            SELECT id, filename, content_type, size_bytes, data, thumbnail_data, thumbnail_content_type, metadata, created_at
            FROM images
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_image_info(&self, id: Uuid) -> Result<Option<ImageInfoRow>> {
        let row = sqlx::query_as::<_, ImageInfoRow>(
            r#"
            SELECT id, filename, content_type, size_bytes, metadata, created_at
            FROM images
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_image(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM images WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn list_images(&self, limit: i64, offset: i64) -> Result<Vec<ImageInfoRow>> {
        let rows = sqlx::query_as::<_, ImageInfoRow>(
            r#"
            SELECT id, filename, content_type, size_bytes, metadata, created_at
            FROM images
            ORDER BY created_at DESC
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // ============================================
    // Organizations
    // ============================================

    pub async fn create_organization(
        &self,
        input: CreateOrganizationRow,
    ) -> Result<OrganizationRow> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            INSERT INTO organizations (public_id, name)
            VALUES ($1, $2)
            RETURNING org_id, public_id, name, created_at, updated_at
            "#,
        )
        .bind(&input.public_id)
        .bind(&input.name)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create organization with specific org_id (for seeding).
    /// Returns None if org_id already exists.
    pub async fn create_organization_with_id(
        &self,
        org_id: i64,
        input: CreateOrganizationRow,
    ) -> Result<Option<OrganizationRow>> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            INSERT INTO organizations (org_id, public_id, name)
            VALUES ($1, $2, $3)
            ON CONFLICT (org_id) DO NOTHING
            RETURNING org_id, public_id, name, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_organization(&self, org_id: i64) -> Result<Option<OrganizationRow>> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            SELECT org_id, public_id, name, created_at, updated_at
            FROM organizations
            WHERE org_id = $1
            "#,
        )
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_organization_by_public_id(
        &self,
        public_id: &str,
    ) -> Result<Option<OrganizationRow>> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            SELECT org_id, public_id, name, created_at, updated_at
            FROM organizations
            WHERE public_id = $1
            "#,
        )
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_organizations(&self) -> Result<Vec<OrganizationRow>> {
        let rows = sqlx::query_as::<_, OrganizationRow>(
            r#"
            SELECT org_id, public_id, name, created_at, updated_at
            FROM organizations
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_organization(
        &self,
        org_id: i64,
        input: UpdateOrganization,
    ) -> Result<Option<OrganizationRow>> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            UPDATE organizations
            SET
                name = COALESCE($2, name),
                updated_at = NOW()
            WHERE org_id = $1
            RETURNING org_id, public_id, name, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(&input.name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_organization(&self, org_id: i64) -> Result<bool> {
        let result = sqlx::query("DELETE FROM organizations WHERE org_id = $1")
            .bind(org_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Organization Members
    // ============================================

    pub async fn add_organization_member(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<OrganizationMemberRow> {
        let row = sqlx::query_as::<_, OrganizationMemberRow>(
            r#"
            INSERT INTO organization_members (org_id, user_id)
            VALUES ($1, $2)
            ON CONFLICT (org_id, user_id) DO UPDATE SET org_id = EXCLUDED.org_id
            RETURNING org_id, user_id, created_at
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn remove_organization_member(&self, org_id: i64, user_id: Uuid) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM organization_members WHERE org_id = $1 AND user_id = $2")
                .bind(org_id)
                .bind(user_id)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn list_organization_members(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrganizationMemberRow>> {
        let rows = sqlx::query_as::<_, OrganizationMemberRow>(
            r#"
            SELECT org_id, user_id, created_at
            FROM organization_members
            WHERE org_id = $1
            ORDER BY created_at DESC
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn list_user_organizations(&self, user_id: Uuid) -> Result<Vec<OrganizationRow>> {
        let rows = sqlx::query_as::<_, OrganizationRow>(
            r#"
            SELECT o.org_id, o.public_id, o.name, o.created_at, o.updated_at
            FROM organizations o
            JOIN organization_members om ON o.org_id = om.org_id
            WHERE om.user_id = $1
            ORDER BY o.name
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn is_organization_member(&self, org_id: i64, user_id: Uuid) -> Result<bool> {
        let row: Option<(i32,)> = sqlx::query_as(
            r#"
            SELECT 1 as exists_flag
            FROM organization_members
            WHERE org_id = $1 AND user_id = $2
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.is_some())
    }

    // ============================================
    // Session Key/Value Storage
    // ============================================

    /// Upsert a session key/value (insert or update)
    pub async fn upsert_session_key_value(
        &self,
        input: UpsertSessionKeyValue,
    ) -> Result<SessionKeyValueRow> {
        let row = sqlx::query_as::<_, SessionKeyValueRow>(
            r#"
            INSERT INTO session_key_values (session_id, key, value)
            VALUES ($1, $2, $3)
            ON CONFLICT (session_id, key) DO UPDATE
            SET value = EXCLUDED.value, updated_at = NOW()
            RETURNING id, session_id, key, value, created_at, updated_at
            "#,
        )
        .bind(input.session_id)
        .bind(&input.key)
        .bind(&input.value)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get a session key/value by key
    pub async fn get_session_key_value(
        &self,
        session_id: Uuid,
        key: &str,
    ) -> Result<Option<SessionKeyValueRow>> {
        let row = sqlx::query_as::<_, SessionKeyValueRow>(
            r#"
            SELECT id, session_id, key, value, created_at, updated_at
            FROM session_key_values
            WHERE session_id = $1 AND key = $2
            "#,
        )
        .bind(session_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List all keys for a session (without values)
    pub async fn list_session_keys(&self, session_id: Uuid) -> Result<Vec<SessionKeyInfoRow>> {
        let rows = sqlx::query_as::<_, SessionKeyInfoRow>(
            r#"
            SELECT key, created_at, updated_at
            FROM session_key_values
            WHERE session_id = $1
            ORDER BY key
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Delete a session key/value by key
    pub async fn delete_session_key_value(&self, session_id: Uuid, key: &str) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM session_key_values
            WHERE session_id = $1 AND key = $2
            "#,
        )
        .bind(session_id)
        .bind(key)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Session Secret Storage (Encrypted)
    // ============================================

    /// Upsert a session secret (insert or update)
    pub async fn upsert_session_secret(
        &self,
        input: UpsertSessionSecret,
    ) -> Result<SessionSecretRow> {
        let row = sqlx::query_as::<_, SessionSecretRow>(
            r#"
            INSERT INTO session_secrets (session_id, name, value_encrypted)
            VALUES ($1, $2, $3)
            ON CONFLICT (session_id, name) DO UPDATE
            SET value_encrypted = EXCLUDED.value_encrypted, updated_at = NOW()
            RETURNING id, session_id, name, value_encrypted, created_at, updated_at
            "#,
        )
        .bind(input.session_id)
        .bind(&input.name)
        .bind(&input.value_encrypted)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get a session secret by name
    pub async fn get_session_secret(
        &self,
        session_id: Uuid,
        name: &str,
    ) -> Result<Option<SessionSecretRow>> {
        let row = sqlx::query_as::<_, SessionSecretRow>(
            r#"
            SELECT id, session_id, name, value_encrypted, created_at, updated_at
            FROM session_secrets
            WHERE session_id = $1 AND name = $2
            "#,
        )
        .bind(session_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List all secret names for a session (without encrypted values)
    pub async fn list_session_secrets(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionSecretInfoRow>> {
        let rows = sqlx::query_as::<_, SessionSecretInfoRow>(
            r#"
            SELECT name, created_at, updated_at
            FROM session_secrets
            WHERE session_id = $1
            ORDER BY name
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Delete a session secret by name
    pub async fn delete_session_secret(&self, session_id: Uuid, name: &str) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM session_secrets
            WHERE session_id = $1 AND name = $2
            "#,
        )
        .bind(session_id)
        .bind(name)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}

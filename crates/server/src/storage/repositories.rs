// Repository layer for database operations
// M2 Revised: Agent/Session/Messages/Events model

use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::message_filter::{MessageFilter, MessageQuery};
use everruns_core::typed_id::{
    AgentId, EventId, HarnessId, MessageId, NotificationId, ScheduleId, SessionId,
};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use super::models::*;

/// Database pool configuration loaded from environment variables.
pub struct DatabasePoolConfig {
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout: std::time::Duration,
    pub idle_timeout: std::time::Duration,
}

impl Default for DatabasePoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 50,
            min_connections: 5,
            acquire_timeout: std::time::Duration::from_secs(5),
            idle_timeout: std::time::Duration::from_secs(300),
        }
    }
}

impl DatabasePoolConfig {
    /// Load pool configuration from environment variables with sensible defaults.
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            max_connections: std::env::var("DATABASE_POOL_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(defaults.max_connections),
            min_connections: std::env::var("DATABASE_POOL_MIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(defaults.min_connections),
            acquire_timeout: std::env::var("DATABASE_ACQUIRE_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .map(std::time::Duration::from_secs)
                .unwrap_or(defaults.acquire_timeout),
            idle_timeout: std::env::var("DATABASE_IDLE_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .map(std::time::Duration::from_secs)
                .unwrap_or(defaults.idle_timeout),
        }
    }
}

/// Max search tokens to prevent oversized queries from long inputs (e.g. a poem).
const MAX_SEARCH_TOKENS: usize = 8;

/// Escape SQL LIKE special characters (`%`, `_`, `\`) so user input is treated
/// as literal text.
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '%' | '_' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Build multi-word search SQL conditions. Each whitespace-separated token must
/// match somewhere in the concatenated fields (case-insensitive).
///
/// Returns `(sql_fragment, patterns)` where `sql_fragment` looks like
/// ` AND (lower_expr LIKE $2 ESCAPE '\') AND (lower_expr LIKE $3 ESCAPE '\')`
/// and `patterns` is `["%token1%", "%token2%"]`.
///
/// `lower_expr` should be a SQL expression like
/// `LOWER(name || ' ' || COALESCE(description, ''))`.
///
/// Tokens are capped at [`MAX_SEARCH_TOKENS`] to prevent performance
/// degradation from excessively long queries.
fn build_search_sql(
    search: Option<&str>,
    lower_expr: &str,
    start_param: usize,
) -> (String, Vec<String>) {
    let tokens: Vec<String> = search
        .filter(|q| !q.trim().is_empty())
        .map(|q| {
            q.trim()
                .to_lowercase()
                .split_whitespace()
                .take(MAX_SEARCH_TOKENS)
                .map(|t| format!("%{}%", escape_like(t)))
                .collect()
        })
        .unwrap_or_default();

    if tokens.is_empty() {
        return (String::new(), Vec::new());
    }

    let mut sql = String::new();
    for (i, _) in tokens.iter().enumerate() {
        use std::fmt::Write;
        write!(
            sql,
            " AND ({lower_expr} LIKE ${idx} ESCAPE '\\')",
            idx = start_param + i
        )
        .unwrap();
    }
    (sql, tokens)
}

#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create database connection pool from URL with configurable pool settings.
    ///
    /// Multi-instance sizing: set `DATABASE_POOL_MAX = pg_max_connections / N - margin`
    /// where N = number of control-plane instances.
    pub async fn from_url(database_url: &str) -> Result<Self> {
        let config = DatabasePoolConfig::from_env();
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(config.acquire_timeout)
            .idle_timeout(config.idle_timeout)
            .connect(database_url)
            .await?;
        tracing::info!(
            max_connections = config.max_connections,
            min_connections = config.min_connections,
            acquire_timeout_secs = config.acquire_timeout.as_secs(),
            idle_timeout_secs = config.idle_timeout.as_secs(),
            "Database connection pool initialized"
        );

        // Multi-instance pool sizing check
        let instances: u32 = std::env::var("EXPECTED_INSTANCES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1)
            .max(1);
        if instances > 1 {
            let estimated_total = config.max_connections.saturating_mul(instances);
            // PostgreSQL default max_connections is 100; warn if we'd exceed 80% of it
            let pg_max: u32 = std::env::var("PG_MAX_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100);
            if estimated_total > pg_max * 80 / 100 {
                tracing::warn!(
                    pool_max = config.max_connections,
                    instances,
                    estimated_total,
                    pg_max,
                    "Pool size × instances ({estimated_total}) exceeds 80% of PG_MAX_CONNECTIONS ({pg_max}). \
                     Reduce DATABASE_POOL_MAX or increase PostgreSQL max_connections."
                );
            } else {
                tracing::info!(
                    pool_max = config.max_connections,
                    instances,
                    estimated_total,
                    pg_max,
                    "Database pool sizing OK for multi-instance deployment"
                );
            }
        }

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
            INSERT INTO users (email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, external_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
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
        .bind(&input.external_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create user with a specific UUID (for seeding).
    /// Returns None if id already exists.
    pub async fn create_user_with_id(
        &self,
        id: Uuid,
        input: CreateUserRow,
    ) -> Result<Option<UserRow>> {
        let roles_json = serde_json::to_value(&input.roles)?;

        let row = sqlx::query_as::<_, UserRow>(
            r#"
            INSERT INTO users (id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, external_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO NOTHING
            RETURNING id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
            "#,
        )
        .bind(id)
        .bind(&input.email)
        .bind(&input.name)
        .bind(&input.avatar_url)
        .bind(&roles_json)
        .bind(&input.password_hash)
        .bind(input.email_verified)
        .bind(&input.auth_provider)
        .bind(&input.auth_provider_id)
        .bind(&input.external_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<UserRow>> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
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
            SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
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
            SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
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
            RETURNING id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
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
                    SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
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
                    SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
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

    /// List users within an organization (TM-TENANT-008: org-scoped user listing)
    /// Filters via organization_members join to enforce tenant isolation.
    pub async fn list_users_by_org(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<UserRow>> {
        let rows = match search {
            Some(query) if !query.trim().is_empty() => {
                let search_pattern = format!("%{}%", query.trim().to_lowercase());
                sqlx::query_as::<_, UserRow>(
                    r#"
                    SELECT u.id, u.email, u.name, u.avatar_url, u.roles, u.password_hash, u.email_verified, u.auth_provider, u.auth_provider_id, u.created_at, u.updated_at, u.external_id
                    FROM users u
                    JOIN organization_members om ON u.id = om.user_id
                    WHERE om.org_id = $1 AND (LOWER(u.name) LIKE $2 OR LOWER(u.email) LIKE $2)
                    ORDER BY u.created_at DESC
                    "#,
                )
                .bind(org_id)
                .bind(&search_pattern)
                .fetch_all(&self.pool)
                .await?
            }
            _ => {
                sqlx::query_as::<_, UserRow>(
                    r#"
                    SELECT u.id, u.email, u.name, u.avatar_url, u.roles, u.password_hash, u.email_verified, u.auth_provider, u.auth_provider_id, u.created_at, u.updated_at, u.external_id
                    FROM users u
                    JOIN organization_members om ON u.id = om.user_id
                    WHERE om.org_id = $1
                    ORDER BY u.created_at DESC
                    "#,
                )
                .bind(org_id)
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

    /// Standard agent column list for SELECT queries
    #[allow(dead_code)]
    const AGENT_COLUMNS: &str = "id, public_id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens";

    pub async fn create_agent(&self, org_id: i64, input: CreateAgentRow) -> Result<AgentRow> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            INSERT INTO agents (org_id, public_id, name, description, system_prompt, default_model_id, tags, tools, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active')
            RETURNING id, public_id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, tools,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id)
        .bind(&input.tags)
        .bind(&input.tools)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create or update agent with a specific internal ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_agent_with_id(
        &self,
        org_id: i64,
        id: AgentId,
        input: CreateAgentRow,
    ) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            INSERT INTO agents (id, org_id, public_id, name, description, system_prompt, default_model_id, tags, tools, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'active')
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                description = EXCLUDED.description,
                system_prompt = EXCLUDED.system_prompt,
                tags = EXCLUDED.tags,
                tools = EXCLUDED.tools,
                updated_at = NOW()
            WHERE
                agents.name IS DISTINCT FROM EXCLUDED.name
                OR agents.description IS DISTINCT FROM EXCLUDED.description
                OR agents.system_prompt IS DISTINCT FROM EXCLUDED.system_prompt
                OR agents.tags IS DISTINCT FROM EXCLUDED.tags
                OR agents.tools IS DISTINCT FROM EXCLUDED.tools
            RETURNING id, public_id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, tools,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            "#,
        )
        .bind(id.uuid())
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id.map(|m| m.uuid()))
        .bind(&input.tags)
        .bind(&input.tools)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_agent(&self, org_id: i64, id: AgentId) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, public_id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, tools,
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

    pub async fn get_agent_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, public_id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, tools,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            FROM agents
            WHERE org_id = $1 AND public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_agents(&self, org_id: i64, search: Option<&str>) -> Result<Vec<AgentRow>> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 2);
        let sql = format!(
            r#"SELECT id, public_id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, tools,
                       total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
                FROM agents
                WHERE org_id = $1 AND status = 'active'{search_sql}
                ORDER BY created_at DESC"#
        );
        let mut query = sqlx::query_as::<_, AgentRow>(&sql).bind(org_id);
        for pat in &patterns {
            query = query.bind(pat);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn get_agent_by_name(&self, org_id: i64, name: &str) -> Result<Option<AgentRow>> {
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            SELECT id, public_id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, tools,
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
                tools = COALESCE($9, tools),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, public_id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, tools,
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
        .bind(&input.tools)
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

    /// Upsert agent by public_id. Returns (row, was_created).
    pub async fn upsert_agent(
        &self,
        org_id: i64,
        input: CreateAgentRow,
    ) -> Result<(AgentRow, bool)> {
        // Use CTE to detect insert vs update
        let row = sqlx::query_as::<_, AgentRow>(
            r#"
            WITH existing AS (
                SELECT id FROM agents WHERE org_id = $1 AND public_id = $2
            )
            INSERT INTO agents (org_id, public_id, name, description, system_prompt, default_model_id, tags, tools, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active')
            ON CONFLICT (org_id, public_id) DO UPDATE SET
                name = EXCLUDED.name,
                description = EXCLUDED.description,
                system_prompt = EXCLUDED.system_prompt,
                default_model_id = EXCLUDED.default_model_id,
                tags = EXCLUDED.tags,
                tools = EXCLUDED.tools,
                status = 'active',
                updated_at = NOW()
            RETURNING id, public_id, org_id, name, description, system_prompt, default_model_id, tags, status, created_at, updated_at, tools,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id)
        .bind(&input.tags)
        .bind(&input.tools)
        .fetch_one(&self.pool)
        .await?;

        // Detect if insert or update: if created_at == updated_at, it was a fresh insert
        let was_created = row.created_at == row.updated_at;
        Ok((row, was_created))
    }

    /// Get agent public_id from internal UUID (for session responses)
    pub async fn get_agent_public_id(&self, org_id: i64, id: AgentId) -> Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT public_id FROM agents WHERE org_id = $1 AND id = $2")
                .bind(org_id)
                .bind(id.uuid())
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.map(|r| r.0))
    }

    // ============================================
    // Harnesses (base configuration for sessions)
    // ============================================

    pub async fn create_harness(&self, org_id: i64, input: CreateHarnessRow) -> Result<HarnessRow> {
        let row = sqlx::query_as::<_, HarnessRow>(
            r#"
            INSERT INTO harnesses (org_id, name, description, system_prompt, default_model_id, tags, is_built_in, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'active')
            RETURNING id, org_id, name, description, system_prompt, default_model_id, tags, is_built_in, status, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id.map(|m| m.uuid()))
        .bind(&input.tags)
        .bind(input.is_built_in)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create harness with a specific ID, idempotent (ON CONFLICT DO NOTHING)
    /// Returns None if harness already exists with this ID
    /// Create or update harness with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_harness_with_id(
        &self,
        org_id: i64,
        id: HarnessId,
        input: CreateHarnessRow,
    ) -> Result<Option<HarnessRow>> {
        let row = sqlx::query_as::<_, HarnessRow>(
            r#"
            INSERT INTO harnesses (id, org_id, name, description, system_prompt, default_model_id, tags, is_built_in, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active')
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                description = EXCLUDED.description,
                system_prompt = EXCLUDED.system_prompt,
                tags = EXCLUDED.tags,
                is_built_in = EXCLUDED.is_built_in,
                updated_at = NOW()
            WHERE
                harnesses.name IS DISTINCT FROM EXCLUDED.name
                OR harnesses.description IS DISTINCT FROM EXCLUDED.description
                OR harnesses.system_prompt IS DISTINCT FROM EXCLUDED.system_prompt
                OR harnesses.tags IS DISTINCT FROM EXCLUDED.tags
                OR harnesses.is_built_in IS DISTINCT FROM EXCLUDED.is_built_in
            RETURNING id, org_id, name, description, system_prompt, default_model_id, tags, is_built_in, status, created_at, updated_at
            "#,
        )
        .bind(id.uuid())
        .bind(org_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.system_prompt)
        .bind(input.default_model_id.map(|m| m.uuid()))
        .bind(&input.tags)
        .bind(input.is_built_in)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_harness(&self, org_id: i64, id: HarnessId) -> Result<Option<HarnessRow>> {
        let row = sqlx::query_as::<_, HarnessRow>(
            r#"
            SELECT id, org_id, name, description, system_prompt, default_model_id, tags, is_built_in, status, created_at, updated_at
            FROM harnesses
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id.uuid())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_harnesses(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<HarnessRow>> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 2);
        let sql = format!(
            r#"SELECT id, org_id, name, description, system_prompt, default_model_id, tags, is_built_in, status, created_at, updated_at
                FROM harnesses
                WHERE org_id = $1 AND status = 'active'{search_sql}
                ORDER BY created_at DESC"#
        );
        let mut query = sqlx::query_as::<_, HarnessRow>(&sql).bind(org_id);
        for pat in &patterns {
            query = query.bind(pat);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn update_harness(
        &self,
        org_id: i64,
        id: HarnessId,
        input: UpdateHarness,
    ) -> Result<Option<HarnessRow>> {
        let row = sqlx::query_as::<_, HarnessRow>(
            r#"
            UPDATE harnesses
            SET
                name = COALESCE($3, name),
                description = COALESCE($4, description),
                system_prompt = COALESCE($5, system_prompt),
                default_model_id = COALESCE($6, default_model_id),
                tags = COALESCE($7, tags),
                status = COALESCE($8, status),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, name, description, system_prompt, default_model_id, tags, is_built_in, status, created_at, updated_at
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

    pub async fn delete_harness(&self, org_id: i64, id: HarnessId) -> Result<bool> {
        // Archive instead of hard delete
        let result = sqlx::query(
            r#"
            UPDATE harnesses
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
            INSERT INTO sessions (org_id, harness_id, agent_id, title, tags, model_id, capabilities, tools, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'started')
            RETURNING id, org_id, harness_id, agent_id, title, tags, model_id, capabilities, tools, status, created_at, updated_at, started_at, finished_at,
                      total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            "#,
        )
        .bind(input.org_id)
        .bind(input.harness_id.map(|h| h.uuid()))
        .bind(input.agent_id.map(|a| a.uuid()))
        .bind(&input.title)
        .bind(&input.tags)
        .bind(input.model_id)
        .bind(&input.capabilities)
        .bind(&input.tools)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get session by org and session id
    pub async fn get_session(&self, org_id: i64, id: SessionId) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, harness_id, agent_id, title, tags, model_id, capabilities, tools, status, created_at, updated_at, started_at, finished_at,
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

    /// Get session without org scoping. For internal system use only (e.g. usage tracking).
    pub async fn get_session_unscoped(&self, id: SessionId) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, harness_id, agent_id, title, tags, model_id, capabilities, tools, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            FROM sessions
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List sessions for an organization with optional agent and search filters.
    /// Returns (sessions, total_count).
    pub async fn list_sessions(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
        search: Option<&str>,
        pagination: crate::api::common::Pagination,
    ) -> Result<(Vec<SessionRow>, u32)> {
        // Build WHERE clause dynamically
        let mut where_clause = "WHERE org_id = $1".to_string();
        let mut param_idx = 2;

        if agent_id.is_some() {
            where_clause.push_str(&format!(" AND agent_id = ${param_idx}"));
            param_idx += 1;
        }

        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(COALESCE(title, ''))", param_idx);
        where_clause.push_str(&search_sql);
        param_idx += patterns.len();

        // Helper: bind org_id, agent_id, and search patterns to a query
        macro_rules! bind_params {
            ($q:expr) => {{
                let mut q = $q.bind(org_id);
                if let Some(aid) = agent_id {
                    q = q.bind(aid);
                }
                for pat in &patterns {
                    q = q.bind(pat);
                }
                q
            }};
        }

        // Get total count
        let count_sql = format!("SELECT COUNT(*) as count FROM sessions {where_clause}");
        let count_query = bind_params!(sqlx::query_as::<_, (i64,)>(&count_sql));
        let total: (i64,) = count_query.fetch_one(&self.pool).await?;

        // Get paginated results
        let limit_idx = param_idx;
        let offset_idx = param_idx + 1;
        let select_sql = format!(
            r#"SELECT id, org_id, harness_id, agent_id, title, tags, model_id, capabilities, tools, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            FROM sessions {where_clause}
            ORDER BY created_at DESC
            LIMIT ${limit_idx} OFFSET ${offset_idx}"#,
        );
        let data_query = bind_params!(sqlx::query_as::<_, SessionRow>(&select_sql));
        let rows: Vec<SessionRow> = data_query
            .bind(pagination.limit as i64)
            .bind(pagination.offset as i64)
            .fetch_all(&self.pool)
            .await?;

        Ok((rows, total.0 as u32))
    }

    /// Count sessions grouped by status for an organization.
    pub async fn count_sessions_by_status(&self, org_id: i64) -> Result<Vec<(String, i64)>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT status, COUNT(*) as count
            FROM sessions
            WHERE org_id = $1
            GROUP BY status
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Find a single session matching ALL given tags within an org.
    /// Used for singleton patterns like global chat (one session per user per org).
    pub async fn find_session_by_tags(
        &self,
        org_id: i64,
        tags: &[String],
    ) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, harness_id, agent_id, title, tags, model_id, capabilities, tools, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            FROM sessions
            WHERE org_id = $1 AND tags @> $2
            ORDER BY created_at ASC
            LIMIT 1
            "#,
        )
        .bind(org_id)
        .bind(tags)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Find all active sessions with Slack tags (for startup recovery).
    /// Returns sessions where status = 'active' and any tag starts with 'slack:app:'.
    pub async fn find_active_slack_sessions(&self) -> Result<Vec<SessionRow>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, org_id, harness_id, agent_id, title, tags, model_id, capabilities, tools, status, created_at, updated_at, started_at, finished_at,
                   total_input_tokens, total_output_tokens, total_cache_read_tokens, total_cache_creation_tokens
            FROM sessions
            WHERE status = 'active'
              AND EXISTS (
                  SELECT 1 FROM unnest(tags) AS t
                  WHERE t LIKE 'slack:app:%'
              )
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
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
            RETURNING id, org_id, harness_id, agent_id, title, tags, model_id, capabilities, tools, status, created_at, updated_at, started_at, finished_at,
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
    // Pinned Sessions
    // ============================================

    /// Pin a session for a user
    pub async fn pin_session(
        &self,
        user_id: Uuid,
        session_id: SessionId,
        org_id: i64,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO pinned_sessions (user_id, session_id, org_id)
            VALUES ($1, $2, $3)
            ON CONFLICT (user_id, session_id) DO NOTHING
            "#,
        )
        .bind(user_id)
        .bind(session_id)
        .bind(org_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Unpin a session for a user
    pub async fn unpin_session(&self, user_id: Uuid, session_id: SessionId) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM pinned_sessions
            WHERE user_id = $1 AND session_id = $2
            "#,
        )
        .bind(user_id)
        .bind(session_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Get the set of pinned session IDs for a user in an org
    pub async fn list_pinned_session_ids(
        &self,
        user_id: Uuid,
        org_id: i64,
    ) -> Result<Vec<SessionId>> {
        let rows: Vec<(SessionId,)> = sqlx::query_as(
            r#"
            SELECT session_id
            FROM pinned_sessions
            WHERE user_id = $1 AND org_id = $2
            ORDER BY pinned_at DESC
            "#,
        )
        .bind(user_id)
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    // ============================================
    // Notifications
    // ============================================

    pub async fn create_notification_turn_request(
        &self,
        input: CreateNotificationTurnRequestRow,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO notification_turn_requests (input_message_id, org_id, user_id, session_id)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (input_message_id) DO UPDATE SET
                org_id = EXCLUDED.org_id,
                user_id = EXCLUDED.user_id,
                session_id = EXCLUDED.session_id
            "#,
        )
        .bind(input.input_message_id)
        .bind(input.org_id)
        .bind(input.user_id)
        .bind(input.session_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_notification_turn_request(
        &self,
        input_message_id: MessageId,
    ) -> Result<Option<NotificationTurnRequestRow>> {
        sqlx::query_as::<_, NotificationTurnRequestRow>(
            r#"
            SELECT input_message_id, org_id, user_id, session_id, created_at
            FROM notification_turn_requests
            WHERE input_message_id = $1
            "#,
        )
        .bind(input_message_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn create_notification(
        &self,
        input: CreateNotificationRow,
    ) -> Result<NotificationRow> {
        let row = sqlx::query_as::<_, NotificationRow>(
            r#"
            INSERT INTO notifications (
                org_id, user_id, kind, title, body, target_type, target_id, href, payload, dedupe_key
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (org_id, user_id, dedupe_key)
                WHERE dedupe_key IS NOT NULL AND viewed_at IS NULL
            DO UPDATE SET
                title = EXCLUDED.title,
                body = EXCLUDED.body,
                target_type = EXCLUDED.target_type,
                target_id = EXCLUDED.target_id,
                href = EXCLUDED.href,
                payload = EXCLUDED.payload,
                occurrence_count = notifications.occurrence_count + 1,
                updated_at = NOW()
            RETURNING
                id, org_id, user_id, kind, title, body, target_type, target_id, href,
                payload, dedupe_key, occurrence_count, viewed_at, created_at, updated_at
            "#,
        )
        .bind(input.org_id)
        .bind(input.user_id)
        .bind(&input.kind)
        .bind(&input.title)
        .bind(&input.body)
        .bind(&input.target_type)
        .bind(&input.target_id)
        .bind(&input.href)
        .bind(&input.payload)
        .bind(&input.dedupe_key)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_notification(
        &self,
        org_id: i64,
        user_id: Uuid,
        id: NotificationId,
    ) -> Result<Option<NotificationRow>> {
        sqlx::query_as::<_, NotificationRow>(
            r#"
            SELECT
                id, org_id, user_id, kind, title, body, target_type, target_id, href,
                payload, dedupe_key, occurrence_count, viewed_at, created_at, updated_at
            FROM notifications
            WHERE org_id = $1 AND user_id = $2 AND id = $3
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list_notifications(
        &self,
        org_id: i64,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<NotificationRow>> {
        sqlx::query_as::<_, NotificationRow>(
            r#"
            SELECT
                id, org_id, user_id, kind, title, body, target_type, target_id, href,
                payload, dedupe_key, occurrence_count, viewed_at, created_at, updated_at
            FROM notifications
            WHERE org_id = $1 AND user_id = $2
            ORDER BY created_at DESC, id DESC
            LIMIT $3
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list_notifications_updated_since(
        &self,
        org_id: i64,
        user_id: Uuid,
        updated_since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<NotificationRow>> {
        sqlx::query_as::<_, NotificationRow>(
            r#"
            SELECT
                id, org_id, user_id, kind, title, body, target_type, target_id, href,
                payload, dedupe_key, occurrence_count, viewed_at, created_at, updated_at
            FROM notifications
            WHERE org_id = $1
              AND user_id = $2
              AND ($3::timestamptz IS NULL OR updated_at >= $3)
            ORDER BY updated_at ASC, id ASC
            LIMIT $4
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(updated_since)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn count_unviewed_notifications(&self, org_id: i64, user_id: Uuid) -> Result<u32> {
        let (count,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)::BIGINT
            FROM notifications
            WHERE org_id = $1 AND user_id = $2 AND viewed_at IS NULL
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count as u32)
    }

    pub async fn count_unviewed_notifications_by_kind(
        &self,
        org_id: i64,
        user_id: Uuid,
        kind: &str,
    ) -> Result<u32> {
        let (count,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)::BIGINT
            FROM notifications
            WHERE org_id = $1 AND user_id = $2 AND kind = $3 AND viewed_at IS NULL
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(kind)
        .fetch_one(&self.pool)
        .await?;
        Ok(count as u32)
    }

    pub async fn mark_notification_viewed(
        &self,
        org_id: i64,
        user_id: Uuid,
        id: NotificationId,
    ) -> Result<Option<NotificationRow>> {
        sqlx::query_as::<_, NotificationRow>(
            r#"
            UPDATE notifications
            SET
                viewed_at = COALESCE(viewed_at, NOW()),
                updated_at = NOW()
            WHERE org_id = $1 AND user_id = $2 AND id = $3
            RETURNING
                id, org_id, user_id, kind, title, body, target_type, target_id, href,
                payload, dedupe_key, occurrence_count, viewed_at, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
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

    /// Check if an input.message event with a given slack_ts already exists in a session.
    /// Used for dedup when Slack sends duplicate events (app_mention + message).
    pub async fn has_event_with_slack_ts(
        &self,
        session_id: SessionId,
        slack_ts: &str,
    ) -> Result<bool> {
        let pattern = serde_json::json!({
            "message": { "metadata": { "slack_ts": slack_ts } }
        });
        let row: (bool,) = sqlx::query_as(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM events
                WHERE session_id = $1
                  AND event_type = 'input.message'
                  AND data @> $2
            )
            "#,
        )
        .bind(session_id)
        .bind(&pattern)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_events(
        &self,
        session_id: SessionId,
        since_sequence: Option<i32>,
        since_id: Option<EventId>,
        filter_types: &[String],
        exclude_types: &[String],
        before_sequence: Option<i32>,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>> {
        // UUID v7 is NOT guaranteed monotonically increasing across concurrent inserts,
        // so we always filter and order by the dedicated sequence column.
        // When since_id is provided, resolve it to sequence via an inline subquery (PK lookup).
        // filter_types: positive filter — when non-empty, only return matching event types.
        // exclude_types: negative filter — remove matching event types from results.
        // When both are provided, filter_types narrows first, then exclude_types removes.
        //
        // Backward pagination: when `limit` is provided (with optional `before_sequence`),
        // fetch the last N events by using ORDER BY sequence DESC LIMIT N, then reverse
        // so results are returned oldest→newest.
        if let Some(limit) = limit {
            // Backward pagination: fetch last N events before a cursor
            let rows = if let Some(before_seq) = before_sequence {
                sqlx::query_as::<_, EventRow>(
                    r#"
                    SELECT * FROM (
                        SELECT id, session_id, sequence, event_type, ts, context, data, metadata, tags, created_at
                        FROM events
                        WHERE session_id = $1
                          AND sequence < $2
                          AND (cardinality($3::text[]) = 0 OR event_type = ANY($3))
                          AND (cardinality($4::text[]) = 0 OR event_type <> ALL($4))
                        ORDER BY sequence DESC
                        LIMIT $5
                    ) batch
                    ORDER BY sequence ASC
                    "#,
                )
                .bind(session_id.uuid())
                .bind(before_seq)
                .bind(filter_types)
                .bind(exclude_types)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            } else {
                // No cursor: fetch the last N events
                sqlx::query_as::<_, EventRow>(
                    r#"
                    SELECT * FROM (
                        SELECT id, session_id, sequence, event_type, ts, context, data, metadata, tags, created_at
                        FROM events
                        WHERE session_id = $1
                          AND (cardinality($2::text[]) = 0 OR event_type = ANY($2))
                          AND (cardinality($3::text[]) = 0 OR event_type <> ALL($3))
                        ORDER BY sequence DESC
                        LIMIT $4
                    ) batch
                    ORDER BY sequence ASC
                    "#,
                )
                .bind(session_id.uuid())
                .bind(filter_types)
                .bind(exclude_types)
                .bind(limit as i64)
                .fetch_all(&self.pool)
                .await?
            };
            return Ok(rows);
        }

        // Safety-limited forward query (backward compat when no limit param).
        // Cap at 10 000 rows to prevent unbounded result sets.
        const FORWARD_SAFETY_LIMIT: i64 = 10_000;
        let rows = match (since_id, since_sequence) {
            (Some(id), _) => {
                sqlx::query_as::<_, EventRow>(
                    r#"
                    SELECT id, session_id, sequence, event_type, ts, context, data, metadata, tags, created_at
                    FROM events
                    WHERE session_id = $1
                      AND sequence > (SELECT sequence FROM events WHERE id = $2)
                      AND (cardinality($3::text[]) = 0 OR event_type = ANY($3))
                      AND (cardinality($4::text[]) = 0 OR event_type <> ALL($4))
                    ORDER BY sequence ASC
                    LIMIT $5
                    "#,
                )
                .bind(session_id.uuid())
                .bind(id.uuid())
                .bind(filter_types)
                .bind(exclude_types)
                .bind(FORWARD_SAFETY_LIMIT)
                .fetch_all(&self.pool)
                .await?
            }
            (None, Some(seq)) => {
                sqlx::query_as::<_, EventRow>(
                    r#"
                    SELECT id, session_id, sequence, event_type, ts, context, data, metadata, tags, created_at
                    FROM events
                    WHERE session_id = $1 AND sequence > $2
                      AND (cardinality($3::text[]) = 0 OR event_type = ANY($3))
                      AND (cardinality($4::text[]) = 0 OR event_type <> ALL($4))
                    ORDER BY sequence ASC
                    LIMIT $5
                    "#,
                )
                .bind(session_id.uuid())
                .bind(seq)
                .bind(filter_types)
                .bind(exclude_types)
                .bind(FORWARD_SAFETY_LIMIT)
                .fetch_all(&self.pool)
                .await?
            }
            (None, None) => {
                sqlx::query_as::<_, EventRow>(
                    r#"
                    SELECT id, session_id, sequence, event_type, ts, context, data, metadata, tags, created_at
                    FROM events
                    WHERE session_id = $1
                      AND (cardinality($2::text[]) = 0 OR event_type = ANY($2))
                      AND (cardinality($3::text[]) = 0 OR event_type <> ALL($3))
                    ORDER BY sequence ASC
                    LIMIT $4
                    "#,
                )
                .bind(session_id.uuid())
                .bind(filter_types)
                .bind(exclude_types)
                .bind(FORWARD_SAFETY_LIMIT)
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(rows)
    }

    /// Count events for a session using SELECT COUNT(*) — no row materialization.
    /// When `exclude_types` is non-empty, excludes matching event types from the count.
    pub async fn count_events(
        &self,
        session_id: SessionId,
        exclude_types: &[String],
    ) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM events
            WHERE session_id = $1
              AND (cardinality($2::text[]) = 0 OR event_type <> ALL($2))
            "#,
        )
        .bind(session_id.uuid())
        .bind(exclude_types)
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    /// Find the nearest turn.started sequence at or before the given sequence.
    /// Used for turn boundary snapping in pagination.
    pub async fn find_turn_boundary(
        &self,
        session_id: SessionId,
        before_sequence: i32,
    ) -> Result<Option<i32>> {
        let row: Option<(i32,)> = sqlx::query_as(
            r#"
            SELECT sequence
            FROM events
            WHERE session_id = $1
              AND sequence <= $2
              AND event_type = 'turn.started'
            ORDER BY sequence DESC
            LIMIT 1
            "#,
        )
        .bind(session_id.uuid())
        .bind(before_sequence)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.0))
    }

    /// List only message events for a session (for MessageStore implementation)
    ///
    /// Returns events with types: input.message, output.message.completed, tool.completed
    /// Ordered by sequence for conversation reconstruction.
    /// Note: Tool calls are embedded in output.message.completed events via ContentPart::ToolCall.
    /// Note: Tool results come from tool.completed events (not message.tool_result).
    pub async fn list_message_events(&self, session_id: SessionId) -> Result<Vec<EventRow>> {
        self.list_message_events_limited(session_id, None).await
    }

    /// List message events with an optional limit.
    /// When `limit` is Some, returns the most recent N messages (by sequence)
    /// in ascending order for correct conversation reconstruction.
    pub async fn list_message_events_limited(
        &self,
        session_id: SessionId,
        limit: Option<i32>,
    ) -> Result<Vec<EventRow>> {
        let rows = if let Some(limit) = limit {
            // Subquery: get most recent N by sequence DESC, then re-order ASC
            sqlx::query_as::<_, EventRow>(
                r#"
                SELECT * FROM (
                    SELECT id, session_id, sequence, event_type, ts, context, data, metadata, tags, created_at
                    FROM events
                    WHERE session_id = $1
                      AND event_type IN ('input.message', 'output.message.completed', 'tool.completed')
                    ORDER BY sequence DESC
                    LIMIT $2
                ) recent
                ORDER BY sequence ASC
                "#,
            )
            .bind(session_id.uuid())
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            // Safety cap when no explicit limit — prevents unbounded result sets.
            const MESSAGE_SAFETY_LIMIT: i64 = 5_000;
            sqlx::query_as::<_, EventRow>(
                r#"
                SELECT id, session_id, sequence, event_type, ts, context, data, metadata, tags, created_at
                FROM events
                WHERE session_id = $1
                  AND event_type IN ('input.message', 'output.message.completed', 'tool.completed')
                ORDER BY sequence ASC
                LIMIT $2
                "#,
            )
            .bind(session_id.uuid())
            .bind(MESSAGE_SAFETY_LIMIT)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }

    /// Count message events for a session using SELECT COUNT(*) — no row materialization.
    pub async fn count_message_events(&self, session_id: SessionId) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM events
            WHERE session_id = $1
              AND event_type IN ('input.message', 'output.message.completed', 'tool.completed')
            "#,
        )
        .bind(session_id.uuid())
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
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
                        " AND search_vector @@ plainto_tsquery('english', ${})",
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
            WITH clear_default AS (
                UPDATE llm_models
                SET is_default = FALSE, updated_at = NOW()
                WHERE is_default = TRUE AND org_id = $1 AND $6 = TRUE
            )
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
            WITH clear_default AS (
                UPDATE llm_models
                SET is_default = FALSE, updated_at = NOW()
                WHERE is_default = TRUE AND org_id = $2 AND id != $1 AND $7 = TRUE
            )
            INSERT INTO llm_models (id, org_id, provider_id, model_id, display_name, capabilities, is_default, is_favorite, source, provider_metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO UPDATE SET
                display_name = EXCLUDED.display_name,
                is_default = EXCLUDED.is_default,
                is_favorite = EXCLUDED.is_favorite,
                updated_at = NOW()
            WHERE
                llm_models.display_name IS DISTINCT FROM EXCLUDED.display_name
                OR llm_models.is_default IS DISTINCT FROM EXCLUDED.is_default
                OR llm_models.is_favorite IS DISTINCT FROM EXCLUDED.is_favorite
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

    pub async fn get_llm_model(&self, org_id: i64, id: Uuid) -> Result<Option<LlmModelRow>> {
        let row = sqlx::query_as::<_, LlmModelRow>(
            r#"
            SELECT id, org_id, provider_id, model_id, display_name, capabilities, is_default, is_favorite, status, source, last_seen_at, provider_metadata, created_at, updated_at
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
            SELECT m.id, m.org_id, m.provider_id, m.model_id, m.display_name, m.capabilities, m.is_default, m.is_favorite, m.status, m.source, m.last_seen_at, m.provider_metadata, m.created_at, m.updated_at,
                   p.name as provider_name, p.provider_type
            FROM llm_models m
            JOIN llm_providers p ON m.provider_id = p.id
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
            SELECT id, org_id, provider_id, model_id, display_name, capabilities, is_default, is_favorite, status, source, last_seen_at, provider_metadata, created_at, updated_at
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
            WITH clear_default AS (
                UPDATE llm_models
                SET is_default = FALSE, updated_at = NOW()
                WHERE is_default = TRUE AND org_id = $1 AND id != $2 AND $6 = TRUE
            )
            UPDATE llm_models
            SET
                model_id = COALESCE($3, model_id),
                display_name = COALESCE($4, display_name),
                capabilities = COALESCE($5, capabilities),
                is_default = COALESCE($6, is_default),
                is_favorite = COALESCE($7, is_favorite),
                status = COALESCE($8, status),
                last_seen_at = COALESCE($9, last_seen_at),
                provider_metadata = COALESCE($10, provider_metadata),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, provider_id, model_id, display_name, capabilities, is_default, is_favorite, status, source, last_seen_at, provider_metadata, created_at, updated_at
            "#,
        )
        .bind(org_id)
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
    // Harness Capabilities
    // ============================================

    /// Get capabilities for a harness, ordered by position
    pub async fn get_harness_capabilities(
        &self,
        harness_id: Uuid,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        let rows = sqlx::query_as::<_, HarnessCapabilityRow>(
            r#"
            SELECT id, harness_id, capability_id, position, config, created_at
            FROM harness_capabilities
            WHERE harness_id = $1
            ORDER BY position ASC
            "#,
        )
        .bind(harness_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Set capabilities for a harness (replaces existing capabilities)
    /// capabilities: list of (capability_id, position, config) tuples
    pub async fn set_harness_capabilities(
        &self,
        harness_id: Uuid,
        capabilities: Vec<(String, i32, serde_json::Value)>,
    ) -> Result<Vec<HarnessCapabilityRow>> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM harness_capabilities WHERE harness_id = $1")
            .bind(harness_id)
            .execute(&mut *tx)
            .await?;

        for (capability_id, position, config) in &capabilities {
            sqlx::query(
                r#"
                INSERT INTO harness_capabilities (harness_id, capability_id, position, config)
                VALUES ($1, $2, $3, $4)
                "#,
            )
            .bind(harness_id)
            .bind(capability_id)
            .bind(position)
            .bind(config)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        self.get_harness_capabilities(harness_id).await
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

    /// Check if a path or its subtree contains any readonly files
    pub async fn has_readonly_session_files(&self, session_id: Uuid, path: &str) -> Result<bool> {
        let pattern = if path == "/" {
            "^/".to_string()
        } else {
            format!("^{}(/|$)", regex::escape(path))
        };

        let result: Option<(bool,)> = sqlx::query_as(
            "SELECT TRUE FROM session_files WHERE session_id = $1 AND path ~ $2 AND is_readonly = true LIMIT 1",
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
    /// Create or update MCP server with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
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
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                description = EXCLUDED.description,
                url = EXCLUDED.url,
                transport_type = EXCLUDED.transport_type,
                updated_at = NOW()
            WHERE
                mcp_servers.name IS DISTINCT FROM EXCLUDED.name
                OR mcp_servers.description IS DISTINCT FROM EXCLUDED.description
                OR mcp_servers.url IS DISTINCT FROM EXCLUDED.url
                OR mcp_servers.transport_type IS DISTINCT FROM EXCLUDED.transport_type
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

    pub async fn get_mcp_server(&self, org_id: i64, id: Uuid) -> Result<Option<McpServerRow>> {
        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
            FROM mcp_servers
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Batch fetch multiple MCP servers by IDs in a single query.
    pub async fn get_mcp_servers_batch(
        &self,
        org_id: i64,
        ids: &[Uuid],
    ) -> Result<Vec<McpServerRow>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let rows = sqlx::query_as::<_, McpServerRow>(
            r#"
            SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
            FROM mcp_servers
            WHERE org_id = $1 AND id = ANY($2)
            "#,
        )
        .bind(org_id)
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn get_mcp_server_by_name(
        &self,
        org_id: i64,
        name: &str,
    ) -> Result<Option<McpServerRow>> {
        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
            FROM mcp_servers
            WHERE org_id = $1 AND name = $2
            "#,
        )
        .bind(org_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_mcp_servers(
        &self,
        org_id: i64,
        search: Option<&str>,
    ) -> Result<Vec<McpServerRow>> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 2);
        let sql = format!(
            r#"SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
                FROM mcp_servers
                WHERE org_id = $1{search_sql}
                ORDER BY created_at DESC"#
        );
        let mut query = sqlx::query_as::<_, McpServerRow>(&sql).bind(org_id);
        for pat in &patterns {
            query = query.bind(pat);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    /// List only active MCP servers (for capability listing)
    pub async fn list_active_mcp_servers(&self, org_id: i64) -> Result<Vec<McpServerRow>> {
        let rows = sqlx::query_as::<_, McpServerRow>(
            r#"
            SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
            FROM mcp_servers
            WHERE org_id = $1 AND status = 'active'
            ORDER BY name ASC
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_mcp_server(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMcpServer,
    ) -> Result<Option<McpServerRow>> {
        // Handle api_key_set: if we're updating the encrypted key, also update the flag
        let api_key_set = input.api_key_encrypted.as_ref().map(|_| true);

        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            UPDATE mcp_servers
            SET
                name = COALESCE($3, name),
                description = COALESCE($4, description),
                url = COALESCE($5, url),
                transport_type = COALESCE($6, transport_type),
                status = COALESCE($7, status),
                api_key_encrypted = COALESCE($8, api_key_encrypted),
                api_key_set = COALESCE($9, api_key_set),
                headers = COALESCE($10, headers),
                settings = COALESCE($11, settings)
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
            "#,
        )
        .bind(org_id)
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
        org_id: i64,
        id: Uuid,
        input: UpdateMcpServerTools,
    ) -> Result<Option<McpServerRow>> {
        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            UPDATE mcp_servers
            SET
                cached_tools = $3,
                tools_cached_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.cached_tools)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_mcp_server(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM mcp_servers WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Skills
    // ============================================

    pub async fn create_skill(&self, org_id: i64, input: CreateSkillRow) -> Result<SkillRow> {
        let row = sqlx::query_as::<_, SkillRow>(
            r#"
            INSERT INTO skills (org_id, public_id, name, description, license, compatibility, metadata, allowed_tools, instructions, source_type, archive_data, version)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING id, public_id, org_id, name, description, license, compatibility, metadata, allowed_tools, instructions, source_type, archive_data, status, version, created_at, updated_at
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
            SELECT id, public_id, org_id, name, description, license, compatibility, metadata, allowed_tools, instructions, source_type, archive_data, status, version, created_at, updated_at
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
            SELECT id, public_id, org_id, name, description, license, compatibility, metadata, allowed_tools, instructions, source_type, archive_data, status, version, created_at, updated_at
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

    pub async fn list_skills(&self, org_id: i64, search: Option<&str>) -> Result<Vec<SkillRow>> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 2);
        let sql = format!(
            r#"SELECT id, public_id, org_id, name, description, license, compatibility, metadata, allowed_tools, instructions, source_type, archive_data, status, version, created_at, updated_at
                FROM skills
                WHERE org_id = $1{search_sql}
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
            RETURNING id, public_id, org_id, name, description, license, compatibility, metadata, allowed_tools, instructions, source_type, archive_data, status, version, created_at, updated_at
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
        let result = sqlx::query("DELETE FROM skills WHERE id = $1 AND org_id = $2")
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

    // ============================================
    // Organizations
    // ============================================

    pub async fn create_organization(
        &self,
        input: CreateOrganizationRow,
    ) -> Result<OrganizationRow> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            INSERT INTO organizations (public_id, name, created_by)
            VALUES ($1, $2, $3)
            RETURNING org_id, public_id, name, created_at, updated_at, external_id, created_by
            "#,
        )
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(input.created_by)
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
            INSERT INTO organizations (org_id, public_id, name, created_by)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (org_id) DO NOTHING
            RETURNING org_id, public_id, name, created_at, updated_at, external_id, created_by
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(input.created_by)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_organization(&self, org_id: i64) -> Result<Option<OrganizationRow>> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            SELECT org_id, public_id, name, created_at, updated_at, external_id
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
            SELECT org_id, public_id, name, created_at, updated_at, external_id
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
            SELECT org_id, public_id, name, created_at, updated_at, external_id
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
            RETURNING org_id, public_id, name, created_at, updated_at, external_id
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
        role: &str,
    ) -> Result<OrganizationMemberRow> {
        let row = sqlx::query_as::<_, OrganizationMemberRow>(
            r#"
            INSERT INTO organization_members (org_id, user_id, role)
            VALUES ($1, $2, $3)
            ON CONFLICT (org_id, user_id) DO UPDATE SET role = EXCLUDED.role
            RETURNING org_id, user_id, role, created_at
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(role)
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
            SELECT org_id, user_id, role, created_at
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

    /// List members with user info (for API responses)
    pub async fn list_organization_members_with_users(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrganizationMemberWithUserRow>> {
        let rows = sqlx::query_as::<_, OrganizationMemberWithUserRow>(
            r#"
            SELECT u.id as user_id, u.email, u.name, u.avatar_url, om.role, om.created_at as joined_at
            FROM organization_members om
            JOIN users u ON u.id = om.user_id
            WHERE om.org_id = $1
            ORDER BY om.created_at
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Get a specific organization member with user info
    pub async fn get_organization_member(
        &self,
        org_id: i64,
        user_id: Uuid,
    ) -> Result<Option<OrganizationMemberWithUserRow>> {
        let row = sqlx::query_as::<_, OrganizationMemberWithUserRow>(
            r#"
            SELECT u.id as user_id, u.email, u.name, u.avatar_url, om.role, om.created_at as joined_at
            FROM organization_members om
            JOIN users u ON u.id = om.user_id
            WHERE om.org_id = $1 AND om.user_id = $2
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Update a member's role
    pub async fn update_organization_member_role(
        &self,
        org_id: i64,
        user_id: Uuid,
        role: &str,
    ) -> Result<Option<OrganizationMemberRow>> {
        let row = sqlx::query_as::<_, OrganizationMemberRow>(
            r#"
            UPDATE organization_members SET role = $3
            WHERE org_id = $1 AND user_id = $2
            RETURNING org_id, user_id, role, created_at
            "#,
        )
        .bind(org_id)
        .bind(user_id)
        .bind(role)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Count owners in an organization (for preventing last owner removal)
    pub async fn count_organization_owners(&self, org_id: i64) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM organization_members WHERE org_id = $1 AND role = 'owner'",
        )
        .bind(org_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0)
    }

    pub async fn list_user_organizations(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<OrganizationWithRoleRow>> {
        let rows = sqlx::query_as::<_, OrganizationWithRoleRow>(
            r#"
            SELECT o.org_id, o.public_id, o.name, om.role
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

    /// Get user by external identity provider ID
    pub async fn get_user_by_external_id(&self, external_id: &str) -> Result<Option<UserRow>> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            SELECT id, email, name, avatar_url, roles, password_hash, email_verified, auth_provider, auth_provider_id, created_at, updated_at, external_id
            FROM users
            WHERE external_id = $1
            "#,
        )
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get organization by external identity provider ID
    pub async fn get_organization_by_external_id(
        &self,
        external_id: &str,
    ) -> Result<Option<OrganizationRow>> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            SELECT org_id, public_id, name, created_at, updated_at, external_id
            FROM organizations
            WHERE external_id = $1
            "#,
        )
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Upsert organization by external ID (for external auth provider sync)
    pub async fn upsert_org_by_external_id(
        &self,
        external_id: &str,
        public_id: &str,
        name: &str,
    ) -> Result<OrganizationRow> {
        let row = sqlx::query_as::<_, OrganizationRow>(
            r#"
            INSERT INTO organizations (public_id, name, external_id)
            VALUES ($1, $2, $3)
            ON CONFLICT (external_id) DO UPDATE SET name = EXCLUDED.name, updated_at = NOW()
            RETURNING org_id, public_id, name, created_at, updated_at, external_id
            "#,
        )
        .bind(public_id)
        .bind(name)
        .bind(external_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Ensure user is a member of organization (idempotent)
    pub async fn ensure_membership(&self, user_id: Uuid, org_id: i64, role: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO organization_members (org_id, user_id, role) VALUES ($1, $2, $3) ON CONFLICT (org_id, user_id) DO NOTHING"
        )
        .bind(org_id)
        .bind(user_id)
        .bind(role)
        .execute(&self.pool)
        .await?;

        Ok(())
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

    // ============================================
    // User Connections
    // ============================================

    /// Create or replace a user connection for a provider.
    /// Deletes any existing connection for the same (user_id, provider) first.
    pub async fn upsert_user_connection(
        &self,
        input: CreateUserConnectionRow,
    ) -> Result<UserConnectionRow> {
        // App-level uniqueness: delete existing connection for this user+provider
        sqlx::query("DELETE FROM user_connections WHERE user_id = $1 AND provider = $2")
            .bind(input.user_id)
            .bind(&input.provider)
            .execute(&self.pool)
            .await?;

        let row = sqlx::query_as::<_, UserConnectionRow>(
            r#"
            INSERT INTO user_connections (user_id, provider, connection_type, provider_user_id, provider_username, access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING id, user_id, provider, connection_type, provider_user_id, provider_username, access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, created_at, updated_at
            "#,
        )
        .bind(input.user_id)
        .bind(&input.provider)
        .bind(&input.connection_type)
        .bind(&input.provider_user_id)
        .bind(&input.provider_username)
        .bind(&input.access_token_encrypted)
        .bind(&input.refresh_token_encrypted)
        .bind(&input.scopes)
        .bind(input.expires_at)
        .bind(input.installation_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get a user's connection for a specific provider
    pub async fn get_user_connection(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<UserConnectionRow>> {
        let row = sqlx::query_as::<_, UserConnectionRow>(
            r#"
            SELECT id, user_id, provider, connection_type, provider_user_id, provider_username, access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, created_at, updated_at
            FROM user_connections
            WHERE user_id = $1 AND provider = $2
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List all connections for a user
    pub async fn list_user_connections(&self, user_id: Uuid) -> Result<Vec<UserConnectionRow>> {
        let rows = sqlx::query_as::<_, UserConnectionRow>(
            r#"
            SELECT id, user_id, provider, connection_type, provider_user_id, provider_username, access_token_encrypted, refresh_token_encrypted, scopes, expires_at, installation_id, created_at, updated_at
            FROM user_connections
            WHERE user_id = $1
            ORDER BY provider ASC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Get the encrypted connection token for a session's org member.
    /// Joins session → org_members → user_connections to resolve lazily.
    /// Returns None for GitHub App connections (use get_installation_id_for_session instead).
    pub async fn get_connection_token_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<Vec<u8>>> {
        let row: Option<(Option<Vec<u8>>,)> = sqlx::query_as(
            r#"
            SELECT uc.access_token_encrypted
            FROM sessions s
            JOIN organization_members om ON om.org_id = s.org_id
            JOIN user_connections uc ON uc.user_id = om.user_id AND uc.provider = $2
            WHERE s.id = $1
              AND uc.access_token_encrypted IS NOT NULL
            LIMIT 1
            "#,
        )
        .bind(session_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|(blob,)| blob))
    }

    /// Get the GitHub App installation ID for a session's org member.
    /// Used by the connection resolver to mint fresh installation tokens.
    pub async fn get_installation_id_for_session(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<i64>> {
        let row: Option<(i64,)> = sqlx::query_as(
            r#"
            SELECT uc.installation_id
            FROM sessions s
            JOIN organization_members om ON om.org_id = s.org_id
            JOIN user_connections uc ON uc.user_id = om.user_id AND uc.provider = $2
            WHERE s.id = $1
              AND uc.installation_id IS NOT NULL
            LIMIT 1
            "#,
        )
        .bind(session_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(id,)| id))
    }

    /// Delete a user's connection for a specific provider
    pub async fn delete_user_connection(&self, user_id: Uuid, provider: &str) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM user_connections WHERE user_id = $1 AND provider = $2")
                .bind(user_id)
                .bind(provider)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() > 0)
    }

    // ============================================
    // Session Schedules
    // ============================================

    pub async fn create_session_schedule(
        &self,
        input: CreateSessionScheduleRow,
    ) -> Result<SessionScheduleRow> {
        let id = ScheduleId::new();
        let public_id = id.to_string();

        let row = sqlx::query_as::<_, SessionScheduleRow>(
            r#"
            INSERT INTO session_schedules (id, public_id, org_id, session_id, description, cron_expression, scheduled_at, timezone, next_trigger_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&public_id)
        .bind(input.org_id)
        .bind(input.session_id)
        .bind(&input.description)
        .bind(&input.cron_expression)
        .bind(input.scheduled_at)
        .bind(&input.timezone)
        .bind(input.next_trigger_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<Option<SessionScheduleRow>> {
        let row = sqlx::query_as::<_, SessionScheduleRow>(
            "SELECT * FROM session_schedules WHERE id = $1 AND org_id = $2",
        )
        .bind(schedule_id)
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_session_schedules(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<Vec<SessionScheduleRow>> {
        let rows = sqlx::query_as::<_, SessionScheduleRow>(
            "SELECT * FROM session_schedules WHERE org_id = $1 AND session_id = $2 ORDER BY created_at DESC",
        )
        .bind(org_id)
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
        input: UpdateSessionScheduleRow,
    ) -> Result<Option<SessionScheduleRow>> {
        let row = sqlx::query_as::<_, SessionScheduleRow>(
            r#"
            UPDATE session_schedules SET
                enabled = COALESCE($3, enabled),
                next_trigger_at = CASE WHEN $4 THEN $5 ELSE next_trigger_at END,
                last_triggered_at = COALESCE($6, last_triggered_at),
                trigger_count = trigger_count + CASE WHEN $7 THEN 1 ELSE 0 END,
                updated_at = NOW()
            WHERE id = $1 AND org_id = $2
            RETURNING *
            "#,
        )
        .bind(schedule_id)
        .bind(org_id)
        .bind(input.enabled)
        .bind(input.next_trigger_at.is_some()) // flag: should update next_trigger_at
        .bind(input.next_trigger_at.unwrap_or(None)) // value (may be NULL for one-shot)
        .bind(input.last_triggered_at)
        .bind(input.trigger_count_increment)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<bool> {
        let result = sqlx::query("DELETE FROM session_schedules WHERE id = $1 AND org_id = $2")
            .bind(schedule_id)
            .bind(org_id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn count_active_session_schedules(&self, session_id: SessionId) -> Result<u32> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM session_schedules WHERE session_id = $1 AND enabled = true",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0 as u32)
    }

    /// Claim due session schedules for processing.
    /// Uses SELECT FOR UPDATE SKIP LOCKED for multi-instance safety.
    pub async fn claim_due_session_schedules(&self, limit: i32) -> Result<Vec<SessionScheduleRow>> {
        let rows = sqlx::query_as::<_, SessionScheduleRow>(
            r#"
            SELECT * FROM session_schedules
            WHERE enabled = true
              AND next_trigger_at IS NOT NULL
              AND next_trigger_at <= NOW()
            ORDER BY next_trigger_at ASC
            LIMIT $1
            FOR UPDATE SKIP LOCKED
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // Audit Logs (TM-OBS-007)

    pub async fn create_audit_log(&self, input: CreateAuditLogRow) -> Result<AuditLogRow> {
        let row = sqlx::query_as::<_, AuditLogRow>(
            r#"
            INSERT INTO audit_logs (org_id, actor_id, event_type, ip_address, metadata)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING *
            "#,
        )
        .bind(input.org_id)
        .bind(input.actor_id)
        .bind(&input.event_type)
        .bind(&input.ip_address)
        .bind(&input.metadata)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_audit_logs(
        &self,
        org_id: i64,
        limit: i64,
        before: Option<DateTime<Utc>>,
        event_type_prefix: Option<&str>,
        actor_id: Option<Uuid>,
    ) -> Result<Vec<AuditLogRow>> {
        // Build query dynamically based on filters
        let before_ts = before.unwrap_or_else(|| Utc::now() + chrono::Duration::seconds(1));
        let rows = if let Some(prefix) = event_type_prefix {
            if let Some(aid) = actor_id {
                sqlx::query_as::<_, AuditLogRow>(
                    r#"
                    SELECT * FROM audit_logs
                    WHERE org_id = $1 AND created_at < $2 AND event_type LIKE $3 AND actor_id = $4
                    ORDER BY created_at DESC
                    LIMIT $5
                    "#,
                )
                .bind(org_id)
                .bind(before_ts)
                .bind(format!("{}%", prefix))
                .bind(aid)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            } else {
                sqlx::query_as::<_, AuditLogRow>(
                    r#"
                    SELECT * FROM audit_logs
                    WHERE org_id = $1 AND created_at < $2 AND event_type LIKE $3
                    ORDER BY created_at DESC
                    LIMIT $4
                    "#,
                )
                .bind(org_id)
                .bind(before_ts)
                .bind(format!("{}%", prefix))
                .bind(limit)
                .fetch_all(&self.pool)
                .await?
            }
        } else if let Some(aid) = actor_id {
            sqlx::query_as::<_, AuditLogRow>(
                r#"
                SELECT * FROM audit_logs
                WHERE org_id = $1 AND created_at < $2 AND actor_id = $3
                ORDER BY created_at DESC
                LIMIT $4
                "#,
            )
            .bind(org_id)
            .bind(before_ts)
            .bind(aid)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, AuditLogRow>(
                r#"
                SELECT * FROM audit_logs
                WHERE org_id = $1 AND created_at < $2
                ORDER BY created_at DESC
                LIMIT $3
                "#,
            )
            .bind(org_id)
            .bind(before_ts)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }

    pub async fn delete_audit_logs_before(&self, before: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query("DELETE FROM audit_logs WHERE created_at < $1")
            .bind(before)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    // ============================================
    // App CRUD
    // ============================================

    pub async fn create_app(&self, org_id: i64, input: CreateAppRow) -> Result<AppRow> {
        let row = sqlx::query_as::<_, AppRow>(
            r#"
            INSERT INTO apps (org_id, public_id, name, description, harness_id, agent_id, channel_type, channel_config, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'draft')
            RETURNING id, org_id, public_id, name, description, harness_id, agent_id, channel_type, channel_config, status, published_at, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(input.harness_id)
        .bind(input.agent_id)
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_app_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AppRow>> {
        let row = sqlx::query_as::<_, AppRow>(
            r#"
            SELECT id, org_id, public_id, name, description, harness_id, agent_id, channel_type, channel_config, status, published_at, created_at, updated_at
            FROM apps
            WHERE org_id = $1 AND public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Lookup app by public_id without org scoping (for unauthenticated webhooks).
    pub async fn get_app_by_public_id_unscoped(&self, public_id: &str) -> Result<Option<AppRow>> {
        let row = sqlx::query_as::<_, AppRow>(
            r#"
            SELECT id, org_id, public_id, name, description, harness_id, agent_id, channel_type, channel_config, status, published_at, created_at, updated_at
            FROM apps
            WHERE public_id = $1
            "#,
        )
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_apps(&self, org_id: i64, search: Option<&str>) -> Result<Vec<AppRow>> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 2);
        let sql = format!(
            r#"SELECT id, org_id, public_id, name, description, harness_id, agent_id, channel_type, channel_config, status, published_at, created_at, updated_at
                FROM apps
                WHERE org_id = $1 AND status != 'archived'{search_sql}
                ORDER BY created_at DESC"#
        );
        let mut query = sqlx::query_as::<_, AppRow>(&sql).bind(org_id);
        for pat in &patterns {
            query = query.bind(pat);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn update_app(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateApp,
    ) -> Result<Option<AppRow>> {
        let row = sqlx::query_as::<_, AppRow>(
            r#"
            UPDATE apps
            SET
                name = COALESCE($3, name),
                description = COALESCE($4, description),
                harness_id = COALESCE($5, harness_id),
                agent_id = COALESCE($6, agent_id),
                channel_type = COALESCE($7, channel_type),
                channel_config = COALESCE($8, channel_config),
                status = COALESCE($9, status),
                published_at = CASE WHEN $10 THEN $11 ELSE published_at END,
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, public_id, name, description, harness_id, agent_id, channel_type, channel_config, status, published_at, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(input.harness_id)
        .bind(input.agent_id)
        .bind(&input.channel_type)
        .bind(&input.channel_config)
        .bind(&input.status)
        .bind(input.published_at.is_some())
        .bind(input.published_at.flatten())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_app(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE apps
            SET status = 'archived', updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status != 'archived'
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_pool_config_defaults() {
        let config = DatabasePoolConfig::default();
        assert_eq!(config.max_connections, 50);
        assert_eq!(config.min_connections, 5);
        assert_eq!(config.acquire_timeout, std::time::Duration::from_secs(5));
        assert_eq!(config.idle_timeout, std::time::Duration::from_secs(300));
    }

    #[test]
    fn database_pool_config_from_env() {
        // SAFETY: test is run single-threaded (--test-threads=1)
        unsafe {
            std::env::set_var("DATABASE_POOL_MAX", "100");
            std::env::set_var("DATABASE_POOL_MIN", "10");
            std::env::set_var("DATABASE_ACQUIRE_TIMEOUT_SECS", "15");
            std::env::set_var("DATABASE_IDLE_TIMEOUT_SECS", "600");
        }

        let config = DatabasePoolConfig::from_env();
        assert_eq!(config.max_connections, 100);
        assert_eq!(config.min_connections, 10);
        assert_eq!(config.acquire_timeout, std::time::Duration::from_secs(15));
        assert_eq!(config.idle_timeout, std::time::Duration::from_secs(600));

        unsafe {
            std::env::remove_var("DATABASE_POOL_MAX");
            std::env::remove_var("DATABASE_POOL_MIN");
            std::env::remove_var("DATABASE_ACQUIRE_TIMEOUT_SECS");
            std::env::remove_var("DATABASE_IDLE_TIMEOUT_SECS");
        }
    }

    #[test]
    fn database_pool_config_invalid_values_use_defaults() {
        // SAFETY: test is run single-threaded (--test-threads=1)
        unsafe {
            std::env::set_var("DATABASE_POOL_MAX", "not_a_number");
        }

        let config = DatabasePoolConfig::from_env();
        assert_eq!(config.max_connections, 50); // falls back to default

        unsafe {
            std::env::remove_var("DATABASE_POOL_MAX");
        }
    }

    // ─── build_search_sql / escape_like tests ───

    #[test]
    fn escape_like_leaves_normal_text_unchanged() {
        assert_eq!(escape_like("hello world"), "hello world");
    }

    #[test]
    fn escape_like_escapes_percent() {
        assert_eq!(escape_like("100%"), "100\\%");
    }

    #[test]
    fn escape_like_escapes_underscore() {
        assert_eq!(escape_like("a_b"), "a\\_b");
    }

    #[test]
    fn escape_like_escapes_backslash() {
        assert_eq!(escape_like("c:\\path"), "c:\\\\path");
    }

    #[test]
    fn escape_like_handles_all_special_chars_together() {
        assert_eq!(escape_like("%_\\"), "\\%\\_\\\\");
    }

    #[test]
    fn build_search_sql_none_returns_empty() {
        let (sql, patterns) = build_search_sql(None, "LOWER(name)", 2);
        assert!(sql.is_empty());
        assert!(patterns.is_empty());
    }

    #[test]
    fn build_search_sql_empty_string_returns_empty() {
        let (sql, patterns) = build_search_sql(Some(""), "LOWER(name)", 2);
        assert!(sql.is_empty());
        assert!(patterns.is_empty());
    }

    #[test]
    fn build_search_sql_whitespace_only_returns_empty() {
        let (sql, patterns) = build_search_sql(Some("   "), "LOWER(name)", 2);
        assert!(sql.is_empty());
        assert!(patterns.is_empty());
    }

    #[test]
    fn build_search_sql_single_word() {
        let (sql, patterns) = build_search_sql(Some("hello"), "LOWER(name)", 2);
        assert_eq!(patterns, vec!["%hello%"]);
        assert!(sql.contains("LIKE $2 ESCAPE"));
        assert!(!sql.contains("$3"));
    }

    #[test]
    fn build_search_sql_multi_word() {
        let (sql, patterns) = build_search_sql(Some("hello world"), "LOWER(name)", 2);
        assert_eq!(patterns, vec!["%hello%", "%world%"]);
        assert!(sql.contains("LIKE $2 ESCAPE"));
        assert!(sql.contains("LIKE $3 ESCAPE"));
    }

    #[test]
    fn build_search_sql_escapes_like_wildcards() {
        let (_, patterns) = build_search_sql(Some("100% done"), "LOWER(name)", 2);
        assert_eq!(patterns, vec!["%100\\%%", "%done%"]);
    }

    #[test]
    fn build_search_sql_escapes_underscore() {
        let (_, patterns) = build_search_sql(Some("my_var"), "LOWER(name)", 2);
        assert_eq!(patterns, vec!["%my\\_var%"]);
    }

    #[test]
    fn build_search_sql_caps_tokens_at_max() {
        // A poem or long query should be capped
        let poem = "roses are red violets are blue sugar is sweet and so are you extra words here";
        let (_, patterns) = build_search_sql(Some(poem), "LOWER(name)", 2);
        assert_eq!(patterns.len(), MAX_SEARCH_TOKENS);
    }

    #[test]
    fn build_search_sql_unicode_tokens() {
        let (_, patterns) = build_search_sql(Some("日本語 テスト"), "LOWER(name)", 2);
        assert_eq!(patterns, vec!["%日本語%", "%テスト%"]);
    }

    #[test]
    fn build_search_sql_emoji_input() {
        let (_, patterns) = build_search_sql(Some("🤖 bot"), "LOWER(name)", 2);
        assert_eq!(patterns, vec!["%🤖%", "%bot%"]);
    }

    #[test]
    fn build_search_sql_sql_injection_attempt() {
        // SQL injection via search should be harmless — values are parameterized
        let (sql, patterns) = build_search_sql(Some("'; DROP TABLE agents; --"), "LOWER(name)", 2);
        // The tokens are just words, bound as parameters
        assert_eq!(patterns.len(), 5); // "';", "drop", "table", "agents;", "--"
        // SQL structure is safe — only LIKE clauses
        assert!(sql.contains("LIKE $2"));
        assert!(!sql.contains("DROP"));
    }

    #[test]
    fn build_search_sql_param_offset() {
        // When starting at param 4 (e.g. after org_id, agent_id, other filters)
        let (sql, _) = build_search_sql(Some("test query"), "LOWER(name)", 4);
        assert!(sql.contains("$4"));
        assert!(sql.contains("$5"));
    }

    #[test]
    fn build_search_sql_case_insensitive() {
        let (_, patterns) = build_search_sql(Some("HeLLo WoRLd"), "LOWER(name)", 2);
        assert_eq!(patterns, vec!["%hello%", "%world%"]);
    }

    #[test]
    fn build_search_sql_extra_whitespace_collapsed() {
        let (_, patterns) = build_search_sql(Some("  hello    world  "), "LOWER(name)", 2);
        assert_eq!(patterns, vec!["%hello%", "%world%"]);
    }
}

// Repository layer for database operations
// Decision: PostgreSQL-backed, split into per-entity modules (EVE-100).

mod agents;
mod apps;
mod audit_logs;
mod auth;
mod events;
mod harnesses;
mod llm;
mod mcp_servers;
mod notifications;
mod organizations;
mod schedules;
mod session_files;
mod session_git;
mod session_storage;
mod sessions;
mod skills;
mod user_connections;
mod users;

#[cfg(test)]
mod tests;

use anyhow::Result;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

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
}

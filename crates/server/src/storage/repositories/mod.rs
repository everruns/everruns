// Repository layer for database operations
// Decision: PostgreSQL-backed, split into per-entity modules (EVE-100). An
// entity's rows sit beside its repository in `<entity>/rows.rs` and are
// re-exported here, so callers name them as `crate::storage::*`.

mod agent_activity;
mod agent_channels;
pub use agent_channels::rows::*;
mod agent_check_rules;
pub use agent_check_rules::rows::*;
mod agent_health_checks;
pub use agent_health_checks::rows::*;
mod agent_keys;
pub use agent_keys::rows::*;
mod agent_mcp_secret_bindings;
pub use agent_mcp_secret_bindings::rows::*;
mod agent_scripts;
pub use agent_scripts::rows::*;
mod agent_triggers;
pub use agent_triggers::rows::*;
mod agents;
pub use agents::rows::*;
mod apps;
pub use apps::rows::*;
mod audit_logs;
pub use audit_logs::rows::*;
mod auth;
pub use auth::rows::*;
mod budgets;
pub use budgets::rows::*;
mod compaction_checkpoints;
pub use compaction_checkpoints::rows::*;
mod virtual_user_connections;
pub use virtual_user_connections::rows::*;
mod virtual_users;
pub use advisory_locks::ADVISORY_LOCK_WAIT;
pub use agent_activity::{
    AGENT_ACTIVITY_HOURS, AgentActivityRows, AgentLoadRow, AgentRunBucketRow,
    AgentTriggerSummaryRow, CHANNEL_ACTIVITY_DAYS, ChannelAudienceRow, ChannelSessionBucketRow,
};
pub use budgets::BudgetSubjectLookup;
pub use virtual_user_connections::OrganizationConnectionInUse;
pub use virtual_users::rows::*;
mod a2a_push_configs;
mod advisory_locks;
mod agent_avatars;
mod agent_trigger_mcp_subscriptions;
mod command_idempotency;
mod declarative_capabilities;
pub use declarative_capabilities::rows::*;
mod entity_changes;
mod evals;
pub use evals::rows::*;
mod events;
pub use events::rows::*;
mod files;
pub use files::rows::*;
mod github_apps;
pub use github_apps::rows::*;
mod harnesses;
pub use harnesses::rows::*;
mod health_issues;
mod knowledge_bases;
pub use knowledge_bases::rows::*;
mod knowledge_indexes;
pub use knowledge_indexes::rows::*;
mod late_generation_usage;
mod manager_context;
mod mcp_event_subscriptions;
mod mcp_servers;
pub use mcp_servers::rows::*;
mod mcp_tool_labels;
pub use mcp_tool_labels::rows::*;
mod memory;
pub use memory::rows::*;
mod notifications;
pub use notifications::rows::*;
mod oauth_grants;
pub use oauth_grants::OAUTH_GRANT_ACCESS_READ_AND_RUN;
pub use oauth_grants::rows::*;
mod observers;
pub use observers::rows::*;
mod org_feature_flags;
mod org_slack_connections;
mod organizations;
pub use organizations::rows::*;
mod payments;
pub use payments::rows::*;
mod plugins;
pub use plugins::rows::*;
mod principals;
pub use principals::rows::*;
mod providers;
pub use providers::rows::*;
mod reporting;
mod schedules;
pub use schedules::rows::*;
mod session_delete;
mod session_files;
pub use session_files::rows::*;
mod session_git;
pub use session_git::rows::*;
mod session_participants;
pub use session_participants::rows::*;
mod session_resources;
pub use session_resources::rows::*;
mod session_storage;
pub use session_storage::rows::*;
mod session_tasks;
pub use session_tasks::rows::*;
mod sessions;
pub use sessions::rows::*;
mod session_trace;
pub use session_trace::reads::*;
pub use session_trace::rows::*;
pub use session_trace::{TRACE_PASS_BUDGET, projection as session_trace_projection};
mod skills;
pub use skills::rows::*;
mod user_connections;
pub use user_connections::rows::*;
mod user_mcp_servers;
mod user_preferences;
pub use user_mcp_servers::{OwnedMcpServerRow, UserMcpServerRow};
pub use user_preferences::rows::*;
mod users;
pub use users::rows::*;
mod virtual_user_preferences;
mod waiting_turn_resolutions;
mod workspaces;
pub use workspaces::rows::*;

#[cfg(test)]
mod tests;

use anyhow::Result;
use everruns_core::config::{env_duration_secs, env_or};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// Database pool configuration loaded from environment variables.
///
/// Two pools, not one (EVE-1081). The request pool is sized for HTTP handlers
/// and uses a shorter bounded acquire timeout, because a caller is waiting on
/// the other end of every acquire. The background pool is small and patient,
/// because nobody is waiting on a sweep and the next tick would only retry into
/// the same contention. Sharing one pool made saturation from any source fail
/// every consumer at the same instant; see `Database::connect_with_config`.
pub struct DatabasePoolConfig {
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout: std::time::Duration,
    pub idle_timeout: std::time::Duration,
    /// Connections reserved for background sweeps. Sized against the number of
    /// background loops the server spawns (currently ~10), each of which runs
    /// its queries sequentially; the one exception is the reporting repair
    /// path, which holds an advisory-lock connection while acquiring a second.
    /// Brief queuing among sweeps is fine — that contention is bounded and
    /// self-inflicted, which is exactly what sharing with the request path was
    /// not. `0` puts background work back on the request pool — the
    /// pre-EVE-1081 behavior, kept as a config-only rollback.
    pub background_max_connections: u32,
    /// Acquire timeout for the background pool. Deliberately far longer than
    /// the request timeout: a sweep can afford to wait.
    pub background_acquire_timeout: std::time::Duration,
}

impl Default for DatabasePoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 50,
            min_connections: 5,
            acquire_timeout: std::time::Duration::from_secs(5),
            idle_timeout: std::time::Duration::from_secs(300),
            background_max_connections: 8,
            background_acquire_timeout: std::time::Duration::from_secs(30),
        }
    }
}

impl DatabasePoolConfig {
    /// Load pool configuration from environment variables with sensible defaults.
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            max_connections: env_or("DATABASE_POOL_MAX", defaults.max_connections),
            min_connections: env_or("DATABASE_POOL_MIN", defaults.min_connections),
            acquire_timeout: env_duration_secs(
                "DATABASE_ACQUIRE_TIMEOUT_SECS",
                defaults.acquire_timeout,
            ),
            idle_timeout: env_duration_secs("DATABASE_IDLE_TIMEOUT_SECS", defaults.idle_timeout),
            background_max_connections: env_or(
                "DATABASE_BACKGROUND_POOL_MAX",
                defaults.background_max_connections,
            ),
            background_acquire_timeout: env_duration_secs(
                "DATABASE_BACKGROUND_ACQUIRE_TIMEOUT_SECS",
                defaults.background_acquire_timeout,
            ),
        }
    }

    /// Connections this process opens to PostgreSQL across both pools.
    pub fn total_max_connections(&self) -> u32 {
        if self.background_max_connections == 0 {
            self.max_connections
        } else {
            self.max_connections
                .saturating_add(self.background_max_connections)
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
    /// The request pool. Queries on it join the current task's command
    /// transaction when one is open (`crate::storage::transaction`).
    pool: crate::storage::transaction::TxPool,
    /// Connections reserved for background sweeps (EVE-1081). Separate from
    /// `pool` so a request burst cannot starve them; equal to `pool` when the
    /// database was built without a background pool (tests, embedded uses).
    background_pool: PgPool,
    /// Optional S3-compatible blob backend. When set, file/image content bytes
    /// are offloaded to the object store and these tables hold only metadata +
    /// a pointer. `None` keeps bytes inline in
    /// PostgreSQL (default behavior).
    blob_store: Option<crate::storage::blob_store::SharedBlobStore>,
    /// Keeps a test's private database alive while any clone uses it.
    test_database: Option<std::sync::Arc<crate::storage::test_database::TestDatabase>>,
}

/// Idle time after which a pooled connection is pinged before reuse.
const PING_AFTER_IDLE: std::time::Duration = std::time::Duration::from_secs(30);

/// Pool options shared by the request and background pools.
///
/// sqlx pings every idle connection on checkout by default, which doubles the
/// round trips of every query outside a transaction. With the database a few
/// milliseconds away that ping was close to half of the send-message path. A
/// connection that was in use moments ago is almost certainly alive, so only
/// one idle past `PING_AFTER_IDLE` is checked; a failed ping drops it and the
/// pool hands out another.
fn pool_options() -> PgPoolOptions {
    PgPoolOptions::new()
        .test_before_acquire(false)
        .before_acquire(|conn, meta| {
            Box::pin(async move {
                if meta.idle_for >= PING_AFTER_IDLE {
                    sqlx::Connection::ping(conn).await?;
                }
                Ok(true)
            })
        })
}

impl Database {
    pub fn new(pool: PgPool) -> Self {
        Self {
            background_pool: pool.clone(),
            pool: crate::storage::transaction::TxPool::new(pool),
            blob_store: None,
            test_database: None,
        }
    }

    /// Tie a test's private database to this value and its clones.
    pub fn with_test_database(
        mut self,
        database: std::sync::Arc<crate::storage::test_database::TestDatabase>,
    ) -> Self {
        self.test_database = Some(database);
        self
    }

    /// The test database this value runs on, if any.
    pub(crate) fn test_database(
        &self,
    ) -> Option<&std::sync::Arc<crate::storage::test_database::TestDatabase>> {
        self.test_database.as_ref()
    }

    /// Attach an object-storage blob backend for content offload.
    pub fn with_blob_store(
        mut self,
        blob_store: Option<crate::storage::blob_store::SharedBlobStore>,
    ) -> Self {
        self.blob_store = blob_store;
        self
    }

    /// The configured blob backend, if content offload is enabled.
    pub fn blob_store(&self) -> Option<&crate::storage::blob_store::SharedBlobStore> {
        self.blob_store.as_ref()
    }

    /// Create database connection pools from URL, configured from the environment.
    ///
    /// Multi-instance sizing: set `DATABASE_POOL_MAX = pg_max_connections / N - margin`
    /// where N = number of control-plane instances. The background pool adds
    /// `DATABASE_BACKGROUND_POOL_MAX` on top of that per instance.
    pub async fn from_url(database_url: &str) -> Result<Self> {
        Self::connect_with_config(database_url, DatabasePoolConfig::from_env()).await
    }

    /// Create database connection pools from an explicit configuration.
    ///
    /// Builds two pools over the same database (EVE-1081). Production showed
    /// four independent background loops — the durable scheduler, the observer
    /// scoring worker and the sweeps around them — reporting
    /// `pool timed out while waiting for an open connection` within two seconds
    /// of each other, sharing a trace with user-facing 500s. They were all
    /// waiting on the one process-wide pool, so whatever saturated it failed
    /// every consumer at the same instant. A separate small pool means a
    /// request burst can no longer take the sweeps down with it, and the
    /// sweeps can no longer consume the connections requests need.
    pub async fn connect_with_config(
        database_url: &str,
        config: DatabasePoolConfig,
    ) -> Result<Self> {
        let pool = pool_options()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(config.acquire_timeout)
            .idle_timeout(config.idle_timeout)
            .connect(database_url)
            .await?;

        // Lazy: the background pool must not add a second round of connects to
        // startup, and its first sweep tick is seconds away regardless.
        let background_pool = if config.background_max_connections == 0 {
            pool.clone()
        } else {
            pool_options()
                .max_connections(config.background_max_connections)
                .min_connections(1)
                .acquire_timeout(config.background_acquire_timeout)
                .idle_timeout(config.idle_timeout)
                .connect_lazy(database_url)?
        };

        tracing::info!(
            max_connections = config.max_connections,
            min_connections = config.min_connections,
            acquire_timeout_secs = config.acquire_timeout.as_secs(),
            idle_timeout_secs = config.idle_timeout.as_secs(),
            background_max_connections = config.background_max_connections,
            background_acquire_timeout_secs = config.background_acquire_timeout.as_secs(),
            "Database connection pool initialized"
        );

        // Multi-instance pool sizing check
        let instances: u32 = std::env::var("EXPECTED_INSTANCES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1)
            .max(1);
        if instances > 1 {
            // Both pools count against the server's connection budget.
            let per_instance = config.total_max_connections();
            let estimated_total = per_instance.saturating_mul(instances);
            // PostgreSQL default max_connections is 100; warn if we'd exceed 80% of it
            let pg_max: u32 = std::env::var("PG_MAX_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100);
            if estimated_total > pg_max * 80 / 100 {
                tracing::warn!(
                    pool_max = config.max_connections,
                    background_pool_max = config.background_max_connections,
                    per_instance,
                    instances,
                    estimated_total,
                    pg_max,
                    "Pool size × instances ({estimated_total}) exceeds 80% of PG_MAX_CONNECTIONS ({pg_max}). \
                     Reduce DATABASE_POOL_MAX / DATABASE_BACKGROUND_POOL_MAX or increase PostgreSQL max_connections."
                );
            } else {
                tracing::info!(
                    pool_max = config.max_connections,
                    background_pool_max = config.background_max_connections,
                    per_instance,
                    instances,
                    estimated_total,
                    pg_max,
                    "Database pool sizing OK for multi-instance deployment"
                );
            }
        }

        Ok(Self {
            pool: crate::storage::transaction::TxPool::new(pool),
            background_pool,
            blob_store: None,
            test_database: None,
        })
    }

    /// The request pool: short acquire timeout, sized for HTTP handlers.
    ///
    /// Queries run on it directly bypass the current command transaction;
    /// prefer the repository methods, which run on `tx_pool`.
    pub fn pool(&self) -> &PgPool {
        self.pool.raw()
    }

    /// The request pool as repository methods use it: inside a command
    /// transaction, queries run on that transaction's connection.
    pub fn tx_pool(&self) -> &crate::storage::transaction::TxPool {
        &self.pool
    }

    /// The background pool: small and patient, for sweeps and workers that no
    /// caller is waiting on.
    pub fn background_pool(&self) -> &PgPool {
        &self.background_pool
    }

    /// A view of this database whose queries run on the background pool.
    ///
    /// Every repository method takes `&self.pool`, so handing a background loop
    /// this value routes its whole call graph off the request pool without
    /// touching a single query.
    pub fn for_background(&self) -> Self {
        Self {
            pool: crate::storage::transaction::TxPool::new(self.background_pool.clone()),
            background_pool: self.background_pool.clone(),
            blob_store: self.blob_store.clone(),
            test_database: self.test_database.clone(),
        }
    }
}

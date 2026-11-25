// Database pool sizes derived from the database itself.
//
// Decision: pools a deployment does not size explicitly are derived at startup
// from the database's own `max_connections`, so resizing PostgreSQL (and its
// connection limit with it) never needs a matching application setting. The
// per-process share is the limit, less PostgreSQL's reserved slots and a
// reserve for other clients, divided across the server processes that can run
// at once (`EXPECTED_INSTANCES`, counting rollout overlap) and less the
// dedicated session connections each process opens outside its pools.
//
// Decision: the request pool never goes below three. A request can hold a lock
// transaction and still read through the pool; two connections let one such
// request starve every other request on the process (the Slack install 500).
// A database too small for that floor gets a warning, not a smaller pool.

use super::DatabasePoolConfig;

/// Connections each server process opens outside its pools: the PostgreSQL
/// listeners (task, notification and cache-invalidation wakeups, the outbox),
/// the event feed's partition locks and a typical mutation guard.
pub const DEDICATED_PER_PROCESS: u32 = 6;

const REQUEST_FLOOR: u32 = 3;
const REQUEST_CEILING: u32 = 50;
const BACKGROUND_CEILING: u32 = 8;

/// What the database reports about its connection limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseLimits {
    pub max_connections: u32,
    /// `superuser_reserved_connections` plus `reserved_connections` (PG 16+).
    pub reserved: u32,
}

/// Which pool sizes the environment fixed; derivation leaves those alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExplicitSizes {
    pub request: bool,
    pub min: bool,
    pub background: bool,
    pub liveness: bool,
}

impl ExplicitSizes {
    /// The pool sizes set in the environment.
    pub fn from_env() -> Self {
        let set = |name: &str| std::env::var(name).is_ok_and(|value| !value.trim().is_empty());
        Self {
            request: set("DATABASE_POOL_MAX"),
            min: set("DATABASE_POOL_MIN"),
            background: set("DATABASE_BACKGROUND_POOL_MAX"),
            liveness: set("DATABASE_LIVENESS_POOL_MAX"),
        }
    }

    /// Every pool is sized by the environment: nothing to derive.
    pub fn all(&self) -> bool {
        self.request && self.background && self.liveness
    }
}

/// Read the connection limit from the database at `url` on a connection of
/// its own, closed before the pools open.
pub async fn read_limits(url: &str) -> anyhow::Result<DatabaseLimits> {
    use sqlx::Connection;
    let mut connection = sqlx::PgConnection::connect(url).await?;
    let (max_connections, reserved): (i32, i32) = sqlx::query_as(
        "SELECT current_setting('max_connections')::int4, \
                current_setting('superuser_reserved_connections')::int4 \
                + COALESCE(current_setting('reserved_connections', true), '0')::int4",
    )
    .fetch_one(&mut connection)
    .await?;
    connection.close().await?;
    Ok(DatabaseLimits {
        max_connections: max_connections.max(0) as u32,
        reserved: reserved.max(0) as u32,
    })
}

/// The per-process connection share for `instances` processes, after
/// `other_clients` connections held back for everything that is not a server
/// process (migrations, workers' tools, operators, a pooler).
pub fn process_share(limits: DatabaseLimits, instances: u32, other_clients: u32) -> u32 {
    let usable = limits
        .max_connections
        .saturating_sub(limits.reserved)
        .saturating_sub(other_clients);
    (usable / instances.max(1)).saturating_sub(DEDICATED_PER_PROCESS)
}

/// Default reserve for clients that are not server processes: a tenth of the
/// limit, at least five.
pub fn default_other_clients(limits: DatabaseLimits) -> u32 {
    (limits.max_connections / 10).max(5)
}

/// Fill in every size the environment left unset from `share`. Returns whether
/// the share was big enough for the floors.
pub fn fit(config: &mut DatabasePoolConfig, explicit: ExplicitSizes, share: u32) -> bool {
    if !explicit.liveness {
        config.liveness_max_connections = 1;
    }
    let liveness = config.liveness_max_connections;
    if !explicit.background {
        config.background_max_connections = (share / 4).clamp(1, BACKGROUND_CEILING);
    }
    let background = config.background_max_connections;
    if !explicit.request {
        config.max_connections = share
            .saturating_sub(background)
            .saturating_sub(liveness)
            .clamp(REQUEST_FLOOR, REQUEST_CEILING);
    }
    if !explicit.min {
        config.min_connections = config.min_connections.min(config.max_connections);
    }
    share >= REQUEST_FLOOR + background + liveness
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(max_connections: u32) -> DatabaseLimits {
        DatabaseLimits {
            max_connections,
            reserved: 3,
        }
    }

    fn fitted(max_connections: u32, instances: u32) -> (DatabasePoolConfig, bool) {
        let limits = limits(max_connections);
        let share = process_share(limits, instances, default_other_clients(limits));
        let mut config = DatabasePoolConfig::default();
        let fits = fit(&mut config, ExplicitSizes::default(), share);
        (config, fits)
    }

    #[test]
    fn a_default_postgres_serves_one_process_generously() {
        // 100 - 3 reserved - 10 other clients = 87, less 6 dedicated = 81.
        let (config, fits) = fitted(100, 1);
        assert!(fits);
        assert_eq!(config.liveness_max_connections, 1);
        assert_eq!(config.background_max_connections, 8);
        assert_eq!(config.max_connections, 50);
    }

    #[test]
    fn a_small_database_shared_by_four_processes_keeps_the_request_floor() {
        // 60 - 3 - 6 = 51 / 4 = 12, less 6 dedicated = 6.
        let (config, fits) = fitted(60, 4);
        assert!(fits);
        assert_eq!(config.background_max_connections, 1);
        assert_eq!(config.max_connections, 4);
        assert!(config.min_connections <= config.max_connections);
    }

    #[test]
    fn a_database_too_small_still_gets_the_floor_and_reports_it() {
        let (config, fits) = fitted(30, 4);
        assert!(!fits);
        assert_eq!(config.max_connections, REQUEST_FLOOR);
        assert_eq!(config.background_max_connections, 1);
    }

    #[test]
    fn explicit_sizes_are_kept() {
        let mut config = DatabasePoolConfig {
            max_connections: 2,
            background_max_connections: 4,
            ..Default::default()
        };
        let explicit = ExplicitSizes {
            request: true,
            min: false,
            background: true,
            liveness: false,
        };
        fit(&mut config, explicit, 80);
        assert_eq!(config.max_connections, 2);
        assert_eq!(config.background_max_connections, 4);
        assert_eq!(config.min_connections, 2, "min never exceeds max");
    }

    #[test]
    fn resizing_the_database_resizes_the_pools() {
        let (small, _) = fitted(100, 4);
        let (large, _) = fitted(400, 4);
        assert!(large.max_connections > small.max_connections);
    }
}

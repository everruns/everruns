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
//
// Decision: `DATABASE_POOLER_ADDR` routes the request and liveness pools
// through a transaction pooler (PgBouncer) with `DATABASE_URL`'s credentials
// and database, while everything session-scoped stays on the direct URL. One
// secret serves both legs, so adding a pooler is a plain setting next to the
// deployment rather than a second credential. The pooler presents its own
// certificate, not the database's, so certificate verification is relaxed to
// `require` on that leg only; encryption is kept.

use super::DatabasePoolConfig;
use sqlx::postgres::{PgConnectOptions, PgSslMode};

/// Connections each server process opens outside its pools: the PostgreSQL
/// listeners (task, notification and cache-invalidation wakeups, the outbox),
/// the event feed's partition locks and a typical mutation guard.
pub const DEDICATED_PER_PROCESS: u32 = 6;

const REQUEST_FLOOR: u32 = 3;
const REQUEST_CEILING: u32 = 50;
const BACKGROUND_CEILING: u32 = 8;

/// `DATABASE_POOLER_ADDR` as `(host, port)`, port defaulting to 6432.
pub fn pooler_address() -> anyhow::Result<Option<(String, u16)>> {
    match std::env::var("DATABASE_POOLER_ADDR") {
        Ok(value) if !value.trim().is_empty() => parse_pooler_address(value.trim()).map(Some),
        _ => Ok(None),
    }
}

fn parse_pooler_address(value: &str) -> anyhow::Result<(String, u16)> {
    match value.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => {
            let port = port
                .parse()
                .map_err(|_| anyhow::anyhow!("DATABASE_POOLER_ADDR has an invalid port"))?;
            Ok((host.to_string(), port))
        }
        Some(_) => anyhow::bail!("DATABASE_POOLER_ADDR needs a host"),
        None => Ok((value.to_string(), 6432)),
    }
}

/// `direct` aimed at the pooler at `host:port` instead of the database.
pub fn through_pooler(direct: PgConnectOptions, host: &str, port: u16) -> PgConnectOptions {
    let ssl_mode = match direct.get_ssl_mode() {
        PgSslMode::VerifyCa | PgSslMode::VerifyFull => PgSslMode::Require,
        other => other,
    };
    direct.host(host).port(port).ssl_mode(ssl_mode)
}

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

/// The reserve for other clients: `DATABASE_RESERVED_CONNECTIONS` as a count
/// (`20`) or a share of the limit (`35%`), so a reserve that covers a pooler
/// sized from the same limit grows with it. Unset or unreadable values use
/// [`default_other_clients`].
pub fn other_clients(limits: DatabaseLimits, setting: Option<&str>) -> u32 {
    let parsed = setting
        .map(str::trim)
        .and_then(|value| match value.strip_suffix('%') {
            Some(percent) => percent
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|percent| *percent <= 100)
                .map(|percent| limits.max_connections * percent / 100),
            None => value.parse().ok(),
        });
    parsed.unwrap_or_else(|| default_other_clients(limits))
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
    fn the_reserve_is_a_count_or_a_share() {
        let limits = limits(200);
        assert_eq!(other_clients(limits, None), 20);
        assert_eq!(other_clients(limits, Some("30")), 30);
        assert_eq!(other_clients(limits, Some("35%")), 70);
        assert_eq!(
            other_clients(limits, Some("150%")),
            20,
            "out of range falls back"
        );
        assert_eq!(other_clients(limits, Some("lots")), 20);
    }

    #[test]
    fn pooler_address_parses_host_and_port() {
        assert_eq!(
            parse_pooler_address("pgbouncer:6433").unwrap(),
            ("pgbouncer".to_string(), 6433)
        );
        assert_eq!(
            parse_pooler_address("pgbouncer").unwrap(),
            ("pgbouncer".to_string(), 6432)
        );
        assert!(parse_pooler_address("pgbouncer:x").is_err());
        assert!(parse_pooler_address(":6432").is_err());
    }

    #[test]
    fn the_pooler_leg_keeps_credentials_and_encryption() {
        let direct: PgConnectOptions =
            "postgres://app:secret@db.internal:5432/everruns?sslmode=verify-full"
                .parse()
                .unwrap();
        let pooled = through_pooler(direct, "pgbouncer", 6432);
        assert_eq!(pooled.get_host(), "pgbouncer");
        assert_eq!(pooled.get_port(), 6432);
        assert_eq!(pooled.get_username(), "app");
        assert_eq!(pooled.get_database(), Some("everruns"));
        assert!(matches!(pooled.get_ssl_mode(), PgSslMode::Require));

        let plain: PgConnectOptions = "postgres://app@db/everruns?sslmode=disable"
            .parse()
            .unwrap();
        let pooled = through_pooler(plain, "pgbouncer", 6432);
        assert!(matches!(pooled.get_ssl_mode(), PgSslMode::Disable));
    }

    #[test]
    fn resizing_the_database_resizes_the_pools() {
        let (small, _) = fitted(100, 4);
        let (large, _) = fitted(400, 4);
        assert!(large.max_connections > small.max_connections);
    }
}

//! Server replicas' heartbeats, stored in `system_instances`, so a doctor run
//! on any replica sees all of them.
//!
//! Decisions:
//! - Every server process writes its row at start and every
//!   [`HEARTBEAT_INTERVAL`]. A row last seen more than [`LIVE_WINDOW`] ago is a
//!   replica that is gone. Rows are never deleted: a gone replica is history,
//!   and the doctor only lists the ones that stopped within [`RECENT_WINDOW`].
//! - One row per process, not per host: a restarted pod is a new instance, so
//!   its start time says when it began serving.
//! - The heartbeat goes to the background pool and never blocks requests. A
//!   failed write is logged and retried on the next beat.
//! - Only what replicas compare with each other is stored: version, schema
//!   level, and whether NATS and Valkey are set. No URLs, no secrets.

use crate::setup::migrations;
use crate::storage::StorageBackend;
use chrono::{DateTime, Utc};
use everruns_core::config::env_opt_string;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// `kind` of a server process.
pub const KIND_SERVER: &str = "server";

/// How often a server writes its heartbeat.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// A replica seen within this window is running.
pub(crate) const LIVE_WINDOW: chrono::TimeDelta = chrono::TimeDelta::seconds(90);

/// How far back stopped replicas are listed.
pub(crate) const RECENT_WINDOW: chrono::TimeDelta = chrono::TimeDelta::hours(1);

/// Settings replicas compare with each other.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceDetails {
    #[serde(default)]
    pub nats: bool,
    #[serde(default)]
    pub valkey: bool,
}

/// One running process, as it reports itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub id: String,
    pub kind: &'static str,
    pub hostname: Option<String>,
    pub version: String,
    pub schema_version: Option<i64>,
    pub details: InstanceDetails,
    pub started_at: DateTime<Utc>,
}

impl Instance {
    /// This server process. Built once, so the doctor and the heartbeat agree
    /// on its id.
    pub fn this_server() -> &'static Instance {
        static THIS: OnceLock<Instance> = OnceLock::new();
        THIS.get_or_init(|| Instance {
            id: uuid::Uuid::now_v7().to_string(),
            kind: KIND_SERVER,
            hostname: hostname::get()
                .ok()
                .and_then(|name| name.into_string().ok()),
            version: env!("CARGO_PKG_VERSION").to_string(),
            schema_version: migrations::migrator()
                .iter()
                .filter(|migration| migration.migration_type.is_up_migration())
                .map(|migration| migration.version)
                .max(),
            details: InstanceDetails {
                nats: env_opt_string("NATS_URL").is_some(),
                valkey: env_opt_string("VALKEY_URL").is_some(),
            },
            started_at: Utc::now(),
        })
    }
}

/// A stored heartbeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstanceRow {
    pub id: String,
    pub hostname: Option<String>,
    pub version: String,
    pub schema_version: Option<i64>,
    pub details: InstanceDetails,
    pub started_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

impl InstanceRow {
    /// How a person tells replicas apart: the host name, else a short id.
    pub fn label(&self) -> String {
        self.hostname
            .clone()
            .unwrap_or_else(|| self.id[self.id.len().saturating_sub(8)..].to_string())
    }
}

/// Write `instance`'s heartbeat, creating its row on the first beat.
pub async fn record(pool: &PgPool, instance: &Instance) -> sqlx::Result<()> {
    let details = serde_json::to_value(&instance.details).unwrap_or_default();
    sqlx::query(
        "INSERT INTO system_instances
             (id, kind, hostname, version, schema_version, details, started_at, last_seen_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
         ON CONFLICT (id) DO UPDATE SET
             hostname = EXCLUDED.hostname,
             version = EXCLUDED.version,
             schema_version = EXCLUDED.schema_version,
             details = EXCLUDED.details,
             last_seen_at = NOW()",
    )
    .bind(&instance.id)
    .bind(instance.kind)
    .bind(&instance.hostname)
    .bind(&instance.version)
    .bind(instance.schema_version)
    .bind(details)
    .bind(instance.started_at)
    .execute(pool)
    .await?;
    Ok(())
}

type Row = (
    String,
    Option<String>,
    String,
    Option<i64>,
    serde_json::Value,
    DateTime<Utc>,
    DateTime<Utc>,
);

/// Instances of `kind` seen within `window`, newest first, with the
/// database's clock to compare their times against.
pub(crate) async fn recent(
    pool: &PgPool,
    kind: &str,
    window: chrono::TimeDelta,
) -> sqlx::Result<(DateTime<Utc>, Vec<InstanceRow>)> {
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT NOW()").fetch_one(pool).await?;
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, hostname, version, schema_version, details, started_at, last_seen_at
         FROM system_instances
         WHERE kind = $1 AND last_seen_at >= $2
         ORDER BY last_seen_at DESC",
    )
    .bind(kind)
    .bind(now - window)
    .fetch_all(pool)
    .await?;
    let rows = rows
        .into_iter()
        .map(
            |(id, hostname, version, schema_version, details, started_at, last_seen_at)| {
                InstanceRow {
                    id,
                    hostname,
                    version,
                    schema_version,
                    details: serde_json::from_value(details).unwrap_or_default(),
                    started_at,
                    last_seen_at,
                }
            },
        )
        .collect();
    Ok((now, rows))
}

/// Write this server's heartbeat now and every [`HEARTBEAT_INTERVAL`].
/// `db` is the background backend, so beats never queue behind requests.
pub fn spawn_heartbeat(db: Arc<StorageBackend>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let instance = Instance::this_server();
        let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = record(db.pool(), instance).await {
                tracing::warn!(%error, "Failed to record this server's heartbeat");
            }
        }
    })
}

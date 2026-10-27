// Valkey (Redis-compatible) client for distributed rate limiting
//
// Decision: Use fred crate — full-featured async client with cluster support and Lua scripting.
// Decision: Valkey is optional. When VALKEY_URL is not set, rate limiting falls back to
//   in-memory governor (per-instance). This preserves the zero-dependency dev experience.
// Decision: Sliding window rate limiting via Lua script — atomic, O(1) per check, no races.
// Decision: Prefer Valkey over Redis (Linux Foundation fork, no SSPL licensing concerns).
// Decision: Single-shot dedup primitive (`try_record_nonce`) is exposed for
//   replay-protection paths (e.g. A2A signing — TM-A2A-010). Implemented via
//   SET key value NX EX ttl — atomic and O(1).
// Decision: Always reconnect. fred treats a missing ReconnectPolicy as "never
//   reconnect", so one Valkey restart left every request paying the 2s command
//   timeout until the process restarted (SaaS prod, 2026-10-26: valkey-0 was
//   rescheduled, API floor 2.2s for ~3h). Retry forever with capped backoff, and
//   force-close connections that stop answering (a rescheduled pod behind a
//   ClusterIP can blackhole the old socket without a RST).

use anyhow::{Context, Result};
use fred::interfaces::{KeysInterface, LuaInterface};
use fred::prelude::*;
use fred::types::config::UnresponsiveConfig;
use fred::types::{Expiration, SetOptions};
use std::sync::Arc;
use std::time::Duration;

/// Per-command deadline. Callers fail open/closed on expiry, so keep it short.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
/// A connection with a frame unanswered this long is closed and re-dialed.
const UNRESPONSIVE_TIMEOUT: Duration = Duration::from_secs(5);

/// Lua script for sliding-window rate limiting using a sorted set.
///
/// - Removes entries older than the window
/// - Counts current entries
/// - If under limit, adds a new entry and sets TTL
/// - Returns remaining count (negative = over limit)
///
/// Atomic: entire operation runs in a single EVAL on the server.
const RATE_LIMIT_SCRIPT: &str = r#"
local key = KEYS[1]
local limit = tonumber(ARGV[1])
local window = tonumber(ARGV[2])
local now = tonumber(ARGV[3])
local cutoff = now - window

redis.call('ZREMRANGEBYSCORE', key, '-inf', cutoff)
local count = redis.call('ZCARD', key)

if count < limit then
    redis.call('ZADD', key, now, now .. ':' .. math.random(1000000))
    redis.call('EXPIRE', key, math.ceil(window / 1000) + 1)
    return limit - count - 1
else
    return -1
end
"#;

/// Error returned by `ValkeyClient::check_rate_limit`.
///
/// Callers use this to choose fail-open vs fail-closed on backend errors:
/// - Auth endpoints: deny on both `Exceeded` and `BackendError` (fail-closed).
/// - General API / expensive ops: deny on `Exceeded`, allow on `BackendError` (fail-open).
#[derive(Debug)]
pub enum RateLimitCheckError {
    /// The sliding-window limit was reached.
    Exceeded,
    /// The Valkey command itself failed (network error, timeout, etc.).
    BackendError,
}

/// Valkey client wrapper. Owns the connection and exposes rate-limiting primitives.
#[derive(Clone)]
pub struct ValkeyClient {
    client: Arc<Client>,
}

impl ValkeyClient {
    /// Connect to Valkey using the given URL.
    ///
    /// URL format: `valkey://host:port` or `redis://host:port` (both work).
    /// TLS: `valkeys://host:port` or `rediss://host:port`.
    pub async fn connect(url: &str) -> Result<Self> {
        let config = Config::from_url(url).context(
            "Invalid VALKEY_URL format (expected valkey://host:port or redis://host:port)",
        )?;

        let (perf, connection, policy) = client_options();
        let client = Client::new(config, Some(perf), Some(connection), Some(policy));

        client.init().await.context("Failed to connect to Valkey")?;
        tracing::info!("Connected to Valkey");

        Ok(Self {
            client: Arc::new(client),
        })
    }

    /// Check rate limit using sliding window counter.
    ///
    /// Returns `Ok(remaining)` if the request is allowed.
    /// Returns `Err(RateLimitCheckError::Exceeded)` if the limit is hit.
    /// Returns `Err(RateLimitCheckError::BackendError)` if the Valkey command fails
    /// (logged at error level; caller decides fail-open vs fail-closed).
    pub async fn check_rate_limit(
        &self,
        key: &str,
        limit: u32,
        window_secs: u64,
    ) -> Result<u32, RateLimitCheckError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let window_ms = (window_secs * 1000) as i64;

        let result: i64 = match self
            .client
            .eval(
                RATE_LIMIT_SCRIPT,
                vec![key],
                vec![limit as i64, window_ms, now],
            )
            .await
        {
            Ok(result) => result,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    key = %key,
                    "Valkey rate limit check failed"
                );
                return Err(RateLimitCheckError::BackendError);
            }
        };

        if result >= 0 {
            Ok(result as u32)
        } else {
            Err(RateLimitCheckError::Exceeded)
        }
    }

    /// Atomically reserve a nonce. Returns `Ok(true)` if the key was newly
    /// stored (first sighting), `Ok(false)` if the key already exists
    /// (replay), or `Err(())` if the Valkey command itself failed (caller
    /// chooses fail-open vs fail-closed).
    ///
    /// Uses `SET key 1 NX EX ttl_secs` — single round-trip, O(1).
    pub async fn try_record_nonce(&self, key: &str, ttl_secs: u64) -> Result<bool, ()> {
        let result: Option<String> = match self
            .client
            .set(
                key,
                "1",
                Some(Expiration::EX(ttl_secs as i64)),
                Some(SetOptions::NX),
                false,
            )
            .await
        {
            Ok(value) => value,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    key = %key,
                    "Valkey nonce SET NX failed"
                );
                return Err(());
            }
        };

        Ok(result.is_some())
    }
}

/// Client tuning: bounded command latency, dead-connection detection, and an
/// unlimited reconnect policy (see the reconnect decision at the top).
fn client_options() -> (PerformanceConfig, ConnectionConfig, ReconnectPolicy) {
    let perf = PerformanceConfig {
        default_command_timeout: COMMAND_TIMEOUT,
        ..Default::default()
    };
    let connection = ConnectionConfig {
        connection_timeout: COMMAND_TIMEOUT,
        internal_command_timeout: COMMAND_TIMEOUT,
        unresponsive: UnresponsiveConfig {
            max_timeout: Some(UNRESPONSIVE_TIMEOUT),
            interval: Duration::from_secs(1),
        },
        ..Default::default()
    };
    // max_attempts 0 = unlimited; 100ms doubling up to 5s.
    let policy = ReconnectPolicy::new_exponential(0, 100, 5_000, 2);
    (perf, connection, policy)
}

/// Try to create a Valkey client from environment.
/// Returns `None` if `VALKEY_URL` is not set (falls back to in-memory rate limiting).
pub async fn from_env() -> Result<Option<ValkeyClient>> {
    match std::env::var("VALKEY_URL") {
        Ok(url) if !url.is_empty() => {
            let client = ValkeyClient::connect(&url).await?;
            Ok(Some(client))
        }
        _ => {
            tracing::info!(
                "VALKEY_URL not set — using in-memory rate limiting (per-instance). \
                 Set VALKEY_URL for distributed rate limiting across control-plane instances."
            );
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};

    fn server_binary() -> Option<&'static str> {
        ["valkey-server", "redis-server"].into_iter().find(|bin| {
            Command::new(bin)
                .arg("--version")
                .stdout(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
    }

    fn spawn(bin: &str, port: u16) -> Child {
        Command::new(bin)
            .args([
                "--port",
                &port.to_string(),
                "--save",
                "",
                "--appendonly",
                "no",
            ])
            .stdout(Stdio::null())
            .spawn()
            .expect("spawn valkey")
    }

    async fn wait_ok(client: &ValkeyClient, key: &str) -> bool {
        for _ in 0..50 {
            if client.check_rate_limit(key, 1000, 60).await.is_ok() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        false
    }

    #[test]
    fn client_always_reconnects() {
        let (_, connection, policy) = client_options();
        assert_eq!(policy.max_attempts(), 0, "0 = unlimited attempts");
        assert_eq!(
            connection.unresponsive.max_timeout,
            Some(UNRESPONSIVE_TIMEOUT)
        );
    }

    /// Regression (SaaS prod 2026-10-26): after a Valkey restart the client
    /// never reconnected and every command timed out until the process restarted.
    #[tokio::test]
    async fn recovers_after_server_restart() {
        let Some(bin) = server_binary() else {
            eprintln!("skipping: no valkey-server/redis-server on PATH");
            return;
        };
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut server = spawn(bin, port);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let client = ValkeyClient::connect(&format!("redis://127.0.0.1:{port}"))
            .await
            .expect("connect");
        assert!(wait_ok(&client, "rl:test:before").await);

        server.kill().unwrap();
        server.wait().unwrap();
        let mut server = spawn(bin, port);

        let recovered = wait_ok(&client, "rl:test:after").await;
        server.kill().ok();
        server.wait().ok();
        assert!(recovered, "client did not reconnect after Valkey restart");
    }
}

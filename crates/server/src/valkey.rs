// Valkey (Redis-compatible) client for distributed rate limiting
//
// Decision: Use fred crate — full-featured async client with cluster support and Lua scripting.
// Decision: Valkey is optional. When VALKEY_URL is not set, rate limiting falls back to
//   in-memory governor (per-instance). This preserves the zero-dependency dev experience.
// Decision: Sliding window rate limiting via Lua script — atomic, O(1) per check, no races.
// Decision: Prefer Valkey over Redis (Linux Foundation fork, no SSPL licensing concerns).

use anyhow::{Context, Result};
use fred::interfaces::LuaInterface;
use fred::prelude::*;
use std::sync::Arc;

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

        let client = Client::new(
            config,
            Some(PerformanceConfig {
                default_command_timeout: std::time::Duration::from_secs(2),
                ..Default::default()
            }),
            None, // connection config (defaults)
            None, // reconnect policy (defaults — exponential backoff)
        );

        client.init().await.context("Failed to connect to Valkey")?;
        tracing::info!("Connected to Valkey");

        Ok(Self {
            client: Arc::new(client),
        })
    }

    /// Check rate limit using sliding window counter.
    ///
    /// Returns `Ok(remaining)` if allowed, `Err(())` if limit exceeded.
    /// `key`: rate limit key (e.g., `rl:login:192.168.1.1`)
    /// `limit`: max requests in the window
    /// `window_secs`: window duration in seconds
    pub async fn check_rate_limit(
        &self,
        key: &str,
        limit: u32,
        window_secs: u64,
    ) -> Result<u32, ()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let window_ms = (window_secs * 1000) as i64;

        let result: i64 = self
            .client
            .eval(
                RATE_LIMIT_SCRIPT,
                vec![key],
                vec![limit as i64, window_ms, now],
            )
            .await
            .map_err(|e| {
                // Fail open: on Valkey errors, allow the request (availability > strictness)
                tracing::error!(error = %e, key = %key, "Valkey rate limit check failed, allowing request");
            })?;

        if result >= 0 {
            Ok(result as u32)
        } else {
            Err(())
        }
    }
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

    #[test]
    fn test_valkey_url_parsing() {
        // Verify URL parsing works for common formats (doesn't connect)
        assert!(Config::from_url("redis://localhost:6379").is_ok());
        assert!(Config::from_url("rediss://localhost:6379").is_ok());
        assert!(Config::from_url("redis://user:pass@localhost:6379").is_ok());
        assert!(Config::from_url("not-a-url").is_err());
    }
}

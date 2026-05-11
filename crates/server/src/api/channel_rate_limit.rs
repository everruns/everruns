// Per-app, per-IP rate limiter shared by public app channels.
//
// Decision: Public app channels (AG-UI, A2A) can configure a stricter
// per-IP cap than the global API limit via the channel config's
// `rate_limit_per_minute`. This module owns the enforcement primitive shared
// across requests.
// Decision: Two backends mirror `auth::rate_limit::ApiRateLimiter` —
//   in-memory (governor) for single-instance/dev, Valkey for distributed.
// Decision: In-memory limiters are stored per opaque `scope` (caller
//   composes it: AG-UI passes the app id; A2A passes `app_id:channel_id`
//   so multiple A2A channels on the same app keep independent buckets and
//   cannot reset each other's state by alternating limits) and remember the
//   active `limit` next to the limiter. When the configured limit changes
//   we replace the entry in place rather than inserting a new bucket,
//   which previously allowed an attacker who could cycle
//   `rate_limit_per_minute` to grow the cache without bound (TM-DOS-010,
//   TM-A2A-013).
// Decision: Channel kind is captured as a `namespace` so distinct channels
//   land in distinct Valkey keys (`rl:{namespace}:{scope}:{ip}`) and never
//   share buckets, even when both are enabled on the same app.

use std::collections::HashMap;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Arc;

use governor::{Quota, RateLimiter, clock::DefaultClock, state::keyed::DashMapStateStore};
use parking_lot::RwLock;

use crate::valkey::ValkeyClient;

type KeyedLimiter = RateLimiter<IpAddr, DashMapStateStore<IpAddr>, DefaultClock>;

#[derive(Clone)]
struct CachedLimiter {
    limit: u32,
    limiter: Arc<KeyedLimiter>,
}

type LimiterCache = RwLock<HashMap<String, CachedLimiter>>;

const WINDOW_SECS: u64 = 60;

/// Per-app channel rate limiter, keyed by `(app_id, ip)`.
#[derive(Clone)]
pub struct ChannelRateLimiter {
    namespace: &'static str,
    backend: Backend,
}

#[derive(Clone)]
enum Backend {
    InMemory {
        // Key: app_id. Each entry stores the active limit and limiter; when the
        // limit changes we replace the entry to avoid unbounded per-limit growth.
        cache: Arc<LimiterCache>,
    },
    Valkey(ValkeyClient),
}

impl ChannelRateLimiter {
    pub fn in_memory(namespace: &'static str) -> Self {
        Self {
            namespace,
            backend: Backend::InMemory {
                cache: Arc::new(RwLock::new(HashMap::new())),
            },
        }
    }

    pub fn with_valkey(namespace: &'static str, client: ValkeyClient) -> Self {
        Self {
            namespace,
            backend: Backend::Valkey(client),
        }
    }

    /// Check the per-scope, per-IP limit. `scope` is an opaque caller-chosen
    /// identifier — typically the app public id for kinds with one channel
    /// per app (AG-UI), or `app_id:channel_id` for kinds that allow multiple
    /// channels per app (A2A). `limit` is requests per minute; a `limit` of
    /// zero is treated as "no per-channel limit" and always allows.
    pub async fn check(&self, scope: &str, ip: IpAddr, limit: u32) -> Result<(), RateLimitError> {
        if limit == 0 {
            return Ok(());
        }
        match &self.backend {
            Backend::InMemory { cache } => {
                // parking_lot::RwLock does not poison; this stays panic-free
                // even if a writer panics holding the lock.
                let limiter = {
                    if let Some(existing) = cache.read().get(scope)
                        && existing.limit == limit
                    {
                        existing.limiter.clone()
                    } else {
                        let mut write = cache.write();
                        match write.get(scope) {
                            Some(existing) if existing.limit == limit => existing.limiter.clone(),
                            _ => {
                                let limiter = Arc::new(RateLimiter::keyed(Quota::per_minute(
                                    NonZeroU32::new(limit).expect("limit > 0 checked above"),
                                )));
                                write.insert(
                                    scope.to_string(),
                                    CachedLimiter {
                                        limit,
                                        limiter: limiter.clone(),
                                    },
                                );
                                limiter
                            }
                        }
                    }
                };
                match limiter.check_key(&ip) {
                    Ok(_) => Ok(()),
                    Err(_) => {
                        tracing::warn!(
                            namespace = %self.namespace,
                            scope = %scope,
                            ip = %ip,
                            limit,
                            "Per-channel rate limit exceeded (in-memory)"
                        );
                        Err(RateLimitError)
                    }
                }
            }
            Backend::Valkey(client) => {
                let key = format!("rl:{}:{scope}:{ip}", self.namespace);
                match client.check_rate_limit(&key, limit, WINDOW_SECS).await {
                    Ok(_remaining) => Ok(()),
                    Err(()) => {
                        tracing::warn!(
                            namespace = %self.namespace,
                            scope = %scope,
                            ip = %ip,
                            limit,
                            "Per-channel rate limit exceeded (valkey)"
                        );
                        Err(RateLimitError)
                    }
                }
            }
        }
    }
}

pub struct RateLimitError;

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[tokio::test]
    async fn limit_zero_always_allows() {
        let limiter = ChannelRateLimiter::in_memory("test");
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        for _ in 0..100 {
            assert!(limiter.check("app_x", ip, 0).await.is_ok());
        }
    }

    #[tokio::test]
    async fn blocks_after_burst() {
        let limiter = ChannelRateLimiter::in_memory("test");
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        for _ in 0..3 {
            assert!(limiter.check("app_x", ip, 3).await.is_ok());
        }
        assert!(limiter.check("app_x", ip, 3).await.is_err());
    }

    #[tokio::test]
    async fn separate_apps_have_separate_buckets() {
        let limiter = ChannelRateLimiter::in_memory("test");
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 3));
        for _ in 0..2 {
            let _ = limiter.check("app_a", ip, 2).await;
        }
        assert!(limiter.check("app_a", ip, 2).await.is_err());
        // A different app under the same IP should still be allowed.
        assert!(limiter.check("app_b", ip, 2).await.is_ok());
    }

    #[tokio::test]
    async fn separate_ips_have_separate_buckets() {
        let limiter = ChannelRateLimiter::in_memory("test");
        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 4));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));
        for _ in 0..2 {
            let _ = limiter.check("app_x", ip1, 2).await;
        }
        assert!(limiter.check("app_x", ip1, 2).await.is_err());
        assert!(limiter.check("app_x", ip2, 2).await.is_ok());
    }

    #[tokio::test]
    async fn changing_limit_replaces_cached_entry_for_app() {
        let limiter = ChannelRateLimiter::in_memory("test");
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7));

        assert!(limiter.check("app_x", ip, 2).await.is_ok());
        assert!(limiter.check("app_x", ip, 7).await.is_ok());

        let Backend::InMemory { cache } = &limiter.backend else {
            panic!("expected in-memory backend")
        };

        let cache = cache.read();
        let entry = cache.get("app_x").expect("cache entry should exist");
        assert_eq!(entry.limit, 7);
        assert_eq!(cache.len(), 1);
    }

    #[tokio::test]
    async fn raising_limit_uses_fresh_limiter() {
        let limiter = ChannelRateLimiter::in_memory("test");
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 6));
        for _ in 0..2 {
            let _ = limiter.check("app_x", ip, 2).await;
        }
        assert!(limiter.check("app_x", ip, 2).await.is_err());
        // Raising the limit creates a new bucket — the old throttled state
        // does not leak across different limit values.
        assert!(limiter.check("app_x", ip, 10).await.is_ok());
    }

    #[tokio::test]
    async fn separate_namespaces_have_independent_buckets() {
        // Distinct channel kinds (e.g. AG-UI vs A2A) on the same app/IP must
        // not share a token bucket, even though both are configured on the
        // same app. Distinct ChannelRateLimiter instances enforce this in
        // memory; the namespace also keeps Valkey keys disjoint.
        let agui = ChannelRateLimiter::in_memory("agui");
        let a2a = ChannelRateLimiter::in_memory("a2a");
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 9));
        for _ in 0..2 {
            let _ = agui.check("app_x", ip, 2).await;
        }
        assert!(agui.check("app_x", ip, 2).await.is_err());
        assert!(a2a.check("app_x", ip, 2).await.is_ok());
    }
}

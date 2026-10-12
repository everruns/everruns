//! Per-org egress grants (open egress, allowlist extensions) at the egress boundary: a short-TTL cache in
//! front of the host's resolver, and the process-wide slot hosts install it in.
//!
//! Decision: the resolver is late-bound through a process-wide slot rather
//! than threaded into every `DirectEgressService` constructor. Runtime egress
//! is built in several places that have no database or control-plane client
//! in reach (the default host composition, the MCP service, the plugin API),
//! while the resolver needs one. The server installs its database resolver and
//! the gRPC worker its control-plane resolver once at startup; every service
//! built with `for_runtime_traffic_from_env()` reads the slot per request, so
//! construction order does not matter. Tests and embedders that want an
//! explicit resolver use `DirectEgressService::with_org_allowlist`.
//!
//! The cache keeps each org's answer for [`ORG_EGRESS_ALLOWLIST_CACHE_TTL`], so
//! an admin's edit or a revoked grant takes effect within that window. Lookup
//! failures are not cached and count as "no extension" (fail closed).

use everruns_contracts::runtime::org_egress_allowlist::{OrgEgressAllowlist, OrgEgressGrant};
use everruns_contracts::typed_id::OrgId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// How long an org's extension is reused before it is looked up again.
pub const ORG_EGRESS_ALLOWLIST_CACHE_TTL: Duration = Duration::from_secs(60);

/// Above this many cached orgs, expired entries are swept on insert.
const SWEEP_THRESHOLD: usize = 1024;

type CachedGrant = Option<Arc<OrgEgressGrant>>;

/// TTL cache over an [`OrgEgressAllowlist`] resolver.
pub(crate) struct OrgAllowlistCache {
    resolver: Arc<dyn OrgEgressAllowlist>,
    ttl: Duration,
    entries: Mutex<HashMap<OrgId, (Instant, CachedGrant)>>,
}

impl OrgAllowlistCache {
    pub(crate) fn new(resolver: Arc<dyn OrgEgressAllowlist>, ttl: Duration) -> Self {
        Self {
            resolver,
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// The org's enforced grant, from cache when fresh; `None` when it has
    /// none (or the lookup failed).
    pub(crate) async fn grant(&self, org_id: &OrgId) -> CachedGrant {
        {
            let entries = self
                .entries
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some((fetched, grant)) = entries.get(org_id)
                && fetched.elapsed() < self.ttl
            {
                return grant.clone();
            }
        }
        match self.resolver.grant(org_id).await {
            Ok(mut grant) => {
                grant.extension = grant.extension.filter(|list| !list.allowed.is_empty());
                let grant = (!grant.is_empty()).then(|| Arc::new(grant));
                let mut entries = self
                    .entries
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if entries.len() >= SWEEP_THRESHOLD {
                    let ttl = self.ttl;
                    entries.retain(|_, (fetched, _)| fetched.elapsed() < ttl);
                }
                entries.insert(*org_id, (Instant::now(), grant.clone()));
                grant
            }
            Err(error) => {
                tracing::warn!(
                    org_id = %org_id,
                    error = %error,
                    "org egress grant lookup failed; enforcing without it"
                );
                None
            }
        }
    }
}

static RUNTIME_ORG_ALLOWLIST: RwLock<Option<Arc<OrgAllowlistCache>>> = RwLock::new(None);

/// Install the process's org allowlist resolver for runtime egress.
///
/// Every `DirectEgressService` built by `for_runtime_traffic_from_env()`
/// consults it, including ones built before this call. A later call replaces
/// the resolver and starts with an empty cache.
pub fn install_runtime_org_egress_allowlist(resolver: Arc<dyn OrgEgressAllowlist>) {
    let cache = Arc::new(OrgAllowlistCache::new(
        resolver,
        ORG_EGRESS_ALLOWLIST_CACHE_TTL,
    ));
    *RUNTIME_ORG_ALLOWLIST
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(cache);
}

pub(crate) fn runtime_org_allowlist() -> Option<Arc<OrgAllowlistCache>> {
    RUNTIME_ORG_ALLOWLIST
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Where a `DirectEgressService` finds org extensions.
#[derive(Clone, Default)]
pub(crate) enum OrgAllowlistSource {
    /// No org extensions (bare transports, tests).
    #[default]
    None,
    /// The process-wide resolver, when one is installed.
    Runtime,
    /// A resolver attached to this service.
    Explicit(Arc<OrgAllowlistCache>),
}

impl OrgAllowlistSource {
    pub(crate) fn resolve(&self) -> Option<Arc<OrgAllowlistCache>> {
        match self {
            Self::None => None,
            Self::Runtime => runtime_org_allowlist(),
            Self::Explicit(cache) => Some(cache.clone()),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use async_trait::async_trait;
    use everruns_contracts::runtime::network_access::NetworkAccessList;

    pub(crate) fn extension_grant(list: NetworkAccessList) -> OrgEgressGrant {
        OrgEgressGrant {
            open: false,
            extension: Some(list),
        }
    }
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A resolver with a fixed answer per org that counts its lookups.
    #[derive(Default)]
    pub(crate) struct FixedResolver {
        pub(crate) answers: HashMap<OrgId, Result<OrgEgressGrant, String>>,
        pub(crate) lookups: AtomicUsize,
    }

    #[async_trait]
    impl OrgEgressAllowlist for FixedResolver {
        async fn grant(&self, org_id: &OrgId) -> Result<OrgEgressGrant, String> {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            self.answers
                .get(org_id)
                .cloned()
                .unwrap_or(Ok(OrgEgressGrant::default()))
        }
    }

    fn org(n: u128) -> OrgId {
        format!("org_{n:032x}").parse().unwrap()
    }

    #[tokio::test]
    async fn caches_answers_for_the_ttl_and_refetches_after() {
        let mut resolver = FixedResolver::default();
        resolver.answers.insert(
            org(1),
            Ok(extension_grant(NetworkAccessList::allow_only([
                "api.acme.com",
            ]))),
        );
        let resolver = Arc::new(resolver);
        let cache = OrgAllowlistCache::new(resolver.clone(), Duration::from_millis(50));

        for _ in 0..3 {
            let grant = cache.grant(&org(1)).await.expect("granted");
            let extension = grant.extension.as_ref().expect("extension");
            assert!(extension.is_url_allowed("https://api.acme.com/x"));
        }
        assert!(cache.grant(&org(2)).await.is_none());
        assert!(cache.grant(&org(2)).await.is_none());
        assert_eq!(resolver.lookups.load(Ordering::SeqCst), 2, "one per org");

        tokio::time::sleep(Duration::from_millis(60)).await;
        cache.grant(&org(1)).await;
        assert_eq!(resolver.lookups.load(Ordering::SeqCst), 3, "expired");
    }

    #[tokio::test]
    async fn lookup_failures_fail_closed_and_are_not_cached() {
        let mut resolver = FixedResolver::default();
        resolver.answers.insert(org(1), Err("db down".into()));
        let resolver = Arc::new(resolver);
        let cache = OrgAllowlistCache::new(resolver.clone(), Duration::from_secs(60));

        assert!(cache.grant(&org(1)).await.is_none());
        assert!(cache.grant(&org(1)).await.is_none());
        assert_eq!(resolver.lookups.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn an_empty_extension_is_no_grant() {
        let mut resolver = FixedResolver::default();
        resolver
            .answers
            .insert(org(1), Ok(extension_grant(NetworkAccessList::default())));
        let cache = OrgAllowlistCache::new(Arc::new(resolver), Duration::from_secs(60));
        assert!(cache.grant(&org(1)).await.is_none());
    }

    #[tokio::test]
    async fn an_open_grant_without_patterns_is_kept() {
        let mut resolver = FixedResolver::default();
        resolver.answers.insert(
            org(1),
            Ok(OrgEgressGrant {
                open: true,
                extension: None,
            }),
        );
        let cache = OrgAllowlistCache::new(Arc::new(resolver), Duration::from_secs(60));
        assert!(cache.grant(&org(1)).await.expect("open").open);
    }
}

//! Resolve effective feature flags for an organization.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use everruns_platform::FeatureFlags;
use moka::future::Cache;

use crate::storage::StorageBackend;

/// How long a cached org row survives without an explicit invalidation.
///
/// Matches the skill-list cache in `services/capability.rs`. It only has to
/// cover the window where another replica wrote flags we did not see, because
/// in-process writes invalidate eagerly (see [`invalidate_org_feature_flags`]).
const ORG_FLAG_CACHE_TTL: Duration = Duration::from_secs(300);

/// Cached `org_id -> {flag_name: enabled}` rows, process-wide.
///
/// Deliberately a process global rather than a field on a service. Every writer
/// must be able to invalidate every reader, and the readers are spread across
/// structs that are built independently — `AuthState` for HTTP, `WorkerServiceImpl`
/// for gRPC, the MCP endpoint's state, `DirectWorkerAdapters`. One cache per
/// struct would mean a flag flip landing in one of them and not the others, which
/// is worse than no cache at all. One cache per process makes the invalidation in
/// `StorageBackend::replace_org_feature_flags` sufficient for all of them.
static ORG_FLAG_CACHE: LazyLock<Cache<i64, Arc<HashMap<String, bool>>>> = LazyLock::new(|| {
    Cache::builder()
        .time_to_live(ORG_FLAG_CACHE_TTL)
        .max_capacity(10_000)
        .build()
});

/// Read an org's flags straight from the database.
///
/// The settings API uses this: a user who just toggled a flag and reloaded the
/// page must see what they wrote, not a cached row.
pub async fn resolve_org_feature_flags(
    db: &StorageBackend,
    org_id: i64,
    system: &FeatureFlags,
) -> anyhow::Result<FeatureFlags> {
    let org_enabled = db.list_org_feature_flags(org_id).await?;
    Ok(FeatureFlags::for_org(system, &org_enabled))
}

/// The same answer, served from the process-wide cache where possible.
///
/// This is the one for request paths: it sits on every authenticated HTTP
/// request, every MCP call and every worker command, where the uncached version
/// costs a database round trip apiece. `system` is not part of the key — it is
/// env-derived and fixed for the life of the process, and `for_org` is pure — so
/// only the org's row is cached.
pub async fn resolve_org_feature_flags_cached(
    db: &StorageBackend,
    org_id: i64,
    system: &FeatureFlags,
) -> anyhow::Result<FeatureFlags> {
    if let Some(cached) = ORG_FLAG_CACHE.get(&org_id).await {
        return Ok(FeatureFlags::for_org(system, &cached));
    }

    let org_enabled = Arc::new(db.list_org_feature_flags(org_id).await?);
    ORG_FLAG_CACHE.insert(org_id, org_enabled.clone()).await;
    Ok(FeatureFlags::for_org(system, &org_enabled))
}

/// Drop an org's cached row.
///
/// Called from `StorageBackend::replace_org_feature_flags`, which is the only
/// way flags are written — so seeding, the settings API and tests all invalidate
/// without having to remember to.
pub async fn invalidate_org_feature_flags(org_id: i64) {
    ORG_FLAG_CACHE.invalidate(&org_id).await;
}

/// Settings rows a tenant admin may see and act on.
///
/// Platform-managed flags are left out entirely rather than shown disabled: a
/// toggle nobody in the org can move is an invitation to file a bug.
pub fn build_org_feature_flag_settings(
    system: &FeatureFlags,
    org_enabled: &HashMap<String, bool>,
) -> Vec<OrgFeatureFlagSetting> {
    build_settings(system, org_enabled, false)
}

/// Every settings row, including the platform-managed ones. For operator
/// surfaces (the super-admin console), which is the only place they can change.
pub fn build_all_feature_flag_settings(
    system: &FeatureFlags,
    org_enabled: &HashMap<String, bool>,
) -> Vec<OrgFeatureFlagSetting> {
    build_settings(system, org_enabled, true)
}

fn build_settings(
    system: &FeatureFlags,
    org_enabled: &HashMap<String, bool>,
    include_platform_managed: bool,
) -> Vec<OrgFeatureFlagSetting> {
    everruns_platform::API_FEATURE_FLAG_DEFINITIONS
        .iter()
        .filter(|def| include_platform_managed || !def.platform_managed)
        .map(|def| {
            let system_enabled = system.is_enabled(def.name);
            let org_on = org_enabled.get(def.name).copied().unwrap_or(false);
            let effective = system_enabled && org_on;
            OrgFeatureFlagSetting {
                name: def.name.to_string(),
                label: def.label.to_string(),
                description: def.description.to_string(),
                experimental: def.experimental,
                platform_managed: def.platform_managed,
                system_enabled,
                org_enabled: org_on,
                effective,
            }
        })
        .collect()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub struct OrgFeatureFlagSetting {
    pub name: String,
    pub label: String,
    pub description: String,
    pub experimental: bool,
    /// Whether only a platform user may enable this flag for the org.
    pub platform_managed: bool,
    /// Whether the deployment allows this flag (env / grade).
    pub system_enabled: bool,
    /// Whether the organization has opted in.
    pub org_enabled: bool,
    /// Effective value (`system_enabled && org_enabled`).
    pub effective: bool,
}

/// Validate a platform user's update to an organization's flags.
///
/// The mirror of [`validate_org_feature_flag_updates`]: this path may set the
/// platform-managed flags and nothing else, so neither actor can quietly do the
/// other's job.
pub fn validate_platform_feature_flag_updates(
    system: &FeatureFlags,
    updates: &HashMap<String, bool>,
) -> Result<(), String> {
    for (name, enabled) in updates {
        if !everruns_platform::is_platform_managed(name) {
            let known = everruns_platform::API_FEATURE_FLAG_DEFINITIONS
                .iter()
                .any(|d| d.name == name.as_str());
            return Err(if known {
                format!("Feature flag '{name}' is the organization's own setting")
            } else {
                format!("Unknown feature flag: {name}")
            });
        }
        if *enabled && !system.is_enabled(name) {
            return Err(format!(
                "Feature flag '{name}' is not available on this deployment"
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct OrgFeatureFlagsSettingsResponse {
    pub flags: Vec<OrgFeatureFlagSetting>,
}

/// Validate PATCH payload: only known API-visible flags; cannot enable when system off.
pub fn validate_org_feature_flag_updates(
    system: &FeatureFlags,
    updates: &HashMap<String, bool>,
) -> Result<(), String> {
    for (name, enabled) in updates {
        let known = everruns_platform::API_FEATURE_FLAG_DEFINITIONS
            .iter()
            .any(|d| d.name == name.as_str());
        if !known {
            return Err(format!("Unknown feature flag: {name}"));
        }
        if everruns_platform::is_platform_managed(name) {
            // Both directions: an org that cannot enrol itself should not be
            // able to unenrol either, or the next platform action looks flaky.
            return Err(format!(
                "Feature flag '{name}' is managed by the platform and cannot be \
                 changed by an organization"
            ));
        }
        if *enabled && !system.is_enabled(name) {
            return Err(format!(
                "Feature flag '{name}' is not available on this deployment"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system_with_everything() -> FeatureFlags {
        FeatureFlags {
            environments: true,
            skills: true,
            ..FeatureFlags::default()
        }
    }

    #[test]
    fn an_org_cannot_set_a_platform_managed_flag_in_either_direction() {
        let system = system_with_everything();

        for wanted in [true, false] {
            let error = validate_org_feature_flag_updates(
                &system,
                &HashMap::from([("environments".to_string(), wanted)]),
            )
            .expect_err("the org does not own this flag");
            assert!(error.contains("managed by the platform"), "{error}");
        }
    }

    #[test]
    fn a_platform_user_cannot_set_the_orgs_own_flags() {
        let system = system_with_everything();

        // Acting as the tenant is exactly what the operator console does not do.
        let error = validate_platform_feature_flag_updates(
            &system,
            &HashMap::from([("skills".to_string(), true)]),
        )
        .expect_err("skills is the org's own setting");
        assert!(error.contains("the organization's own setting"), "{error}");

        let unknown = validate_platform_feature_flag_updates(
            &system,
            &HashMap::from([("nope".to_string(), true)]),
        )
        .expect_err("unknown flag");
        assert!(unknown.contains("Unknown feature flag"), "{unknown}");
    }

    #[test]
    fn a_platform_user_enrols_an_org_only_where_the_deployment_allows_it() {
        let updates = HashMap::from([("environments".to_string(), true)]);

        validate_platform_feature_flag_updates(&system_with_everything(), &updates)
            .expect("enabled on this deployment");

        let error = validate_platform_feature_flag_updates(&FeatureFlags::default(), &updates)
            .expect_err("the deployment gate still comes first");
        assert!(
            error.contains("not available on this deployment"),
            "{error}"
        );
    }

    #[test]
    fn the_tenant_settings_list_hides_what_the_tenant_cannot_change() {
        let system = system_with_everything();
        let org_enabled = HashMap::new();

        let tenant = build_org_feature_flag_settings(&system, &org_enabled);
        assert!(
            !tenant.iter().any(|row| row.name == "environments"),
            "a toggle nobody in the org can move is an invitation to file a bug"
        );

        let platform = build_all_feature_flag_settings(&system, &org_enabled);
        let row = platform
            .iter()
            .find(|row| row.name == "environments")
            .expect("the operator console sees it");
        assert!(row.platform_managed);
        assert!(!row.effective, "enrolment is still off until someone acts");
    }

    /// The cache must not outlive a write. `replace_org_feature_flags` is the
    /// only write path, and it invalidates — so a reader that just cached the
    /// old row sees the new one on its next call, with no call-site cooperation.
    #[tokio::test]
    async fn a_write_invalidates_what_a_reader_cached() {
        let db = StorageBackend::in_memory();
        let system = system_with_everything();
        let org_id = 987_001;

        let before = resolve_org_feature_flags_cached(&db, org_id, &system)
            .await
            .expect("first read populates the cache");
        assert!(!before.is_enabled("skills"), "nothing opted in yet");

        db.replace_org_feature_flags(org_id, &HashMap::from([("skills".to_string(), true)]))
            .await
            .expect("write flags");

        let after = resolve_org_feature_flags_cached(&db, org_id, &system)
            .await
            .expect("second read");
        assert!(
            after.is_enabled("skills"),
            "cached row survived a write — every reader would be serving a stale flag"
        );
    }

    /// Two orgs share one cache; invalidating one must not disturb the other,
    /// and one org's row must never answer for another's.
    #[tokio::test]
    async fn orgs_do_not_share_a_cached_row() {
        let db = StorageBackend::in_memory();
        let system = system_with_everything();
        let (a, b) = (987_002, 987_003);

        db.replace_org_feature_flags(a, &HashMap::from([("skills".to_string(), true)]))
            .await
            .expect("write a");

        let flags_a = resolve_org_feature_flags_cached(&db, a, &system)
            .await
            .expect("read a");
        let flags_b = resolve_org_feature_flags_cached(&db, b, &system)
            .await
            .expect("read b");

        assert!(flags_a.is_enabled("skills"));
        assert!(!flags_b.is_enabled("skills"), "org b picked up org a's row");
    }

    /// The settings API reads uncached on purpose: someone who just toggled a
    /// flag and reloaded must see what they wrote even if a replica cached the
    /// old row. This pins the two functions as genuinely different.
    #[tokio::test]
    async fn the_uncached_read_ignores_the_cache() {
        let db = StorageBackend::in_memory();
        let system = system_with_everything();
        let org_id = 987_004;

        resolve_org_feature_flags_cached(&db, org_id, &system)
            .await
            .expect("populate the cache with the empty row");

        // Write behind the cache's back, the way another replica would.
        db.list_org_feature_flags(org_id).await.expect("row reads");
        let mut flags = HashMap::new();
        flags.insert("skills".to_string(), true);
        db.replace_org_feature_flags(org_id, &flags)
            .await
            .expect("write");
        ORG_FLAG_CACHE
            .insert(org_id, Arc::new(HashMap::new()))
            .await;

        let fresh = resolve_org_feature_flags(&db, org_id, &system)
            .await
            .expect("uncached read");
        assert!(
            fresh.is_enabled("skills"),
            "the uncached read served a cached row"
        );
    }

    /// The cached read must actually skip the database, not just return the
    /// right answer. Seed the cache with a row the database does not have: if
    /// the read still queries, it returns the database's answer and this fails.
    #[tokio::test]
    async fn the_cached_read_serves_from_the_cache() {
        let db = StorageBackend::in_memory();
        let system = system_with_everything();
        let org_id = 987_005;

        ORG_FLAG_CACHE
            .insert(
                org_id,
                Arc::new(HashMap::from([("skills".to_string(), true)])),
            )
            .await;

        let flags = resolve_org_feature_flags_cached(&db, org_id, &system)
            .await
            .expect("cached read");
        assert!(
            flags.is_enabled("skills"),
            "the read went to the database instead of the cache"
        );
    }
}

//! Resolve effective feature flags for an organization.

use std::collections::HashMap;

use everruns_platform::FeatureFlags;

use crate::storage::StorageBackend;

pub async fn resolve_org_feature_flags(
    db: &StorageBackend,
    org_id: i64,
    system: &FeatureFlags,
) -> anyhow::Result<FeatureFlags> {
    let org_enabled = db.list_org_feature_flags(org_id).await?;
    Ok(FeatureFlags::for_org(system, &org_enabled))
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
        let mut flags = FeatureFlags::default();
        flags.environments = true;
        flags.skills = true;
        flags
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
}

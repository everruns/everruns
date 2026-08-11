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

pub fn build_org_feature_flag_settings(
    system: &FeatureFlags,
    org_enabled: &HashMap<String, bool>,
) -> Vec<OrgFeatureFlagSetting> {
    everruns_platform::API_FEATURE_FLAG_DEFINITIONS
        .iter()
        .map(|def| {
            let system_enabled = system.is_enabled(def.name);
            let org_on = org_enabled.get(def.name).copied().unwrap_or(false);
            let effective = system_enabled && org_on;
            OrgFeatureFlagSetting {
                name: def.name.to_string(),
                label: def.label.to_string(),
                description: def.description.to_string(),
                experimental: def.experimental,
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
    /// Whether the deployment allows this flag (env / grade).
    pub system_enabled: bool,
    /// Whether the organization has opted in.
    pub org_enabled: bool,
    /// Effective value (`system_enabled && org_enabled`).
    pub effective: bool,
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
        if *enabled && !system.is_enabled(name) {
            return Err(format!(
                "Feature flag '{name}' is not available on this deployment"
            ));
        }
    }
    Ok(())
}

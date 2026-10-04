//! Rollout-grade policy and durable organisation overrides.

use crate::records::{FeatureFlagGrade, FeatureFlagPolicy, FeatureFlags};
use crate::storage::StorageBackend;
use std::collections::HashMap;

/// Overrides are authorization inputs: read durable state on every enforcement
/// path so a revocation committed by another replica is observed.
pub async fn resolve_org_feature_flags(
    db: &StorageBackend,
    org_id: i64,
    policy: &FeatureFlagPolicy,
) -> anyhow::Result<FeatureFlags> {
    Ok(policy.for_org(&db.list_org_feature_flags(org_id).await?))
}

pub fn build_org_feature_flag_settings(
    policy: &FeatureFlagPolicy,
    overrides: &HashMap<String, bool>,
) -> Vec<OrgFeatureFlagSetting> {
    build_settings(policy, overrides, false)
}

pub fn build_all_feature_flag_settings(
    policy: &FeatureFlagPolicy,
    overrides: &HashMap<String, bool>,
) -> Vec<OrgFeatureFlagSetting> {
    build_settings(policy, overrides, true)
}

fn build_settings(
    policy: &FeatureFlagPolicy,
    overrides: &HashMap<String, bool>,
    platform: bool,
) -> Vec<OrgFeatureFlagSetting> {
    crate::records::API_FEATURE_FLAG_DEFINITIONS
        .iter()
        .filter_map(|definition| {
            let grade = policy.grade(definition.name);
            let org_configurable = grade.org_configurable(policy.deployment);
            // Tenant settings expose only flags the tenant can change. Platform
            // operators see every grade and may enrol internal/adoption features.
            if !platform && !org_configurable {
                return None;
            }
            let org_override = overrides.get(definition.name).copied();
            Some(OrgFeatureFlagSetting {
                name: definition.name.into(),
                label: definition.label.into(),
                description: definition.description.into(),
                grade,
                system_enabled: grade.available(policy.deployment),
                default_enabled: grade.available(policy.deployment) && grade.default_enabled(),
                org_override,
                effective: grade.effective(policy.deployment, org_override),
                can_manage: if platform {
                    matches!(
                        grade,
                        FeatureFlagGrade::Internal | FeatureFlagGrade::Adoption
                    )
                } else {
                    org_configurable
                },
            })
        })
        .collect()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct OrgFeatureFlagSetting {
    pub name: String,
    pub label: String,
    pub description: String,
    pub grade: FeatureFlagGrade,
    pub system_enabled: bool,
    pub default_enabled: bool,
    /// None inherits the grade default; false is a durable prod opt-out.
    pub org_override: Option<bool>,
    pub effective: bool,
    pub can_manage: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct OrgFeatureFlagsSettingsResponse {
    pub flags: Vec<OrgFeatureFlagSetting>,
}

pub fn validate_org_feature_flag_updates(
    policy: &FeatureFlagPolicy,
    updates: &HashMap<String, bool>,
) -> Result<(), String> {
    validate_updates(policy, updates, false)
}

pub fn validate_platform_feature_flag_updates(
    policy: &FeatureFlagPolicy,
    updates: &HashMap<String, bool>,
) -> Result<(), String> {
    validate_updates(policy, updates, true)
}

fn validate_updates(
    policy: &FeatureFlagPolicy,
    updates: &HashMap<String, bool>,
    platform: bool,
) -> Result<(), String> {
    for name in updates.keys() {
        if !crate::records::API_FEATURE_FLAG_DEFINITIONS
            .iter()
            .any(|definition| definition.name == name)
        {
            return Err(format!("Unknown feature flag: {name}"));
        }
        let grade = policy.grade(name);
        if !grade.available(policy.deployment) {
            return Err(format!(
                "Feature flag '{name}' is not available on this deployment"
            ));
        }
        if platform {
            if !matches!(
                grade,
                FeatureFlagGrade::Internal | FeatureFlagGrade::Adoption
            ) {
                return Err(format!(
                    "Feature flag '{name}' is the organization's own setting"
                ));
            }
        } else if !grade.org_configurable(policy.deployment) {
            return Err(format!(
                "Feature flag '{name}' is managed by the platform and cannot be changed by an organization"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::DeploymentGrade;

    fn policy(grade: FeatureFlagGrade, deployment: DeploymentGrade) -> FeatureFlagPolicy {
        FeatureFlagPolicy::from_env(deployment).with_grade("skills", grade)
    }

    #[test]
    fn both_update_directions_enforce_grade_ownership() {
        for deployment in [DeploymentGrade::Dev, DeploymentGrade::Prod] {
            for grade in [
                FeatureFlagGrade::Dev,
                FeatureFlagGrade::Internal,
                FeatureFlagGrade::Adoption,
                FeatureFlagGrade::Prod,
                FeatureFlagGrade::Off,
            ] {
                let policy = policy(grade, deployment);
                for enabled in [false, true] {
                    let updates = HashMap::from([("skills".into(), enabled)]);
                    assert_eq!(
                        validate_org_feature_flag_updates(&policy, &updates).is_ok(),
                        grade.org_configurable(deployment)
                    );
                    assert_eq!(
                        validate_platform_feature_flag_updates(&policy, &updates).is_ok(),
                        matches!(
                            grade,
                            FeatureFlagGrade::Internal | FeatureFlagGrade::Adoption
                        )
                    );
                }
            }
        }
    }

    #[test]
    fn settings_show_defaults_overrides_and_actual_edit_authority() {
        let overrides = HashMap::from([("skills".into(), false)]);
        let prod = policy(FeatureFlagGrade::Prod, DeploymentGrade::Prod);
        let rows = build_org_feature_flag_settings(&prod, &overrides);
        let row = rows.iter().find(|row| row.name == "skills").unwrap();
        assert!(row.default_enabled);
        assert_eq!(row.org_override, Some(false));
        assert!(!row.effective);
        assert!(row.can_manage);
        for grade in [
            FeatureFlagGrade::Off,
            FeatureFlagGrade::Dev,
            FeatureFlagGrade::Internal,
        ] {
            let policy = policy(grade, DeploymentGrade::Prod);
            assert!(
                !build_org_feature_flag_settings(&policy, &overrides)
                    .iter()
                    .any(|row| row.name == "skills")
            );
            let rows = build_all_feature_flag_settings(&policy, &overrides);
            assert_eq!(
                rows.iter()
                    .find(|row| row.name == "skills")
                    .unwrap()
                    .can_manage,
                grade == FeatureFlagGrade::Internal
            );
        }
    }

    #[tokio::test]
    async fn durable_opt_out_survives_reads_and_is_isolated_by_org() {
        let db = StorageBackend::in_memory();
        let policy = policy(FeatureFlagGrade::Prod, DeploymentGrade::Prod);
        assert!(
            resolve_org_feature_flags(&db, 1, &policy)
                .await
                .unwrap()
                .skills
        );
        db.replace_org_feature_flags(1, &HashMap::from([("skills".into(), false)]))
            .await
            .unwrap();
        assert_eq!(
            db.list_org_feature_flags(1).await.unwrap().get("skills"),
            Some(&false)
        );
        assert!(
            !resolve_org_feature_flags(&db, 1, &policy)
                .await
                .unwrap()
                .skills
        );
        assert!(
            resolve_org_feature_flags(&db, 2, &policy)
                .await
                .unwrap()
                .skills
        );
        db.replace_org_feature_flags(1, &HashMap::from([("evals".into(), true)]))
            .await
            .unwrap();
        assert!(
            !resolve_org_feature_flags(&db, 1, &policy)
                .await
                .unwrap()
                .skills
        );
        db.replace_org_feature_flags(1, &HashMap::from([("skills".into(), true)]))
            .await
            .unwrap();
        assert!(
            resolve_org_feature_flags(&db, 1, &policy)
                .await
                .unwrap()
                .skills
        );
    }
}

#[cfg(test)]
mod platform_adoption_tests {
    use super::*;
    #[test]
    fn platform_adoption_authority_matches_catalog_and_preserves_tenant_access() {
        let policy = FeatureFlagPolicy::from_env(everruns_core::DeploymentGrade::Prod)
            .with_grade("skills", FeatureFlagGrade::Adoption);
        for enabled in [true, false] {
            let updates = HashMap::from([("skills".into(), enabled)]);
            assert!(validate_platform_feature_flag_updates(&policy, &updates).is_ok());
            assert!(validate_org_feature_flag_updates(&policy, &updates).is_ok());
        }
        let rows = build_all_feature_flag_settings(&policy, &HashMap::new());
        let row = rows.iter().find(|row| row.name == "skills").unwrap();
        assert!(row.can_manage);
        assert!(!row.effective);
    }
}

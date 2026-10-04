//! Feature rollout policy shared by registration and hosted org resolution.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::DeploymentGrade;

/// Rollout policy for one feature, independent of the running deployment grade.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FeatureFlagGrade {
    /// Available only on local development deployments.
    Dev,
    /// Available after platform-owned organisation enrolment.
    Internal,
    /// Available after organisation opt-in.
    Adoption,
    /// Enabled by default with organisation opt-out.
    Prod,
    #[default]
    /// Unavailable regardless of organisation overrides.
    Off,
}

impl FeatureFlagGrade {
    /// Whether registration and routes may exist on this deployment.
    pub fn available(self, deployment: DeploymentGrade) -> bool {
        match self {
            Self::Off => false,
            Self::Dev => deployment.is_dev(),
            Self::Internal | Self::Adoption | Self::Prod => true,
        }
    }

    /// Whether an organisation inherits an enabled default when available.
    pub fn default_enabled(self) -> bool {
        matches!(self, Self::Dev | Self::Prod)
    }

    /// Whether an organisation may manage its own override.
    pub fn org_configurable(self, deployment: DeploymentGrade) -> bool {
        self.available(deployment) && matches!(self, Self::Dev | Self::Adoption | Self::Prod)
    }

    /// Resolve availability, the grade default, and an optional organisation override.
    pub fn effective(self, deployment: DeploymentGrade, org_override: Option<bool>) -> bool {
        self.available(deployment) && org_override.unwrap_or(self.default_enabled())
    }

    /// Invalid overrides fail closed; a typo must never promote a feature.
    pub fn from_env(env_var: &str, default: Self) -> Self {
        match std::env::var(env_var) {
            Ok(value) => value.parse().unwrap_or_else(|_| {
                tracing::warn!(env_var, value, "Invalid feature grade; disabling feature");
                Self::Off
            }),
            Err(std::env::VarError::NotPresent) => default,
            Err(std::env::VarError::NotUnicode(_)) => {
                tracing::warn!(env_var, "Invalid feature grade encoding; disabling feature");
                Self::Off
            }
        }
    }
}

impl fmt::Display for FeatureFlagGrade {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Dev => "dev",
            Self::Internal => "internal",
            Self::Adoption => "adoption",
            Self::Prod => "prod",
            Self::Off => "off",
        })
    }
}

impl FromStr for FeatureFlagGrade {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "dev" => Ok(Self::Dev),
            "internal" => Ok(Self::Internal),
            "adoption" => Ok(Self::Adoption),
            "prod" => Ok(Self::Prod),
            "off" => Ok(Self::Off),
            _ => Err("expected dev, internal, adoption, prod, or off"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grade_policy_covers_every_deployment_and_override() {
        for deployment in [
            DeploymentGrade::Dev,
            DeploymentGrade::Poc,
            DeploymentGrade::Preview,
            DeploymentGrade::Prod,
        ] {
            for (grade, available, default, tenant) in [
                (
                    FeatureFlagGrade::Dev,
                    deployment.is_dev(),
                    deployment.is_dev(),
                    deployment.is_dev(),
                ),
                (FeatureFlagGrade::Internal, true, false, false),
                (FeatureFlagGrade::Adoption, true, false, true),
                (FeatureFlagGrade::Prod, true, true, true),
                (FeatureFlagGrade::Off, false, false, false),
            ] {
                assert_eq!(grade.available(deployment), available);
                assert_eq!(grade.org_configurable(deployment), tenant);
                assert_eq!(grade.effective(deployment, None), default);
                assert_eq!(grade.effective(deployment, Some(true)), available);
                assert!(!grade.effective(deployment, Some(false)));
            }
        }
    }

    #[test]
    fn grade_wire_values_are_explicit() {
        for value in ["dev", "internal", "adoption", "prod", "off"] {
            let grade: FeatureFlagGrade = value.parse().unwrap();
            assert_eq!(grade.to_string(), value);
            assert_eq!(serde_json::to_value(grade).unwrap(), value);
        }
        for value in ["preview", "true", "false", "1", "0", "DEV", "", " prod "] {
            assert!(value.parse::<FeatureFlagGrade>().is_err());
        }
    }
}

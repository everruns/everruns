//! Deployment grade configuration
//!
//! This module defines the deployment grade (environment tier) which controls
//! which features and capabilities are available in each environment.
//!
//! Grades:
//! - `dev`: Development environment, all experimental features enabled
//! - `poc`: Proof of concept / demo environment
//! - `preview`: Preview/staging environment
//! - `prod`: Production environment, only stable features

use std::fmt;
use std::str::FromStr;

/// Deployment grade (environment tier)
///
/// Controls which features are available. More permissive grades include
/// experimental features that may not be stable or secure for production.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeploymentGrade {
    /// Development environment - all experimental features enabled
    Dev,
    /// Proof of concept / demo environment
    Poc,
    /// Preview/staging environment
    Preview,
    /// Production environment - only stable features
    #[default]
    Prod,
}

impl DeploymentGrade {
    /// Returns true if this is a development environment
    pub fn is_dev(&self) -> bool {
        matches!(self, DeploymentGrade::Dev)
    }

    /// Returns true if experimental features should be enabled
    ///
    /// Currently only enabled in dev environments
    pub fn experimental_features_enabled(&self) -> bool {
        matches!(self, DeploymentGrade::Dev)
    }

    /// Parse from environment variable
    ///
    /// Reads DEPLOYMENT_GRADE env var. Returns Dev if DEV_MODE=true,
    /// otherwise defaults to Prod.
    pub fn from_env() -> Self {
        // Check explicit DEPLOYMENT_GRADE first
        if let Ok(grade) = std::env::var("DEPLOYMENT_GRADE") {
            return grade.parse().unwrap_or_default();
        }

        // Fall back to DEV_MODE for backwards compatibility
        let dev_mode = std::env::var("DEV_MODE")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        if dev_mode {
            DeploymentGrade::Dev
        } else {
            DeploymentGrade::Prod
        }
    }
}

impl fmt::Display for DeploymentGrade {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeploymentGrade::Dev => write!(f, "dev"),
            DeploymentGrade::Poc => write!(f, "poc"),
            DeploymentGrade::Preview => write!(f, "preview"),
            DeploymentGrade::Prod => write!(f, "prod"),
        }
    }
}

impl FromStr for DeploymentGrade {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "dev" | "development" => Ok(DeploymentGrade::Dev),
            "poc" => Ok(DeploymentGrade::Poc),
            "preview" | "staging" => Ok(DeploymentGrade::Preview),
            "prod" | "production" => Ok(DeploymentGrade::Prod),
            _ => Err(format!(
                "Unknown deployment grade: '{}'. Valid values: dev, poc, preview, prod",
                s
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grades_parse_aliases_and_case_into_canonical_policy() {
        for (grade, canonical, aliases, experimental) in [
            (
                DeploymentGrade::Dev,
                "dev",
                vec!["dev", "development"],
                true,
            ),
            (DeploymentGrade::Poc, "poc", vec!["poc"], false),
            (
                DeploymentGrade::Preview,
                "preview",
                vec!["preview", "staging"],
                false,
            ),
            (
                DeploymentGrade::Prod,
                "prod",
                vec!["prod", "production"],
                false,
            ),
        ] {
            for alias in aliases {
                for input in [alias.to_owned(), alias.to_ascii_uppercase()] {
                    assert_eq!(input.parse::<DeploymentGrade>().unwrap(), grade, "{input}");
                }
            }
            assert_eq!(grade.to_string(), canonical);
            assert_eq!(grade.experimental_features_enabled(), experimental);
            assert_eq!(grade.is_dev(), experimental);
        }
    }

    #[test]
    fn invalid_grade_inputs_are_rejected() {
        for input in ["", " ", " dev ", "development-extra", "test", "production1"] {
            assert!(input.parse::<DeploymentGrade>().is_err(), "{input:?}");
        }
    }
}

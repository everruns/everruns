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
    fn test_parse_grades() {
        assert_eq!("dev".parse::<DeploymentGrade>().unwrap(), DeploymentGrade::Dev);
        assert_eq!("development".parse::<DeploymentGrade>().unwrap(), DeploymentGrade::Dev);
        assert_eq!("poc".parse::<DeploymentGrade>().unwrap(), DeploymentGrade::Poc);
        assert_eq!("preview".parse::<DeploymentGrade>().unwrap(), DeploymentGrade::Preview);
        assert_eq!("staging".parse::<DeploymentGrade>().unwrap(), DeploymentGrade::Preview);
        assert_eq!("prod".parse::<DeploymentGrade>().unwrap(), DeploymentGrade::Prod);
        assert_eq!("production".parse::<DeploymentGrade>().unwrap(), DeploymentGrade::Prod);
    }

    #[test]
    fn test_case_insensitive() {
        assert_eq!("DEV".parse::<DeploymentGrade>().unwrap(), DeploymentGrade::Dev);
        assert_eq!("PROD".parse::<DeploymentGrade>().unwrap(), DeploymentGrade::Prod);
    }

    #[test]
    fn test_default() {
        assert_eq!(DeploymentGrade::default(), DeploymentGrade::Prod);
    }

    #[test]
    fn test_experimental_features() {
        assert!(DeploymentGrade::Dev.experimental_features_enabled());
        assert!(!DeploymentGrade::Poc.experimental_features_enabled());
        assert!(!DeploymentGrade::Preview.experimental_features_enabled());
        assert!(!DeploymentGrade::Prod.experimental_features_enabled());
    }

    #[test]
    fn test_display() {
        assert_eq!(DeploymentGrade::Dev.to_string(), "dev");
        assert_eq!(DeploymentGrade::Poc.to_string(), "poc");
        assert_eq!(DeploymentGrade::Preview.to_string(), "preview");
        assert_eq!(DeploymentGrade::Prod.to_string(), "prod");
    }
}

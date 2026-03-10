// Feature flags system
//
// Decision: Feature flags are system-level, computed from env vars + deployment grade.
// Decision: Flags marked "experimental" auto-enable in dev (DeploymentGrade::Dev).
// Decision: Explicit env var (FEATURE_<NAME>=true/false) always takes priority.
// Decision: Struct-based for type safety; `is_enabled(&str)` for dynamic lookup.
// Decision: Future extensibility: per-org/per-user flags, external providers (LaunchDarkly).
// Decision: No database storage needed yet — env vars + deployment grade suffice.

use serde::{Deserialize, Serialize};

use crate::deployment::DeploymentGrade;

/// System-level feature flags.
///
/// Currently backed by environment variables and deployment grade.
/// Future: per-org flags, per-user flags, external providers.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeatureFlags {
    /// Global chat (per-user singleton chat session). Experimental.
    pub global_chat: bool,
    /// Apps (agent deployment to distribution channels). Experimental.
    pub apps: bool,
    /// Global search / command palette (Cmd+K). Experimental.
    pub global_search: bool,
}

impl FeatureFlags {
    /// Compute feature flags from environment variables and deployment grade.
    pub fn from_env(grade: &DeploymentGrade) -> Self {
        Self {
            global_chat: experimental_flag("FEATURE_GLOBAL_CHAT", grade),
            apps: experimental_flag("FEATURE_APPS", grade),
            global_search: experimental_flag("FEATURE_GLOBAL_SEARCH", grade),
        }
    }

    /// Look up a flag by name (for dynamic/string-based access).
    pub fn is_enabled(&self, flag: &str) -> bool {
        match flag {
            "global_chat" => self.global_chat,
            "apps" => self.apps,
            "global_search" => self.global_search,
            _ => false,
        }
    }

    /// All flags enabled (for testing).
    #[cfg(test)]
    pub fn all_enabled() -> Self {
        Self {
            global_chat: true,
            apps: true,
            global_search: true,
        }
    }
}

/// Resolve an experimental flag.
///
/// Priority: explicit env var > experimental default (enabled in dev) > false.
fn experimental_flag(env_var: &str, grade: &DeploymentGrade) -> bool {
    if let Ok(val) = std::env::var(env_var) {
        return val == "true" || val == "1";
    }
    grade.experimental_features_enabled()
}

/// Resolve a standard (non-experimental) flag.
///
/// Priority: explicit env var > default.
#[allow(dead_code)]
fn standard_flag(env_var: &str, default: bool) -> bool {
    std::env::var(env_var)
        .map(|v| v == "true" || v == "1")
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_flags() {
        let flags = FeatureFlags::default();
        assert!(!flags.global_chat);
        assert!(!flags.apps);
        assert!(!flags.global_search);
    }

    // SAFETY: env var tests must run single-threaded (--test-threads=1).
    // set_var/remove_var are unsafe in edition 2024 due to thread-safety.

    #[test]
    fn test_experimental_enabled_in_dev() {
        unsafe { std::env::remove_var("FEATURE_GLOBAL_CHAT") };
        unsafe { std::env::remove_var("FEATURE_APPS") };
        unsafe { std::env::remove_var("FEATURE_GLOBAL_SEARCH") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Dev);
        assert!(flags.global_chat);
        assert!(flags.apps);
        assert!(flags.global_search);
    }

    #[test]
    fn test_experimental_disabled_in_prod() {
        unsafe { std::env::remove_var("FEATURE_GLOBAL_CHAT") };
        unsafe { std::env::remove_var("FEATURE_APPS") };
        unsafe { std::env::remove_var("FEATURE_GLOBAL_SEARCH") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(!flags.global_chat);
        assert!(!flags.apps);
        assert!(!flags.global_search);
    }

    #[test]
    fn test_env_override_enables_in_prod() {
        unsafe { std::env::set_var("FEATURE_GLOBAL_CHAT", "true") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(flags.global_chat);
        unsafe { std::env::remove_var("FEATURE_GLOBAL_CHAT") };
    }

    #[test]
    fn test_env_override_disables_in_dev() {
        unsafe { std::env::set_var("FEATURE_GLOBAL_CHAT", "false") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Dev);
        assert!(!flags.global_chat);
        unsafe { std::env::remove_var("FEATURE_GLOBAL_CHAT") };
    }

    #[test]
    fn test_is_enabled_dynamic() {
        let flags = FeatureFlags {
            global_chat: true,
            apps: true,
            global_search: true,
        };
        assert!(flags.is_enabled("global_chat"));
        assert!(flags.is_enabled("apps"));
        assert!(flags.is_enabled("global_search"));
        assert!(!flags.is_enabled("nonexistent"));
    }

    #[test]
    fn test_serialization() {
        let flags = FeatureFlags {
            global_chat: true,
            apps: true,
            global_search: true,
        };
        let json = serde_json::to_string(&flags).unwrap();
        assert!(json.contains("\"global_chat\":true"));
        assert!(json.contains("\"global_search\":true"));

        let parsed: FeatureFlags = serde_json::from_str(&json).unwrap();
        assert_eq!(flags, parsed);
    }

    #[test]
    fn test_standard_flag() {
        unsafe { std::env::remove_var("FEATURE_TEST_STD") };
        assert!(!standard_flag("FEATURE_TEST_STD", false));
        assert!(standard_flag("FEATURE_TEST_STD", true));

        unsafe { std::env::set_var("FEATURE_TEST_STD", "1") };
        assert!(standard_flag("FEATURE_TEST_STD", false));
        unsafe { std::env::remove_var("FEATURE_TEST_STD") };
    }
}

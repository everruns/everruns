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
    /// In-app notifications (bell, toasts, notification SSE). Standard.
    pub notifications: bool,
    /// MCP endpoint (POST /mcp — Everruns as an MCP server). Experimental.
    pub mcp_endpoint: bool,
    /// Evals (user-facing behavioral evals for agents). Experimental.
    pub evals: bool,
    /// Docker container capability. Standard (disabled by default on all envs).
    pub docker_capability: bool,
}

impl FeatureFlags {
    /// Compute feature flags from environment variables and deployment grade.
    pub fn from_env(grade: &DeploymentGrade) -> Self {
        Self {
            global_chat: experimental_flag("FEATURE_GLOBAL_CHAT", grade),
            apps: experimental_flag("FEATURE_APPS", grade),
            notifications: standard_flag("FEATURE_NOTIFICATIONS", false),
            mcp_endpoint: experimental_flag("FEATURE_MCP_ENDPOINT", grade),
            evals: experimental_flag("FEATURE_EVALS", grade),
            docker_capability: standard_flag("FEATURE_DOCKER_CAPABILITY", false),
        }
    }

    /// Look up a flag by name (for dynamic/string-based access).
    pub fn is_enabled(&self, flag: &str) -> bool {
        match flag {
            "global_chat" => self.global_chat,
            "apps" => self.apps,
            "notifications" => self.notifications,
            "mcp_endpoint" => self.mcp_endpoint,
            "evals" => self.evals,
            "docker_capability" => self.docker_capability,
            _ => false,
        }
    }

    /// All flags enabled (for testing).
    #[cfg(test)]
    pub fn all_enabled() -> Self {
        Self {
            global_chat: true,
            apps: true,
            notifications: true,
            mcp_endpoint: true,
            evals: true,
            docker_capability: true,
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
fn standard_flag(env_var: &str, default: bool) -> bool {
    std::env::var(env_var)
        .map(|v| v == "true" || v == "1")
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Env-var-mutating tests must not run in parallel.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn test_default_flags() {
        let flags = FeatureFlags::default();
        assert!(!flags.global_chat);
        assert!(!flags.apps);
        assert!(!flags.notifications);
        assert!(!flags.docker_capability);
    }

    // SAFETY: env var tests must run single-threaded (--test-threads=1).
    // set_var/remove_var are unsafe in edition 2024 due to thread-safety.

    #[test]
    fn test_experimental_enabled_in_dev() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_GLOBAL_CHAT") };
        unsafe { std::env::remove_var("FEATURE_APPS") };
        unsafe { std::env::remove_var("FEATURE_EVALS") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Dev);
        assert!(flags.global_chat);
        assert!(flags.apps);
        assert!(flags.evals);
    }

    #[test]
    fn test_experimental_disabled_in_prod() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_GLOBAL_CHAT") };
        unsafe { std::env::remove_var("FEATURE_APPS") };
        unsafe { std::env::remove_var("FEATURE_EVALS") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(!flags.global_chat);
        assert!(!flags.apps);
        assert!(!flags.evals);
    }

    #[test]
    fn test_env_override_enables_in_prod() {
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_GLOBAL_CHAT", "true") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(flags.global_chat);
        unsafe { std::env::remove_var("FEATURE_GLOBAL_CHAT") };
    }

    #[test]
    fn test_env_override_disables_in_dev() {
        let _lock = lock_env();
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
            notifications: true,
            mcp_endpoint: true,
            evals: true,
            docker_capability: true,
        };
        assert!(flags.is_enabled("global_chat"));
        assert!(flags.is_enabled("apps"));
        assert!(flags.is_enabled("notifications"));
        assert!(flags.is_enabled("mcp_endpoint"));
        assert!(flags.is_enabled("evals"));
        assert!(flags.is_enabled("docker_capability"));
        assert!(!flags.is_enabled("nonexistent"));
    }

    #[test]
    fn test_serialization() {
        let flags = FeatureFlags {
            global_chat: true,
            apps: true,
            notifications: true,
            mcp_endpoint: true,
            evals: true,
            docker_capability: true,
        };
        let json = serde_json::to_string(&flags).unwrap();
        assert!(json.contains("\"global_chat\":true"));
        assert!(json.contains("\"notifications\":true"));

        let parsed: FeatureFlags = serde_json::from_str(&json).unwrap();
        assert_eq!(flags, parsed);
    }

    #[test]
    fn test_standard_flag() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_TEST_STD") };
        assert!(!standard_flag("FEATURE_TEST_STD", false));
        assert!(standard_flag("FEATURE_TEST_STD", true));

        unsafe { std::env::set_var("FEATURE_TEST_STD", "1") };
        assert!(standard_flag("FEATURE_TEST_STD", false));
        unsafe { std::env::remove_var("FEATURE_TEST_STD") };
    }

    #[test]
    fn test_standard_notifications_flag_defaults_off() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_NOTIFICATIONS") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Dev);
        assert!(!flags.notifications);
    }

    #[test]
    fn test_standard_notifications_flag_respects_env_override() {
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_NOTIFICATIONS", "true") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(flags.notifications);
        unsafe { std::env::remove_var("FEATURE_NOTIFICATIONS") };
    }

    #[test]
    fn test_docker_capability_flag_disabled_by_default_in_dev() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_DOCKER_CAPABILITY") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Dev);
        assert!(
            !flags.docker_capability,
            "docker_capability should be disabled by default even in dev"
        );
    }

    #[test]
    fn test_docker_capability_flag_enabled_by_env_override() {
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_DOCKER_CAPABILITY", "true") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(flags.docker_capability);
        unsafe { std::env::remove_var("FEATURE_DOCKER_CAPABILITY") };
    }
}

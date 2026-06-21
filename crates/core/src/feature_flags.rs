// Feature flags system
//
// Decision: Feature flags are system-level, computed from env vars + deployment grade.
// Decision: Flags marked "experimental" auto-enable in dev (DeploymentGrade::Dev).
// Decision: Explicit env var (FEATURE_<NAME>=true/false) always takes priority.
// Decision: Struct-based for type safety; `is_enabled(&str)` for dynamic lookup.
// Decision: Two structs — FeatureFlags (API-visible) and InternalFeatureFlags (backend-only).
// Decision: Future extensibility: per-org/per-user flags, external providers (LaunchDarkly).
// Decision: No database storage needed yet — env vars + deployment grade suffice.

use serde::{Deserialize, Serialize};

use crate::deployment::DeploymentGrade;

/// Feature flags exposed via `GET /v1/feature-flags` and consumed by the frontend.
///
/// Currently backed by environment variables and deployment grade.
/// Future: per-org flags, per-user flags, external providers.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FeatureFlags {
    /// Global chat (per-user singleton chat session). Experimental.
    pub global_chat: bool,
    /// In-app notifications (bell, toasts, notification SSE). Experimental.
    pub notifications: bool,
    /// MCP endpoint (POST /mcp — Everruns as an MCP server). Experimental.
    pub mcp_endpoint: bool,
    /// Evals (user-facing behavioral evals for agents). Experimental.
    pub evals: bool,
    /// App / channel scoped budgets and periodic budget resets (`5h`, `1d`, ...).
    /// Experimental.
    pub app_budgets: bool,
    /// Immutable agent versions, snapshots, forks, and app version binding.
    /// Experimental.
    pub agent_versions: bool,
    /// Realtime voice endpoints and microphone controls. Experimental.
    pub voice: bool,
    /// Outbound agent delegation capabilities (`a2a_agent_delegation`, `agent_handoff`).
    /// Experimental: auto-enabled in dev, off in prod by default.
    /// When off, these capabilities are not registered and cannot be assigned to agents.
    pub agent_delegation: bool,
    /// Observers (online scoring of production sessions). Experimental.
    pub observers: bool,
}

/// Metadata for an API-visible feature flag (org opt-in UI + catalog).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FeatureFlagDefinition {
    /// Stable flag key (matches `FeatureFlags` field / `is_enabled` name).
    pub name: &'static str,
    /// Human-readable title for settings UI.
    pub label: &'static str,
    /// Short description of what the flag gates.
    pub description: &'static str,
    /// When true, shown with experimental badges in the UI.
    pub experimental: bool,
}

/// All API-visible flags that organizations may opt into when the deployment allows them.
pub const API_FEATURE_FLAG_DEFINITIONS: &[FeatureFlagDefinition] = &[
    FeatureFlagDefinition {
        name: "global_chat",
        label: "Global chat",
        description: "Adds a personal chat session you can open from the sidebar anywhere in the \
             app. It's always available as a quick scratchpad to talk to your agents without \
             first setting up a dedicated app or channel.",
        experimental: true,
    },
    FeatureFlagDefinition {
        name: "notifications",
        label: "Notifications",
        description: "Turns on the in-app notification bell, toasts, and live updates. You get \
             alerted in real time when something you care about happens, instead of refreshing \
             or checking back manually.",
        experimental: true,
    },
    FeatureFlagDefinition {
        name: "mcp_endpoint",
        label: "MCP server endpoint",
        description: "Lets Everruns act as an MCP server so external tools and assistants can \
             connect to it. Other MCP-compatible clients can then reach your agents and data \
             through a standard endpoint.",
        experimental: true,
    },
    FeatureFlagDefinition {
        name: "evals",
        label: "Evals",
        description: "Lets you define and run behavioral evals against your agents. Use it to \
             confirm an agent responds the way you expect and to catch regressions as you change \
             prompts or models.",
        experimental: true,
    },
    FeatureFlagDefinition {
        name: "app_budgets",
        label: "App budgets",
        description: "Adds spending limits scoped to individual apps and channels, with automatic \
             resets on a schedule. It helps you cap and control costs so a single app or channel \
             can't run away with your usage.",
        experimental: true,
    },
    FeatureFlagDefinition {
        name: "agent_versions",
        label: "Agent versions",
        description: "Captures immutable snapshots of your agents so you can fork, roll back, and \
             pin apps to a specific version. This gives you a safety net to experiment freely \
             and return to a known-good agent at any time.",
        experimental: true,
    },
    FeatureFlagDefinition {
        name: "voice",
        label: "Voice",
        description: "Enables realtime voice in chat with microphone controls. You can talk to \
             your agents and hear responses instead of typing, for a hands-free conversation.",
        experimental: true,
    },
    FeatureFlagDefinition {
        name: "observers",
        label: "Observers",
        description: "Runs automatic online scoring on your production sessions. It continuously \
             evaluates live conversations so you can monitor quality on real traffic without \
             manually reviewing each one.",
        experimental: true,
    },
];

impl FeatureFlags {
    /// Effective flags for an organization: deployment/system gates AND explicit org opt-in.
    ///
    /// Org overrides default to disabled; a flag is on only when the deployment allows it
    /// and the org has opted in (`enabled: true` in storage).
    pub fn for_org(system: &Self, org_enabled: &std::collections::HashMap<String, bool>) -> Self {
        let opt_in = |name: &str, system_on: bool| -> bool {
            system_on && org_enabled.get(name).copied().unwrap_or(false)
        };
        Self {
            global_chat: opt_in("global_chat", system.global_chat),
            notifications: opt_in("notifications", system.notifications),
            mcp_endpoint: opt_in("mcp_endpoint", system.mcp_endpoint),
            evals: opt_in("evals", system.evals),
            app_budgets: opt_in("app_budgets", system.app_budgets),
            agent_versions: opt_in("agent_versions", system.agent_versions),
            voice: opt_in("voice", system.voice),
            agent_delegation: opt_in("agent_delegation", system.agent_delegation),
            observers: opt_in("observers", system.observers),
        }
    }

    /// Compute feature flags from environment variables and deployment grade.
    pub fn from_env(grade: &DeploymentGrade) -> Self {
        Self {
            global_chat: experimental_flag("FEATURE_GLOBAL_CHAT", grade),
            notifications: experimental_flag("FEATURE_NOTIFICATIONS", grade),
            mcp_endpoint: experimental_flag("FEATURE_MCP_ENDPOINT", grade),
            evals: experimental_flag("FEATURE_EVALS", grade),
            app_budgets: experimental_flag("FEATURE_APP_BUDGETS", grade),
            agent_versions: experimental_flag("FEATURE_AGENT_VERSIONS", grade),
            voice: experimental_flag("FEATURE_VOICE", grade),
            agent_delegation: experimental_flag("FEATURE_AGENT_DELEGATION", grade),
            observers: experimental_flag("FEATURE_OBSERVERS", grade),
        }
    }

    /// Resolve the current feature flags from env + the env-derived deployment grade.
    /// Convenience for callers that don't have a `FeatureFlags` instance handy.
    pub fn current() -> Self {
        Self::from_env(&DeploymentGrade::from_env())
    }

    /// Look up a flag by name (for dynamic/string-based access).
    pub fn is_enabled(&self, flag: &str) -> bool {
        match flag {
            "global_chat" => self.global_chat,
            "notifications" => self.notifications,
            "mcp_endpoint" => self.mcp_endpoint,
            "evals" => self.evals,
            "app_budgets" => self.app_budgets,
            "agent_versions" => self.agent_versions,
            "voice" => self.voice,
            "agent_delegation" => self.agent_delegation,
            "observers" => self.observers,
            _ => false,
        }
    }

    /// All flags enabled (for testing).
    #[cfg(test)]
    pub fn all_enabled() -> Self {
        Self {
            global_chat: true,
            notifications: true,
            mcp_endpoint: true,
            evals: true,
            app_budgets: true,
            agent_versions: true,
            voice: true,
            agent_delegation: true,
            observers: true,
        }
    }
}

/// Backend-only feature flags. Not exposed via API or frontend.
///
/// Used for internal gating (capability registration, infrastructure behavior).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct InternalFeatureFlags {
    /// Docker container capability. Disabled by default on all envs.
    /// Enable via `FEATURE_DOCKER_CAPABILITY=true`.
    pub docker_capability: bool,
    /// Self-hosted container sandbox capability and coding harness.
    /// Disabled by default on all envs.
    /// Enable via `FEATURE_CONTAINER_SANDBOX=true`, or via the legacy
    /// fallback `FEATURE_DOCKER_CAPABILITY=true` when
    /// `FEATURE_CONTAINER_SANDBOX` is unset.
    pub container_sandbox: bool,
    /// Managed session-owned sandbox capability and lifecycle orchestration.
    /// Experimental and disabled by default.
    pub session_sandbox: bool,
    /// Machine-payment capabilities (e.g. the Parallel paid search/extract/task
    /// capability). Gates registration of any capability that spends real money
    /// through `PaymentAuthority`. Disabled by default on all envs, including dev,
    /// because spend is irreversible. Enable via `FEATURE_MACHINE_PAYMENTS=true`.
    pub machine_payments: bool,
    /// Experimental sandboxed Lua execution capability (`specs/lua-execution.md`).
    /// Disabled by default; requires the `lua` cargo feature to be compiled in to
    /// actually run scripts. Enable via `FEATURE_LUA=true`.
    pub lua: bool,
}

impl InternalFeatureFlags {
    /// Compute internal feature flags from environment variables.
    pub fn from_env() -> Self {
        let docker_capability = standard_flag("FEATURE_DOCKER_CAPABILITY", false);

        Self {
            docker_capability,
            container_sandbox: standard_flag("FEATURE_CONTAINER_SANDBOX", docker_capability),
            session_sandbox: standard_flag("FEATURE_SESSION_SANDBOX", false),
            machine_payments: standard_flag("FEATURE_MACHINE_PAYMENTS", false),
            lua: standard_flag("FEATURE_LUA", false),
        }
    }

    /// Look up a flag by name (for dynamic/string-based access).
    pub fn is_enabled(&self, flag: &str) -> bool {
        match flag {
            "docker_capability" => self.docker_capability,
            "container_sandbox" => self.container_sandbox,
            "session_sandbox" => self.session_sandbox,
            "machine_payments" => self.machine_payments,
            "lua" => self.lua,
            _ => false,
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

    /// Restore an env var to a previously captured value, or remove it if it was unset.
    fn restore_env(key: &str, prev: Option<String>) {
        match prev {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        }
    }

    #[test]
    fn test_default_flags() {
        let flags = FeatureFlags::default();
        assert!(!flags.global_chat);
        assert!(!flags.notifications);
    }

    // SAFETY: env var tests must run single-threaded (--test-threads=1).
    // set_var/remove_var are unsafe in edition 2024 due to thread-safety.

    #[test]
    fn test_experimental_enabled_in_dev() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_GLOBAL_CHAT") };
        unsafe { std::env::remove_var("FEATURE_EVALS") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Dev);
        assert!(flags.global_chat);
        assert!(flags.evals);
    }

    #[test]
    fn test_experimental_disabled_in_prod() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_GLOBAL_CHAT") };
        unsafe { std::env::remove_var("FEATURE_EVALS") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(!flags.global_chat);
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
            notifications: true,
            mcp_endpoint: true,
            evals: true,
            app_budgets: true,
            agent_versions: true,
            voice: true,
            agent_delegation: true,
            observers: true,
        };
        assert!(flags.is_enabled("global_chat"));
        assert!(flags.is_enabled("notifications"));
        assert!(flags.is_enabled("mcp_endpoint"));
        assert!(flags.is_enabled("evals"));
        assert!(flags.is_enabled("app_budgets"));
        assert!(flags.is_enabled("agent_versions"));
        assert!(flags.is_enabled("voice"));
        assert!(flags.is_enabled("agent_delegation"));
        assert!(flags.is_enabled("observers"));
        assert!(!flags.is_enabled("nonexistent"));
    }

    #[test]
    fn test_serialization() {
        let flags = FeatureFlags {
            global_chat: true,
            notifications: true,
            mcp_endpoint: true,
            evals: true,
            app_budgets: true,
            agent_versions: true,
            voice: true,
            agent_delegation: true,
            observers: true,
        };
        let json = serde_json::to_string(&flags).unwrap();
        assert!(json.contains("\"global_chat\":true"));
        assert!(json.contains("\"notifications\":true"));
        assert!(json.contains("\"app_budgets\":true"));
        assert!(json.contains("\"agent_versions\":true"));
        assert!(json.contains("\"voice\":true"));
        assert!(json.contains("\"agent_delegation\":true"));
        assert!(json.contains("\"observers\":true"));

        let parsed: FeatureFlags = serde_json::from_str(&json).unwrap();
        assert_eq!(flags, parsed);
    }

    #[test]
    fn test_agent_delegation_enabled_in_dev() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_AGENT_DELEGATION") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Dev);
        assert!(flags.agent_delegation);
    }

    #[test]
    fn test_agent_delegation_disabled_in_prod() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_AGENT_DELEGATION") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(!flags.agent_delegation);
    }

    #[test]
    fn test_agent_delegation_env_override_in_prod() {
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_AGENT_DELEGATION", "true") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(flags.agent_delegation);
        unsafe { std::env::remove_var("FEATURE_AGENT_DELEGATION") };
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
    fn test_notifications_enabled_in_dev() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_NOTIFICATIONS") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Dev);
        assert!(flags.notifications);
    }

    #[test]
    fn test_notifications_disabled_in_prod() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_NOTIFICATIONS") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(!flags.notifications);
    }

    #[test]
    fn test_for_org_requires_system_and_opt_in() {
        let system = FeatureFlags {
            global_chat: true,
            evals: true,
            ..FeatureFlags::default()
        };
        let mut org = std::collections::HashMap::new();
        org.insert("global_chat".to_string(), true);
        let effective = FeatureFlags::for_org(&system, &org);
        assert!(effective.global_chat);
        assert!(!effective.evals);

        let effective_none = FeatureFlags::for_org(&system, &std::collections::HashMap::new());
        assert!(!effective_none.global_chat);
    }

    #[test]
    fn test_for_org_cannot_enable_when_system_off() {
        let system = FeatureFlags::default();
        let mut org = std::collections::HashMap::new();
        org.insert("global_chat".to_string(), true);
        let effective = FeatureFlags::for_org(&system, &org);
        assert!(!effective.global_chat);
    }

    #[test]
    fn test_notifications_respects_env_override() {
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_NOTIFICATIONS", "true") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(flags.notifications);
        unsafe { std::env::remove_var("FEATURE_NOTIFICATIONS") };
    }

    // =========================================================================
    // InternalFeatureFlags tests
    // =========================================================================

    #[test]
    fn test_internal_default_flags() {
        let flags = InternalFeatureFlags::default();
        assert!(!flags.docker_capability);
        assert!(!flags.container_sandbox);
        assert!(!flags.session_sandbox);
        assert!(!flags.machine_payments);
    }

    #[test]
    fn test_docker_capability_flag_disabled_by_default_in_dev() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_DOCKER_CAPABILITY") };
        let flags = InternalFeatureFlags::from_env();
        assert!(
            !flags.docker_capability,
            "docker_capability should be disabled by default even in dev"
        );
    }

    #[test]
    fn test_docker_capability_flag_enabled_by_env_override() {
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_DOCKER_CAPABILITY", "true") };
        let flags = InternalFeatureFlags::from_env();
        assert!(flags.docker_capability);
        unsafe { std::env::remove_var("FEATURE_DOCKER_CAPABILITY") };
    }

    #[test]
    fn test_container_sandbox_flag_enabled_by_env_override() {
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_CONTAINER_SANDBOX", "true") };
        unsafe { std::env::remove_var("FEATURE_DOCKER_CAPABILITY") };
        let flags = InternalFeatureFlags::from_env();
        assert!(flags.container_sandbox);
        unsafe { std::env::remove_var("FEATURE_CONTAINER_SANDBOX") };
    }

    #[test]
    fn test_container_sandbox_flag_falls_back_to_legacy_docker_flag() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_CONTAINER_SANDBOX") };
        unsafe { std::env::set_var("FEATURE_DOCKER_CAPABILITY", "true") };
        let flags = InternalFeatureFlags::from_env();
        assert!(flags.container_sandbox);
        unsafe { std::env::remove_var("FEATURE_DOCKER_CAPABILITY") };
    }

    #[test]
    fn test_internal_is_enabled_dynamic() {
        let flags = InternalFeatureFlags {
            docker_capability: true,
            container_sandbox: true,
            session_sandbox: true,
            machine_payments: true,
            lua: true,
        };
        assert!(flags.is_enabled("docker_capability"));
        assert!(flags.is_enabled("container_sandbox"));
        assert!(flags.is_enabled("session_sandbox"));
        assert!(flags.is_enabled("machine_payments"));
        assert!(flags.is_enabled("lua"));
        assert!(!flags.is_enabled("nonexistent"));
    }

    #[test]
    fn test_machine_payments_disabled_by_default() {
        let _lock = lock_env();
        let prev = std::env::var("FEATURE_MACHINE_PAYMENTS").ok();
        unsafe { std::env::remove_var("FEATURE_MACHINE_PAYMENTS") };
        let flags = InternalFeatureFlags::from_env();
        assert!(
            !flags.machine_payments,
            "machine_payments should be disabled by default on all envs"
        );
        restore_env("FEATURE_MACHINE_PAYMENTS", prev);
    }

    #[test]
    fn test_machine_payments_enabled_by_env_override() {
        let _lock = lock_env();
        let prev = std::env::var("FEATURE_MACHINE_PAYMENTS").ok();
        unsafe { std::env::set_var("FEATURE_MACHINE_PAYMENTS", "true") };
        let flags = InternalFeatureFlags::from_env();
        assert!(flags.machine_payments);
        restore_env("FEATURE_MACHINE_PAYMENTS", prev);
    }

    #[test]
    fn test_session_sandbox_flag_enabled_by_env_override() {
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_SESSION_SANDBOX", "true") };
        let flags = InternalFeatureFlags::from_env();
        assert!(flags.session_sandbox);
        unsafe { std::env::remove_var("FEATURE_SESSION_SANDBOX") };
    }
}

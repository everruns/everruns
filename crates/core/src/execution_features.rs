// Execution feature decisions (EVE-878).
//
// Decision: the org/product feature-flag records and management logic
// (`FeatureFlags`, `FeatureFlagMap`, `FeatureFlagDefinition`,
// `API_FEATURE_FLAG_DEFINITIONS`, org opt-in resolution) moved to the
// `everruns-platform` crate — they are hosted control-plane state resolved by
// the server before execution. Core retains only the narrowly required
// execution feature decisions consumed at capability-registration time:
// - `InternalFeatureFlags`: backend-only infrastructure gates computed from
//   env vars, never org-configurable and never exposed via API.
// - `ExecutionFeatureDecisions`: the resolved deployment-level snapshot the
//   registry builders consult; per-org effective decisions are applied at the
//   server loading seam (capability filtering before the worker snapshot), so
//   execution never loads feature-management records.
// Decision: Explicit env var (FEATURE_<NAME>=true/false) always takes priority.
// Decision: Flags marked "experimental" auto-enable in dev (DeploymentGrade::Dev).

use crate::deployment::DeploymentGrade;

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
    /// Experimental sandboxed Lua execution capability (`knowledge/execution/lua-execution.md`).
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
            lua: standard_flag("FEATURE_LUA", false),
        }
    }

    /// Look up a flag by name (for dynamic/string-based access).
    pub fn is_enabled(&self, flag: &str) -> bool {
        match flag {
            "docker_capability" => self.docker_capability,
            "container_sandbox" => self.container_sandbox,
            "session_sandbox" => self.session_sandbox,
            "lua" => self.lua,
            _ => false,
        }
    }
}

/// Resolved deployment-level execution feature decisions (EVE-878).
///
/// This is the snapshot the capability registry builders consult when
/// composing built-ins: internal infrastructure gates plus the few
/// experimental product gates that decide whether a capability is registered
/// at all. It is computed once from env vars and the deployment grade — it
/// never reads org feature-management records. Per-org effective decisions
/// are resolved by the platform/server before execution and applied by
/// filtering the capability list handed to the worker.
#[derive(Debug, Clone)]
pub struct ExecutionFeatureDecisions {
    /// Outbound agent delegation capabilities (`a2a_agent_delegation`,
    /// `agent_handoff`). Experimental: auto-enabled in dev, off in prod by
    /// default. When off, the capabilities are not registered at all.
    pub agent_delegation: bool,
    /// Backend-only infrastructure gates.
    pub internal: InternalFeatureFlags,
}

impl ExecutionFeatureDecisions {
    /// Resolve the deployment-level decisions from env vars and the grade.
    pub fn from_env(grade: DeploymentGrade) -> Self {
        Self {
            agent_delegation: experimental_flag("FEATURE_AGENT_DELEGATION", &grade),
            internal: InternalFeatureFlags::from_env(),
        }
    }

    /// Whether a registration-time feature gate is enabled.
    ///
    /// Internal infrastructure flags win; any other name resolves via the
    /// standard `FEATURE_<NAME>` env rule: enabled only by an explicit env
    /// var, never by the grade's experimental default. Registration gates
    /// control real side effects (e.g. the `machine_payments` gate on the
    /// payments capability, where spend is irreversible), so an unknown gate
    /// must fail closed even in dev — the pre-EVE-878 flag catalog classified
    /// `machine_payments` as standard/off, and this preserves that. Used by
    /// `IntegrationPlugin::feature_flag` gating.
    pub fn is_enabled(&self, flag: &str) -> bool {
        match flag {
            "docker_capability" | "container_sandbox" | "session_sandbox" | "lua" => {
                self.internal.is_enabled(flag)
            }
            "agent_delegation" => self.agent_delegation,
            _ => {
                let env_var = format!("FEATURE_{}", flag.to_ascii_uppercase());
                standard_flag(&env_var, false)
            }
        }
    }
}

/// Resolve an experimental flag.
///
/// Priority: explicit env var > experimental default (enabled in dev) > false.
pub fn experimental_flag(env_var: &str, grade: &DeploymentGrade) -> bool {
    if let Ok(val) = std::env::var(env_var) {
        return val == "true" || val == "1";
    }
    grade.experimental_features_enabled()
}

/// Resolve a standard (non-experimental) flag.
///
/// Priority: explicit env var > default.
pub fn standard_flag(env_var: &str, default: bool) -> bool {
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

    // SAFETY: env var tests must run single-threaded (--test-threads=1).
    // set_var/remove_var are unsafe in edition 2024 due to thread-safety.

    #[test]
    fn test_internal_default_flags() {
        let flags = InternalFeatureFlags::default();
        assert!(!flags.docker_capability);
        assert!(!flags.container_sandbox);
        assert!(!flags.session_sandbox);
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
            lua: true,
        };
        assert!(flags.is_enabled("docker_capability"));
        assert!(flags.is_enabled("container_sandbox"));
        assert!(flags.is_enabled("session_sandbox"));
        assert!(flags.is_enabled("lua"));
        assert!(!flags.is_enabled("nonexistent"));
    }

    #[test]
    fn test_session_sandbox_flag_enabled_by_env_override() {
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_SESSION_SANDBOX", "true") };
        let flags = InternalFeatureFlags::from_env();
        assert!(flags.session_sandbox);
        unsafe { std::env::remove_var("FEATURE_SESSION_SANDBOX") };
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
    fn test_agent_delegation_enabled_in_dev() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_AGENT_DELEGATION") };
        let decisions = ExecutionFeatureDecisions::from_env(DeploymentGrade::Dev);
        assert!(decisions.agent_delegation);
        assert!(decisions.is_enabled("agent_delegation"));
    }

    #[test]
    fn test_agent_delegation_disabled_in_prod() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_AGENT_DELEGATION") };
        let decisions = ExecutionFeatureDecisions::from_env(DeploymentGrade::Prod);
        assert!(!decisions.agent_delegation);
        assert!(!decisions.is_enabled("agent_delegation"));
    }

    #[test]
    fn test_agent_delegation_env_override_in_prod() {
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_AGENT_DELEGATION", "true") };
        let decisions = ExecutionFeatureDecisions::from_env(DeploymentGrade::Prod);
        assert!(decisions.agent_delegation);
        unsafe { std::env::remove_var("FEATURE_AGENT_DELEGATION") };
    }

    #[test]
    fn test_decisions_route_internal_flags_to_internal_set() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_CONTAINER_SANDBOX") };
        unsafe { std::env::remove_var("FEATURE_DOCKER_CAPABILITY") };
        // Internal flags never inherit the experimental dev default: even in
        // dev grade, `container_sandbox` stays off without an explicit env var.
        let decisions = ExecutionFeatureDecisions::from_env(DeploymentGrade::Dev);
        assert!(!decisions.is_enabled("container_sandbox"));
        assert!(!decisions.is_enabled("docker_capability"));
        assert!(!decisions.is_enabled("lua"));
    }

    #[test]
    fn test_decisions_resolve_unknown_gates_via_standard_env_rule() {
        let _lock = lock_env();
        // Registration gates fail closed without an explicit env var, in every
        // grade — a payments-style gate must not auto-enable in dev.
        unsafe { std::env::remove_var("FEATURE_MACHINE_PAYMENTS") };
        let dev = ExecutionFeatureDecisions::from_env(DeploymentGrade::Dev);
        assert!(!dev.is_enabled("machine_payments"));
        let prod = ExecutionFeatureDecisions::from_env(DeploymentGrade::Prod);
        assert!(!prod.is_enabled("machine_payments"));

        unsafe { std::env::set_var("FEATURE_MACHINE_PAYMENTS", "true") };
        let dev = ExecutionFeatureDecisions::from_env(DeploymentGrade::Dev);
        assert!(dev.is_enabled("machine_payments"));
        let prod = ExecutionFeatureDecisions::from_env(DeploymentGrade::Prod);
        assert!(prod.is_enabled("machine_payments"));
        unsafe { std::env::remove_var("FEATURE_MACHINE_PAYMENTS") };
    }
}

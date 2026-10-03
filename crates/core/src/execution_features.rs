//! Deployment-level capability registration decisions. Org policy is enforced
//! by the server before a capability list reaches the worker.

use crate::{DeploymentGrade, FeatureFlagGrade};

// Registration defaults are shared with the hosted catalog: promoting a default
// must change API availability and actual registry composition together.
/// Default rollout grade for agent delegation.
pub const AGENT_DELEGATION_DEFAULT_GRADE: FeatureFlagGrade = FeatureFlagGrade::Dev;
/// Default rollout grade for Docker execution.
pub const DOCKER_CAPABILITY_DEFAULT_GRADE: FeatureFlagGrade = FeatureFlagGrade::Off;
/// Default rollout grade for container sandboxes.
pub const CONTAINER_SANDBOX_DEFAULT_GRADE: FeatureFlagGrade = FeatureFlagGrade::Off;
/// Default rollout grade for Lua execution.
pub const LUA_DEFAULT_GRADE: FeatureFlagGrade = FeatureFlagGrade::Off;
/// Default rollout grade for machine payments.
pub const MACHINE_PAYMENTS_DEFAULT_GRADE: FeatureFlagGrade = FeatureFlagGrade::Off;

/// Deployment availability for optional execution infrastructure.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct InternalFeatureFlags {
    /// Docker execution is available for registration.
    pub docker_capability: bool,
    /// Container sandbox execution is available for registration.
    pub container_sandbox: bool,
    /// Lua execution is available for registration.
    pub lua: bool,
}

impl InternalFeatureFlags {
    /// Resolve infrastructure availability for the environment deployment.
    pub fn from_env() -> Self {
        Self::for_deployment(DeploymentGrade::from_env())
    }

    /// Resolve infrastructure availability for the requested deployment.
    pub fn for_deployment(deployment: DeploymentGrade) -> Self {
        let enabled =
            |name, default| FeatureFlagGrade::from_env(name, default).available(deployment);
        Self {
            docker_capability: enabled(
                "FEATURE_DOCKER_CAPABILITY",
                DOCKER_CAPABILITY_DEFAULT_GRADE,
            ),
            container_sandbox: enabled(
                "FEATURE_CONTAINER_SANDBOX",
                CONTAINER_SANDBOX_DEFAULT_GRADE,
            ),
            lua: enabled("FEATURE_LUA", LUA_DEFAULT_GRADE),
        }
    }

    /// Whether the named infrastructure gate permits registration.
    pub fn is_enabled(&self, flag: &str) -> bool {
        match flag {
            "docker_capability" => self.docker_capability,
            "container_sandbox" => self.container_sandbox,
            "lua" => self.lua,
            _ => false,
        }
    }
}

/// Deployment registration decisions before hosted organisation policy is applied.
#[derive(Debug, Clone)]
pub struct ExecutionFeatureDecisions {
    /// Agent delegation is available for registration.
    pub agent_delegation: bool,
    /// Optional infrastructure registration gates.
    pub internal: InternalFeatureFlags,
    deployment: DeploymentGrade,
}

impl ExecutionFeatureDecisions {
    /// Resolve feature availability for the requested deployment.
    pub fn from_env(deployment: DeploymentGrade) -> Self {
        Self {
            agent_delegation: FeatureFlagGrade::from_env(
                "FEATURE_AGENT_DELEGATION",
                AGENT_DELEGATION_DEFAULT_GRADE,
            )
            .available(deployment),
            internal: InternalFeatureFlags::for_deployment(deployment),
            deployment,
        }
    }

    /// Whether a named gate permits capability registration.
    pub fn is_enabled(&self, flag: &str) -> bool {
        match flag {
            "docker_capability" | "container_sandbox" | "lua" => self.internal.is_enabled(flag),
            "agent_delegation" => self.agent_delegation,
            _ => FeatureFlagGrade::from_env(
                &format!("FEATURE_{}", flag.to_ascii_uppercase()),
                if flag == "machine_payments" {
                    MACHINE_PAYMENTS_DEFAULT_GRADE
                } else {
                    FeatureFlagGrade::Off
                },
            )
            .available(self.deployment),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grade_overrides_apply_to_capability_registration() {
        const CHILD: &str = "EVERRUNS_FEATURE_GRADE_TEST_CHILD";
        const KEYS: &[&str] = &[
            "FEATURE_AGENT_DELEGATION",
            "FEATURE_DOCKER_CAPABILITY",
            "FEATURE_CONTAINER_SANDBOX",
            "FEATURE_LUA",
            "FEATURE_MACHINE_PAYMENTS",
            "FEATURE_UNKNOWN",
        ];
        let cases = [
            (None, [true, false, false, false]),
            (Some("dev"), [true, false, false, false]),
            (Some("preview"), [true; 4]),
            (Some("adoption"), [true; 4]),
            (Some("prod"), [true; 4]),
            (Some("off"), [false; 4]),
            (Some("true"), [false; 4]),
            (Some("invalid"), [false; 4]),
        ];
        if let Ok(index) = std::env::var(CHILD) {
            let (value, expected) = cases[index.parse::<usize>().unwrap()];
            for (deployment, available) in [
                DeploymentGrade::Dev,
                DeploymentGrade::Poc,
                DeploymentGrade::Preview,
                DeploymentGrade::Prod,
            ]
            .into_iter()
            .zip(expected)
            {
                let decisions = ExecutionFeatureDecisions::from_env(deployment);
                assert_eq!(decisions.agent_delegation, available);
                for name in [
                    "docker_capability",
                    "container_sandbox",
                    "lua",
                    "machine_payments",
                    "unknown",
                ] {
                    let expected = value.is_some() && available;
                    assert_eq!(
                        decisions.is_enabled(name),
                        expected,
                        "{name}, {deployment}, {value:?}"
                    );
                }
            }
            return;
        }
        for (index, (value, _)) in cases.iter().enumerate() {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "execution_features::tests::grade_overrides_apply_to_capability_registration",
                    "--nocapture",
                ])
                .env(CHILD, index.to_string());
            for key in KEYS {
                command.env_remove(key);
                if let Some(value) = value {
                    command.env(key, value);
                }
            }
            assert!(command.status().unwrap().success(), "{value:?}");
        }
    }
}

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

    #[test]
    fn internal_lookup_reads_each_flag_independently() {
        for values in [
            [false, false, false, false],
            [true, false, false, false],
            [false, true, false, false],
            [false, false, true, false],
            [false, false, false, true],
        ] {
            let flags = InternalFeatureFlags {
                docker_capability: values[0],
                container_sandbox: values[1],
                session_sandbox: values[2],
                lua: values[3],
            };
            for (name, expected) in [
                "docker_capability",
                "container_sandbox",
                "session_sandbox",
                "lua",
            ]
            .into_iter()
            .zip(values)
            {
                assert_eq!(flags.is_enabled(name), expected, "{name}, {values:?}");
            }
            for unknown in ["nonexistent", "Docker_capability", ""] {
                assert!(!flags.is_enabled(unknown));
            }
        }
    }

    #[test]
    fn environment_overrides_and_grade_defaults_are_isolated() {
        const CHILD: &str = "EVERRUNS_EXECUTION_FEATURE_REVIEW_CASE";
        const KEYS: &[&str] = &[
            "FEATURE_DOCKER_CAPABILITY",
            "FEATURE_CONTAINER_SANDBOX",
            "FEATURE_SESSION_SANDBOX",
            "FEATURE_LUA",
            "FEATURE_AGENT_DELEGATION",
            "FEATURE_MACHINE_PAYMENTS",
            "DEPLOYMENT_GRADE",
            "DEV_MODE",
            "FEATURE_REVIEW_MISSING",
        ];
        struct Case {
            name: &'static str,
            env: &'static [(&'static str, &'static str)],
            internal: [bool; 4],
            delegation: [bool; 4],
            grade: DeploymentGrade,
            payments: bool,
        }
        let cases = [
            Case {
                name: "unset",
                env: &[],
                internal: [false, false, false, false],
                delegation: [true, false, false, false],
                grade: DeploymentGrade::Prod,
                payments: false,
            },
            Case {
                name: "legacy docker and dev mode one",
                env: &[("FEATURE_DOCKER_CAPABILITY", "true"), ("DEV_MODE", "1")],
                internal: [true, true, false, false],
                delegation: [true, false, false, false],
                grade: DeploymentGrade::Dev,
                payments: false,
            },
            Case {
                name: "explicit container and preview grade",
                env: &[
                    ("FEATURE_CONTAINER_SANDBOX", "1"),
                    ("DEPLOYMENT_GRADE", "staging"),
                    ("DEV_MODE", "true"),
                ],
                internal: [false, true, false, false],
                delegation: [true, false, false, false],
                grade: DeploymentGrade::Preview,
                payments: false,
            },
            Case {
                name: "explicit container false overrides legacy",
                env: &[
                    ("FEATURE_DOCKER_CAPABILITY", "true"),
                    ("FEATURE_CONTAINER_SANDBOX", "false"),
                    ("DEPLOYMENT_GRADE", "PoC"),
                ],
                internal: [true, false, false, false],
                delegation: [true, false, false, false],
                grade: DeploymentGrade::Poc,
                payments: false,
            },
            Case {
                name: "all enabled with explicit production",
                env: &[
                    ("FEATURE_DOCKER_CAPABILITY", "1"),
                    ("FEATURE_CONTAINER_SANDBOX", "true"),
                    ("FEATURE_SESSION_SANDBOX", "true"),
                    ("FEATURE_LUA", "1"),
                    ("FEATURE_AGENT_DELEGATION", "true"),
                    ("FEATURE_MACHINE_PAYMENTS", "1"),
                    ("DEPLOYMENT_GRADE", "production"),
                    ("DEV_MODE", "true"),
                ],
                internal: [true, true, true, true],
                delegation: [true, true, true, true],
                grade: DeploymentGrade::Prod,
                payments: true,
            },
            Case {
                name: "explicit false overrides dev defaults",
                env: &[
                    ("FEATURE_DOCKER_CAPABILITY", "false"),
                    ("FEATURE_CONTAINER_SANDBOX", "0"),
                    ("FEATURE_SESSION_SANDBOX", "false"),
                    ("FEATURE_LUA", "0"),
                    ("FEATURE_AGENT_DELEGATION", "false"),
                    ("FEATURE_MACHINE_PAYMENTS", "false"),
                    ("DEV_MODE", "true"),
                ],
                internal: [false, false, false, false],
                delegation: [false, false, false, false],
                grade: DeploymentGrade::Dev,
                payments: false,
            },
            Case {
                name: "invalid explicit values fail closed",
                env: &[
                    ("FEATURE_DOCKER_CAPABILITY", "true"),
                    ("FEATURE_CONTAINER_SANDBOX", "TRUE"),
                    ("FEATURE_SESSION_SANDBOX", "typo"),
                    ("FEATURE_LUA", "TRUE"),
                    ("FEATURE_AGENT_DELEGATION", "TRUE"),
                    ("FEATURE_MACHINE_PAYMENTS", "yes"),
                    ("DEPLOYMENT_GRADE", "invalid"),
                    ("DEV_MODE", "true"),
                ],
                internal: [true, false, false, false],
                delegation: [false, false, false, false],
                grade: DeploymentGrade::Prod,
                payments: false,
            },
            Case {
                name: "empty explicit grade suppresses legacy dev",
                env: &[("DEPLOYMENT_GRADE", ""), ("DEV_MODE", "true")],
                internal: [false, false, false, false],
                delegation: [true, false, false, false],
                grade: DeploymentGrade::Prod,
                payments: false,
            },
        ];
        if let Ok(index) = std::env::var(CHILD) {
            let index: usize = index.parse().unwrap();
            let case = &cases[index];
            let expected_internal = InternalFeatureFlags {
                docker_capability: case.internal[0],
                container_sandbox: case.internal[1],
                session_sandbox: case.internal[2],
                lua: case.internal[3],
            };
            assert_eq!(
                InternalFeatureFlags::from_env(),
                expected_internal,
                "{}",
                case.name
            );
            assert_eq!(DeploymentGrade::from_env(), case.grade, "{}", case.name);
            for (grade, expected_delegation) in [
                DeploymentGrade::Dev,
                DeploymentGrade::Poc,
                DeploymentGrade::Preview,
                DeploymentGrade::Prod,
            ]
            .into_iter()
            .zip(case.delegation)
            {
                let decisions = ExecutionFeatureDecisions::from_env(grade);
                assert_eq!(
                    decisions.internal, expected_internal,
                    "{}, {grade}",
                    case.name
                );
                assert_eq!(
                    decisions.agent_delegation, expected_delegation,
                    "{}, {grade}",
                    case.name
                );
                assert_eq!(
                    decisions.is_enabled("agent_delegation"),
                    expected_delegation
                );
                for (name, expected) in [
                    "docker_capability",
                    "container_sandbox",
                    "session_sandbox",
                    "lua",
                ]
                .into_iter()
                .zip(case.internal)
                {
                    assert_eq!(decisions.is_enabled(name), expected, "{name}");
                }
                assert_eq!(decisions.is_enabled("machine_payments"), case.payments);
                assert_eq!(decisions.is_enabled("MACHINE_PAYMENTS"), case.payments);
                assert!(!decisions.is_enabled("review_missing"));
            }
            // Known flags come from the resolved snapshot, even if the current
            // environment says true. Unknown registration gates use the env rule.
            let captured = ExecutionFeatureDecisions {
                agent_delegation: false,
                internal: InternalFeatureFlags::default(),
            };
            for name in [
                "docker_capability",
                "container_sandbox",
                "session_sandbox",
                "lua",
                "agent_delegation",
            ] {
                assert!(!captured.is_enabled(name), "captured {name}");
            }
            assert_eq!(captured.is_enabled("machine_payments"), case.payments);
            assert!(!standard_flag("FEATURE_REVIEW_MISSING", false));
            assert!(standard_flag("FEATURE_REVIEW_MISSING", true));
            println!("feature fixture {index} completed");
            return;
        }
        for (index, case) in cases.iter().enumerate() {
            // A module-local mutex cannot protect process-wide environment reads
            // by other tests. Set variables before each child starts instead.
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command.args([
                "--exact",
                concat!(
                    module_path!(),
                    "::environment_overrides_and_grade_defaults_are_isolated"
                )
                .strip_prefix("everruns_core::")
                .unwrap(),
                "--nocapture",
            ]);
            for key in KEYS {
                command.env_remove(key);
            }
            let output = command
                .envs(case.env.iter().copied())
                .env(CHILD, index.to_string())
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success(),
                "{} failed:\n{stdout}\n{}",
                case.name,
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                stdout.contains(&format!("feature fixture {index} completed")),
                "{} did not run assertions:\n{stdout}",
                case.name
            );
        }
    }
}

// Rollout grades determine availability, defaults, and the owner of org overrides.
// Runtime consumers receive only the resolved booleans, never management records.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use everruns_core::deployment::DeploymentGrade;
use everruns_core::execution_features::{CONTAINER_SANDBOX_DEFAULT_GRADE, LUA_DEFAULT_GRADE};
pub use everruns_core::{FeatureFlagDefinition, FeatureFlagGrade};

/// Feature flags exposed via `GET /v1/platform/feature-flags` and consumed by the frontend.
///
/// Resolved booleans derived from rollout grades and organisation overrides.
///
/// Decision: this is the type-safe representation used throughout the backend.
/// The API does not serialize it directly — it exposes the untyped
/// [`FeatureFlagMap`] instead, so adding/removing a flag never changes
/// `docs/api/openapi.json`.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeatureFlags {
    /// Evals (user-facing behavioral evals for agents). Organization adoption opt-in.
    pub evals: bool,
    /// Native typed extraction API and console. Organization adoption opt-in.
    pub extractions: bool,
    /// Skills registry management UI. Experimental.
    pub skills: bool,
    /// Workspace memory management UI. Experimental.
    pub memory: bool,
    /// Voice channels and the chat microphone (Adoption).
    pub voice: bool,
    /// Observers (online scoring of production sessions). Organization adoption opt-in.
    pub observers: bool,
    /// Public Chat (isolated, public-facing chat web app + `public_chat`
    /// channel). Experimental. Gates the public endpoints, channel creation,
    /// the builder UI, and the public web route.
    pub public_chat: bool,
    /// Run the org's sessions on the Engine (data plane v2 step 4).
    /// Internal: the platform enrols an org; no org manages it. The worker
    /// lease reads the enrolment row itself (`ENGINE_HOSTING_FLAG`).
    #[serde(default)]
    pub engine_hosting: bool,
    #[serde(default)]
    pub container_sandbox: bool,
    #[serde(default)]
    pub lua: bool,
    /// Flags declared by integration crates (see
    /// [`everruns_capabilities::integrations_catalog::feature_flag_definitions`]), keyed by
    /// flag name. Serialized flat, next to the platform flags.
    #[serde(flatten, default)]
    pub integrations: BTreeMap<String, bool>,
}

/// Untyped API representation of feature flags: a generic `{ "<flag>": bool }` map.
///
/// Decision: the public API is intentionally untyped. The set of flags churns
/// frequently; encoding each flag as a named schema property would force a
/// `docs/api/openapi.json` change on every add/remove. A generic string→bool map
/// keeps the API spec stable. The frontend layers its own typed view on top.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(transparent)]
pub struct FeatureFlagMap(pub BTreeMap<String, bool>);

impl From<&FeatureFlags> for FeatureFlagMap {
    fn from(flags: &FeatureFlags) -> Self {
        flags.to_map()
    }
}

impl From<FeatureFlags> for FeatureFlagMap {
    fn from(flags: FeatureFlags) -> Self {
        flags.to_map()
    }
}

/// Every hosted feature flag: the platform catalog below, then the flags
/// integration crates declare for their capabilities and connectors.
pub fn feature_flag_definitions() -> impl Iterator<Item = &'static FeatureFlagDefinition> {
    API_FEATURE_FLAG_DEFINITIONS
        .iter()
        .chain(everruns_capabilities::integrations_catalog::feature_flag_definitions())
}

/// Whether `name` is a known hosted feature flag.
pub fn is_known_feature_flag(name: &str) -> bool {
    feature_flag_definitions().any(|definition| definition.name == name)
}

/// One catalog for platform flags, including infrastructure capabilities.
/// Integration flags live with their integration crates.
pub const API_FEATURE_FLAG_DEFINITIONS: &[FeatureFlagDefinition] = &[
    FeatureFlagDefinition {
        name: "evals",
        label: "Evals",
        description: "Lets you define and run behavioral evals against your agents. Use it to \
             confirm an agent responds the way you expect and to catch regressions as you change \
             prompts or models.",
        grade: FeatureFlagGrade::Adoption,
    },
    FeatureFlagDefinition {
        name: "extractions",
        label: "Extractions",
        description: "Extract typed values and source evidence with an organization-owned provider account.",
        grade: FeatureFlagGrade::Adoption,
    },
    FeatureFlagDefinition {
        name: "skills",
        label: "Skills",
        description: "Create and manage reusable instruction packages that teach agents \
             specialized workflows.",
        grade: FeatureFlagGrade::Dev,
    },
    FeatureFlagDefinition {
        name: "memory",
        label: "Memory",
        description: "Manage knowledge stores agents can read, including manual notes and \
             synchronized files.",
        grade: FeatureFlagGrade::Dev,
    },
    FeatureFlagDefinition {
        name: "voice",
        label: "Voice",
        description: "Adds voice channels and the chat microphone. Callers talk to your agents \
             and hear answers spoken back while the agent keeps its own model, tools and \
             instructions. Needs an OpenAI provider for speech.",
        grade: FeatureFlagGrade::Adoption,
    },
    FeatureFlagDefinition {
        name: "observers",
        label: "Observers",
        description: "Runs automatic online scoring on your production sessions. It continuously \
             evaluates live conversations so you can monitor quality on real traffic without \
             manually reviewing each one.",
        grade: FeatureFlagGrade::Adoption,
    },
    FeatureFlagDefinition {
        name: "public_chat",
        label: "Public Chat",
        description: "Isolated, public-facing chat web app and the public_chat channel.",
        grade: FeatureFlagGrade::Dev,
    },
    FeatureFlagDefinition {
        name: "engine_hosting",
        label: "Engine hosting",
        description: "Runs the organization's sessions on the Everruns Engine. Platform rollout \
             switch; existing sessions move over between turns.",
        grade: FeatureFlagGrade::Internal,
    },
    FeatureFlagDefinition {
        name: "container_sandbox",
        label: "Container sandbox",
        description: "Run the coding harness in a self-hosted container sandbox.",
        grade: CONTAINER_SANDBOX_DEFAULT_GRADE,
    },
    FeatureFlagDefinition {
        name: "lua",
        label: "Lua execution",
        description: "Run sandboxed Lua tools and code mode.",
        grade: LUA_DEFAULT_GRADE,
    },
];

/// Startup snapshot of rollout grades. Deployment availability and org-effective
/// booleans are derived from this policy; flags are never promoted by an org row.
#[derive(Debug, Clone)]
pub struct FeatureFlagPolicy {
    pub deployment: DeploymentGrade,
    grades: BTreeMap<String, FeatureFlagGrade>,
}

impl FeatureFlagPolicy {
    pub fn from_env(deployment: DeploymentGrade) -> Self {
        let grades = feature_flag_definitions()
            .map(|definition| (definition.name.to_string(), definition.grade_from_env()))
            .collect();
        Self { deployment, grades }
    }

    pub fn current() -> Self {
        Self::from_env(DeploymentGrade::from_env())
    }

    pub fn grade(&self, name: &str) -> FeatureFlagGrade {
        self.grades
            .get(name)
            .copied()
            .unwrap_or(FeatureFlagGrade::Off)
    }

    pub fn with_grade(mut self, name: &str, grade: FeatureFlagGrade) -> Self {
        assert!(is_known_feature_flag(name), "unknown feature flag: {name}");
        self.grades.insert(name.to_string(), grade);
        self
    }

    /// Available means registration/routes may exist; it does not grant an org access.
    pub fn deployment_flags(&self) -> FeatureFlags {
        self.resolve(|name| self.grade(name).available(self.deployment))
    }

    pub fn for_org(&self, overrides: &std::collections::HashMap<String, bool>) -> FeatureFlags {
        self.resolve(|name| {
            self.grade(name)
                .effective(self.deployment, overrides.get(name).copied())
        })
    }

    fn resolve(&self, enabled: impl Fn(&str) -> bool) -> FeatureFlags {
        let mut flags = FeatureFlags::default();
        for definition in feature_flag_definitions() {
            flags.set(definition.name, enabled(definition.name));
        }
        flags
    }
}

impl FeatureFlags {
    pub fn from_env(deployment: &DeploymentGrade) -> Self {
        FeatureFlagPolicy::from_env(*deployment).deployment_flags()
    }

    /// Resolve the current feature flags from env + the env-derived deployment grade.
    /// Convenience for callers that don't have a `FeatureFlags` instance handy.
    pub fn current() -> Self {
        Self::from_env(&DeploymentGrade::from_env())
    }

    /// Generic `name -> enabled` map for the untyped API representation.
    ///
    /// Keys and values match the JSON wire format of the typed `FeatureFlags`
    /// response body. The JSON content is equivalent; only the key order differs
    /// (`BTreeMap` sorts keys, whereas the struct serializes in field order), which
    /// is irrelevant to JSON consumers.
    pub fn to_map(&self) -> FeatureFlagMap {
        let mut map = BTreeMap::from([
            ("container_sandbox".to_string(), self.container_sandbox),
            ("lua".to_string(), self.lua),
            ("evals".to_string(), self.evals),
            ("extractions".to_string(), self.extractions),
            ("skills".to_string(), self.skills),
            ("memory".to_string(), self.memory),
            ("voice".to_string(), self.voice),
            ("observers".to_string(), self.observers),
            ("public_chat".to_string(), self.public_chat),
            ("engine_hosting".to_string(), self.engine_hosting),
        ]);
        map.extend(
            self.integrations
                .iter()
                .map(|(name, enabled)| (name.clone(), *enabled)),
        );
        FeatureFlagMap(map)
    }

    /// Look up a flag by name (for dynamic/string-based access).
    pub fn is_enabled(&self, flag: &str) -> bool {
        match flag {
            "container_sandbox" => self.container_sandbox,
            "lua" => self.lua,
            "evals" => self.evals,
            "extractions" => self.extractions,
            "skills" => self.skills,
            "memory" => self.memory,
            "voice" => self.voice,
            "observers" => self.observers,
            "public_chat" => self.public_chat,
            "engine_hosting" => self.engine_hosting,
            _ => self.integrations.get(flag).copied().unwrap_or(false),
        }
    }

    fn set(&mut self, name: &str, enabled: bool) {
        match name {
            "evals" => self.evals = enabled,
            "extractions" => self.extractions = enabled,
            "skills" => self.skills = enabled,
            "memory" => self.memory = enabled,
            "voice" => self.voice = enabled,
            "observers" => self.observers = enabled,
            "public_chat" => self.public_chat = enabled,
            "engine_hosting" => self.engine_hosting = enabled,
            "container_sandbox" => self.container_sandbox = enabled,
            "lua" => self.lua = enabled,
            _ => {
                assert!(
                    everruns_capabilities::integrations_catalog::feature_flag_definitions()
                        .any(|definition| definition.name == name),
                    "catalog and boolean fields must agree: {name}"
                );
                self.integrations.insert(name.to_string(), enabled);
            }
        }
    }

    /// Whether a capability is available under these effective flags.
    pub fn is_capability_enabled(&self, capability_id: &str) -> bool {
        Self::required_for_capability(capability_id).is_none_or(|flag| self.is_enabled(flag))
    }

    /// Feature flag required by a capability, when one exists.
    pub fn required_for_capability(capability_id: &str) -> Option<&'static str> {
        match capability_id {
            "container_sandbox" => Some("container_sandbox"),
            "lua" | "lua_code_mode" => Some("lua"),
            "skills" => Some("skills"),
            "memory" => Some("memory"),
            _ if capability_id.starts_with("skill:") => Some("skills"),
            _ => {
                everruns_capabilities::integrations_catalog::capability_feature_flag(capability_id)
            }
        }
    }

    /// Whether a connection provider is offered under these effective flags.
    pub fn is_connector_enabled(&self, provider_id: &str) -> bool {
        everruns_capabilities::integrations_catalog::connector_feature_flag(provider_id)
            .is_none_or(|flag| self.is_enabled(flag))
    }

    /// All flags enabled (for testing).
    #[cfg(test)]
    pub fn all_enabled() -> Self {
        Self {
            container_sandbox: true,
            lua: true,
            evals: true,
            extractions: true,
            skills: true,
            memory: true,
            voice: true,
            observers: true,
            public_chat: true,
            engine_hosting: true,
            integrations: everruns_capabilities::integrations_catalog::feature_flag_definitions()
                .map(|definition| (definition.name.to_string(), true))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn policy(deployment: DeploymentGrade, grade: FeatureFlagGrade) -> FeatureFlagPolicy {
        FeatureFlagPolicy {
            deployment,
            grades: feature_flag_definitions()
                .map(|def| (def.name.to_string(), grade))
                .collect(),
        }
    }

    #[test]
    fn retired_flags_are_ignored_even_with_explicit_org_opt_outs() {
        let retired = ["dreaming", "monitoring", "knowledge"];
        let overrides = HashMap::from(retired.map(|name| (name.to_string(), false)));
        let policy = policy(DeploymentGrade::Prod, FeatureFlagGrade::Off);
        let flags = policy.for_org(&overrides);
        for name in retired {
            assert!(!is_known_feature_flag(name));
            assert!(!policy.deployment_flags().to_map().0.contains_key(name));
            assert!(!flags.to_map().0.contains_key(name));
        }
        for capability in ["dreaming", "knowledge", "knowledge_base", "knowledge_index"] {
            assert!(flags.is_capability_enabled(capability), "{capability}");
        }
    }

    #[test]
    fn environment_override_replaces_catalog_grade() {
        const CHILD: &str = "EVERRUNS_PLATFORM_FLAG_GRADE_TEST";
        let cases = [
            None,
            Some("dev"),
            Some("internal"),
            Some("preview"),
            Some("adoption"),
            Some("prod"),
            Some("off"),
            Some("true"),
            Some("invalid"),
        ];
        if let Ok(index) = std::env::var(CHILD) {
            let value = cases[index.parse::<usize>().unwrap()];
            let expected = value
                .map(|value| value.parse().unwrap_or(FeatureFlagGrade::Off))
                .unwrap_or(FeatureFlagGrade::Adoption);
            let policy = FeatureFlagPolicy::from_env(DeploymentGrade::Prod);
            assert_eq!(policy.grade("evals"), expected);
            assert_eq!(policy.grade("observers"), expected);
            assert_eq!(
                policy.deployment_flags().observers,
                expected.available(DeploymentGrade::Prod)
            );
            assert_eq!(
                policy.for_org(&HashMap::new()).observers,
                expected.effective(DeploymentGrade::Prod, None)
            );
            assert_eq!(
                policy.deployment_flags().evals,
                expected.available(DeploymentGrade::Prod)
            );
            assert_eq!(
                policy.for_org(&HashMap::new()).evals,
                expected.effective(DeploymentGrade::Prod, None)
            );
            return;
        }
        for (index, value) in cases.into_iter().enumerate() {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "records::feature_flags::tests::environment_override_replaces_catalog_grade",
                    "--nocapture",
                ])
                .env(CHILD, index.to_string())
                .env_remove("FEATURE_EVALS")
                .env_remove("FEATURE_OBSERVERS");
            if let Some(value) = value {
                command
                    .env("FEATURE_EVALS", value)
                    .env("FEATURE_OBSERVERS", value);
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{value:?}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "child must exercise the policy"
            );
        }
    }

    #[test]
    fn hosted_catalog_preserves_opt_in_and_platform_authority() {
        let policy = FeatureFlagPolicy {
            deployment: DeploymentGrade::Prod,
            grades: feature_flag_definitions()
                .map(|def| (def.name.to_string(), def.grade))
                .collect(),
        };
        let adoption_flags = ["evals", "extractions", "observers", "voice"];
        let opted_in = adoption_flags
            .iter()
            .map(|name| (name.to_string(), true))
            .collect();
        for name in adoption_flags {
            assert_eq!(policy.grade(name), FeatureFlagGrade::Adoption, "{name}");
            assert!(policy.deployment_flags().is_enabled(name), "{name}");
            assert!(!policy.for_org(&HashMap::new()).is_enabled(name), "{name}");
            assert!(policy.for_org(&opted_in).is_enabled(name), "{name}");
        }
        for removed in [
            "agent_views",
            "agent_delegation",
            "webmcp",
            "typesafe",
            "openai_agents_api",
            "browserless",
            "modal",
            "sprites",
            "mcp_events",
            "agent_change_reasons_required",
            "reports",
            "mistral",
            // Plugins are in Settings for everyone; approval is the gate
            // (knowledge/integrations/plugins.md).
            "plugins",
            "machine_payments",
            "docker_capability",
        ] {
            assert!(!is_known_feature_flag(removed), "{removed}");
        }
        assert!(!FeatureFlagGrade::Internal.org_configurable(DeploymentGrade::Prod));
        for name in ["skills", "memory", "public_chat"] {
            assert_eq!(policy.grade(name), FeatureFlagGrade::Dev, "{name}");
            assert!(!policy.deployment_flags().is_enabled(name), "{name}");
        }
        for name in ["container_sandbox", "lua"] {
            assert_eq!(policy.grade(name), FeatureFlagGrade::Off, "{name}");
            assert!(
                !policy
                    .for_org(&HashMap::from([(name.to_string(), true)]))
                    .is_enabled(name),
                "{name}"
            );
        }
    }

    #[test]
    fn every_flag_obeys_the_same_grade_policy() {
        for deployment in [DeploymentGrade::Dev, DeploymentGrade::Prod] {
            for grade in [
                FeatureFlagGrade::Dev,
                FeatureFlagGrade::Internal,
                FeatureFlagGrade::Adoption,
                FeatureFlagGrade::Prod,
                FeatureFlagGrade::Off,
            ] {
                let policy = policy(deployment, grade);
                for override_value in [None, Some(true), Some(false)] {
                    let overrides = feature_flag_definitions()
                        .filter_map(|def| override_value.map(|value| (def.name.to_string(), value)))
                        .collect();
                    let flags = policy.for_org(&overrides);
                    for definition in feature_flag_definitions() {
                        assert_eq!(
                            flags.is_enabled(definition.name),
                            grade.effective(deployment, override_value),
                            "{}, {grade}, {deployment}, {override_value:?}",
                            definition.name
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn catalog_and_api_boolean_map_have_exactly_the_same_keys() {
        let flags = FeatureFlags::all_enabled();
        let map = flags.to_map();
        assert_eq!(map.0.len(), feature_flag_definitions().count());
        assert_eq!(
            serde_json::to_value(&flags).unwrap(),
            serde_json::to_value(&map).unwrap()
        );
        for definition in feature_flag_definitions() {
            assert!(flags.is_enabled(definition.name));
            assert!(map.0[definition.name]);
        }
        assert!(!flags.is_enabled("unknown"));
    }

    #[test]
    fn disabled_flags_remove_all_related_runtime_capabilities() {
        let disabled = FeatureFlags::default();
        let enabled = FeatureFlags::all_enabled();
        for capability in [
            "skills",
            "skill:example",
            "memory",
            "container_sandbox",
            "lua",
            "lua_code_mode",
        ] {
            assert!(!disabled.is_capability_enabled(capability), "{capability}");
            assert!(enabled.is_capability_enabled(capability), "{capability}");
        }
        assert!(disabled.is_capability_enabled("unrelated"));
        // Adding the capability to an agent is the opt-in; no flag on top.
        for capability in [
            "spawn_agents",
            "threads",
            "coordinator",
            "a2a_agent_delegation",
            "ag_ui_delegation",
        ] {
            assert!(disabled.is_capability_enabled(capability), "{capability}");
        }
    }

    #[test]
    fn retired_integration_overrides_do_not_disable_built_in_connections() {
        let policy = FeatureFlagPolicy {
            deployment: DeploymentGrade::Prod,
            grades: feature_flag_definitions()
                .map(|def| (def.name.to_string(), def.grade))
                .collect(),
        };
        let retired = [
            "browserless",
            "modal",
            "sprites",
            "mcp_events",
            "agent_change_reasons_required",
        ];
        let overrides = retired
            .iter()
            .map(|name| (name.to_string(), false))
            .collect();
        let flags = policy.for_org(&overrides);
        let map = flags.to_map();
        for name in retired {
            assert!(!map.0.contains_key(name), "{name}");
            assert!(!is_known_feature_flag(name), "{name}");
        }
        for name in ["browserless", "modal"] {
            assert!(flags.is_connector_enabled(name), "{name}");
        }
        for name in ["browserless", "computer_use", "browser_use", "modal"] {
            assert!(flags.is_capability_enabled(name), "{name}");
        }
    }

    #[test]
    fn integration_flags_gate_their_capabilities_and_connectors() {
        let disabled = FeatureFlags::default();
        let enabled = FeatureFlags::all_enabled();
        for capability in ["modal", "browserless", "computer_use", "browser_use"] {
            assert!(disabled.is_capability_enabled(capability), "{capability}");
            assert!(enabled.is_capability_enabled(capability), "{capability}");
        }
        // Integrations an agent only reaches after someone adds the capability
        // or a connection carry no feature flag.
        // Web search providers ship without flags: adding the capability or
        // connecting an account is the opt-in.
        for capability in [
            "resource_discovery",
            "shell_code_mode",
            "agentid",
            "jev",
            "brave_search",
            "duckduckgo",
            "parallel_search",
        ] {
            assert!(disabled.is_capability_enabled(capability), "{capability}");
        }
        assert!(disabled.is_connector_enabled("ard"));
        assert!(disabled.is_connector_enabled("parallel"));
        assert!(disabled.is_connector_enabled("brave_search"));
        for connector in ["browserless", "modal"] {
            assert!(disabled.is_connector_enabled(connector), "{connector}");
            assert!(enabled.is_connector_enabled(connector), "{connector}");
        }
        // Daytona and E2B capabilities and their credential connectors are
        // generally available.
        assert!(disabled.is_capability_enabled("daytona"));
        assert!(disabled.is_capability_enabled("e2b"));
        assert!(disabled.is_connector_enabled("daytona"));
        assert!(enabled.is_connector_enabled("daytona"));
        assert!(disabled.is_connector_enabled("e2b"));
    }
}

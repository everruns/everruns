// Rollout grades determine availability, defaults, and the owner of org overrides.
// Runtime consumers receive only the resolved booleans, never management records.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use everruns_core::deployment::DeploymentGrade;
use everruns_core::execution_features::{
    AGENT_DELEGATION_DEFAULT_GRADE, CONTAINER_SANDBOX_DEFAULT_GRADE,
    DOCKER_CAPABILITY_DEFAULT_GRADE, LUA_DEFAULT_GRADE, MACHINE_PAYMENTS_DEFAULT_GRADE,
};
pub use everruns_core::{FeatureFlagDefinition, FeatureFlagGrade};

/// Feature flags exposed via `GET /v1/feature-flags` and consumed by the frontend.
///
/// Resolved booleans derived from rollout grades and organisation overrides.
///
/// Decision: this is the type-safe representation used throughout the backend.
/// The API does not serialize it directly — it exposes the untyped
/// [`FeatureFlagMap`] instead, so adding/removing a flag never changes
/// `docs/api/openapi.json`.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeatureFlags {
    /// In-app notifications (bell, toasts, notification SSE). Experimental.
    pub notifications: bool,
    /// Evals (user-facing behavioral evals for agents). Experimental.
    pub evals: bool,
    /// Skills registry management UI. Experimental.
    pub skills: bool,
    /// Workspace memory management UI. Experimental.
    pub memory: bool,
    /// Knowledge index management UI. Experimental.
    pub knowledge: bool,
    /// Plugin marketplace and installed-plugin management UI. Experimental.
    pub plugins: bool,
    /// Agent / channel scoped budgets and periodic budget resets (`5h`, `1d`, ...).
    /// Experimental.
    pub channel_budgets: bool,
    /// Immutable agent versions, snapshots, forks, and channel version binding.
    /// Experimental.
    pub agent_versions: bool,
    /// Realtime voice endpoints and microphone controls. Experimental.
    pub voice: bool,
    /// Outbound agent delegation capabilities (`a2a_agent_delegation`,
    /// `ag_ui_delegation`, `agent_handoff`). Available for org opt-in on every deployment.
    /// Deployment availability controls registration; org policy controls use.
    pub agent_delegation: bool,
    /// Observers (online scoring of production sessions). Experimental.
    pub observers: bool,
    /// Public Chat (isolated, public-facing chat web app + `public_chat`
    /// channel). Experimental. Gates the public endpoints, channel creation,
    /// the builder UI, and the public web route.
    pub public_chat: bool,
    /// Browser-native WebMCP tools exposed by the authenticated Everruns UI.
    /// Experimental remote-control surface; requires deployment enablement and
    /// per-org opt-in.
    pub webmcp: bool,
    /// Outbound MCP Events: `/mcp` clients subscribe to session completion,
    /// failure and input-required webhooks. Experimental and org-opt-in,
    /// because the spec is a draft.
    pub mcp_events: bool,
    /// Reports: the usage and cost reporting page, its sidebar entry, and
    /// saved-report search results in the UI. Experimental and org-opt-in, so
    /// it is off for every organization until an admin turns it on. The
    /// reporting API and background aggregation are not gated.
    pub reports: bool,
    /// Machine-payment custody, policy, audit, and paid capability surfaces.
    pub machine_payments: bool,
    /// OpenAI Agents API runtime backend; internal enrolment is platform-owned.
    #[serde(default)]
    pub openai_agents_api: bool,
    #[serde(default)]
    pub docker_capability: bool,
    #[serde(default)]
    pub container_sandbox: bool,
    #[serde(default)]
    pub lua: bool,
    /// Personal ChatGPT plan connections; deployment availability and org enrolment required.
    #[serde(default)]
    pub chatgpt_plan: bool,
    /// Platform Chat workspace with an integrated Threads panel. Org opt-in.
    #[serde(default)]
    pub chat_threads: bool,
    /// Flags declared by integration crates (see
    /// [`everruns_integrations_catalog::feature_flag_definitions`]), keyed by
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
        .chain(everruns_integrations_catalog::feature_flag_definitions())
}

/// Whether `name` is a known hosted feature flag.
pub fn is_known_feature_flag(name: &str) -> bool {
    feature_flag_definitions().any(|definition| definition.name == name)
}

/// One catalog for platform flags, including infrastructure capabilities.
/// Integration flags live with their integration crates.
pub const API_FEATURE_FLAG_DEFINITIONS: &[FeatureFlagDefinition] = &[
    FeatureFlagDefinition {
        name: "chat_threads",
        label: "Chat threads",
        description: "Manage conversations and ongoing work in a Threads panel alongside permanent Chat.",
        grade: FeatureFlagGrade::Adoption,
    },
    FeatureFlagDefinition {
        name: "chatgpt_plan",
        label: "ChatGPT plan connections",
        description: "Connect your personal ChatGPT plan for private agent conversations.",
        grade: FeatureFlagGrade::Off,
    },
    FeatureFlagDefinition {
        name: "notifications",
        label: "Notifications",
        description: "Turns on the in-app notification bell, toasts, and live updates. You get \
             alerted in real time when something you care about happens, instead of refreshing \
             or checking back manually.",
        grade: FeatureFlagGrade::Adoption,
    },
    FeatureFlagDefinition {
        name: "evals",
        label: "Evals",
        description: "Lets you define and run behavioral evals against your agents. Use it to \
             confirm an agent responds the way you expect and to catch regressions as you change \
             prompts or models.",
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
        name: "knowledge",
        label: "Knowledge indexes",
        description: "Connect external document collections and make them searchable by agents.",
        grade: FeatureFlagGrade::Dev,
    },
    FeatureFlagDefinition {
        name: "plugins",
        label: "Plugins",
        description: "Install and manage extensions that add integrations, skills, and other \
             capabilities.",
        grade: FeatureFlagGrade::Dev,
    },
    FeatureFlagDefinition {
        name: "channel_budgets",
        label: "Channel budgets",
        description: "Adds spending limits scoped to individual agents and channels, with \
             automatic resets on a schedule. It helps you cap and control costs so a single agent \
             or channel can't run away with your usage.",
        grade: FeatureFlagGrade::Adoption,
    },
    FeatureFlagDefinition {
        name: "agent_versions",
        label: "Agent versions",
        description: "Captures immutable snapshots of your agents so you can fork, roll back, and \
             pin channels to a specific version. This gives you a safety net to experiment freely \
             and return to a known-good agent at any time.",
        grade: FeatureFlagGrade::Adoption,
    },
    FeatureFlagDefinition {
        name: "voice",
        label: "Voice",
        description: "Enables realtime voice in chat with microphone controls. You can talk to \
             your agents and hear responses instead of typing, for a hands-free conversation.",
        grade: FeatureFlagGrade::Dev,
    },
    FeatureFlagDefinition {
        name: "agent_delegation",
        label: "Agent delegation",
        description: "Enables outbound agent delegation capabilities, including agent handoffs \
             and A2A delegation. When disabled, these capabilities are hidden and cannot be \
             assigned.",
        grade: AGENT_DELEGATION_DEFAULT_GRADE,
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
        name: "webmcp",
        label: "WebMCP UI tools",
        description: "Exposes a small browser-native tool surface from the authenticated UI so a \
             browser agent can search, navigate, and perform confirmed actions in Everruns.",
        grade: FeatureFlagGrade::Adoption,
    },
    FeatureFlagDefinition {
        name: "mcp_events",
        label: "MCP Events",
        description: "Lets MCP clients such as ChatGPT subscribe to webhooks when a session \
             completes, fails, or needs your input.",
        grade: FeatureFlagGrade::Dev,
    },
    FeatureFlagDefinition {
        name: "reports",
        label: "Reports",
        description: "Shows the Reports page for exploring usage and cost across sessions, agents, \
             and models, and for saving and exporting report queries.",
        grade: FeatureFlagGrade::Dev,
    },
    FeatureFlagDefinition {
        name: "machine_payments",
        label: "Machine payments",
        description: "Manage payment accounts, spending policies, and paid capabilities.",
        grade: MACHINE_PAYMENTS_DEFAULT_GRADE,
    },
    FeatureFlagDefinition {
        name: "openai_agents_api",
        label: "OpenAI Agents API runtime",
        description: "Allows agents with the OpenAI Agents API runtime capability to run their loop through OpenAI's Agents API instead of the native Everruns loop.",
        grade: FeatureFlagGrade::Internal,
    },
    FeatureFlagDefinition {
        name: "docker_capability",
        label: "Docker containers",
        description: "Run agent tools in Docker containers.",
        grade: DOCKER_CAPABILITY_DEFAULT_GRADE,
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
            ("docker_capability".to_string(), self.docker_capability),
            ("container_sandbox".to_string(), self.container_sandbox),
            ("lua".to_string(), self.lua),
            ("chatgpt_plan".to_string(), self.chatgpt_plan),
            ("chat_threads".to_string(), self.chat_threads),
            ("notifications".to_string(), self.notifications),
            ("evals".to_string(), self.evals),
            ("skills".to_string(), self.skills),
            ("memory".to_string(), self.memory),
            ("knowledge".to_string(), self.knowledge),
            ("plugins".to_string(), self.plugins),
            ("channel_budgets".to_string(), self.channel_budgets),
            ("agent_versions".to_string(), self.agent_versions),
            ("voice".to_string(), self.voice),
            ("agent_delegation".to_string(), self.agent_delegation),
            ("observers".to_string(), self.observers),
            ("public_chat".to_string(), self.public_chat),
            ("webmcp".to_string(), self.webmcp),
            ("mcp_events".to_string(), self.mcp_events),
            ("reports".to_string(), self.reports),
            ("machine_payments".to_string(), self.machine_payments),
            ("openai_agents_api".to_string(), self.openai_agents_api),
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
            "docker_capability" => self.docker_capability,
            "container_sandbox" => self.container_sandbox,
            "lua" => self.lua,
            "chatgpt_plan" => self.chatgpt_plan,
            "chat_threads" => self.chat_threads,
            "notifications" => self.notifications,
            "evals" => self.evals,
            "skills" => self.skills,
            "memory" => self.memory,
            "knowledge" => self.knowledge,
            "plugins" => self.plugins,
            "channel_budgets" => self.channel_budgets,
            "agent_versions" => self.agent_versions,
            "voice" => self.voice,
            "agent_delegation" => self.agent_delegation,
            "observers" => self.observers,
            "public_chat" => self.public_chat,
            "webmcp" => self.webmcp,
            "mcp_events" => self.mcp_events,
            "reports" => self.reports,
            "machine_payments" => self.machine_payments,
            "openai_agents_api" => self.openai_agents_api,
            _ => self.integrations.get(flag).copied().unwrap_or(false),
        }
    }

    fn set(&mut self, name: &str, enabled: bool) {
        match name {
            "notifications" => self.notifications = enabled,
            "evals" => self.evals = enabled,
            "skills" => self.skills = enabled,
            "memory" => self.memory = enabled,
            "knowledge" => self.knowledge = enabled,
            "plugins" => self.plugins = enabled,
            "channel_budgets" => self.channel_budgets = enabled,
            "agent_versions" => self.agent_versions = enabled,
            "voice" => self.voice = enabled,
            "agent_delegation" => self.agent_delegation = enabled,
            "observers" => self.observers = enabled,
            "public_chat" => self.public_chat = enabled,
            "webmcp" => self.webmcp = enabled,
            "mcp_events" => self.mcp_events = enabled,
            "reports" => self.reports = enabled,
            "machine_payments" => self.machine_payments = enabled,
            "openai_agents_api" => self.openai_agents_api = enabled,
            "docker_capability" => self.docker_capability = enabled,
            "container_sandbox" => self.container_sandbox = enabled,
            "lua" => self.lua = enabled,
            "chatgpt_plan" => self.chatgpt_plan = enabled,
            "chat_threads" => self.chat_threads = enabled,
            _ => {
                assert!(
                    everruns_integrations_catalog::feature_flag_definitions()
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
            "docker_container" => Some("docker_capability"),
            "container_sandbox" => Some("container_sandbox"),
            "lua" | "lua_code_mode" => Some("lua"),
            "parallel" => Some("machine_payments"),
            "skills" => Some("skills"),
            "memory" => Some("memory"),
            "knowledge_index" | "knowledge_base" => Some("knowledge"),
            "a2a_agent_delegation" | "ag_ui_delegation" | "agent_handoff" => {
                Some("agent_delegation")
            }
            everruns_core::capabilities::OPENAI_AGENTS_API_RUNTIME_ID => Some("openai_agents_api"),
            _ if capability_id.starts_with("skill:") => Some("skills"),
            _ if capability_id.starts_with("plugin:") => Some("plugins"),
            _ => everruns_integrations_catalog::capability_feature_flag(capability_id),
        }
    }

    /// Whether a connection provider is offered under these effective flags.
    pub fn is_connector_enabled(&self, provider_id: &str) -> bool {
        everruns_integrations_catalog::connector_feature_flag(provider_id)
            .is_none_or(|flag| self.is_enabled(flag))
    }

    /// All flags enabled (for testing).
    #[cfg(test)]
    pub fn all_enabled() -> Self {
        Self {
            docker_capability: true,
            container_sandbox: true,
            lua: true,
            chatgpt_plan: true,
            chat_threads: true,
            notifications: true,
            evals: true,
            skills: true,
            memory: true,
            knowledge: true,
            plugins: true,
            channel_budgets: true,
            agent_versions: true,
            voice: true,
            agent_delegation: true,
            observers: true,
            public_chat: true,
            webmcp: true,
            mcp_events: true,
            reports: true,
            machine_payments: true,
            openai_agents_api: true,
            integrations: everruns_integrations_catalog::feature_flag_definitions()
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
    fn chat_threads_require_org_adoption_on_every_deployment() {
        let definition = API_FEATURE_FLAG_DEFINITIONS
            .iter()
            .find(|definition| definition.name == "chat_threads")
            .unwrap();
        assert_eq!(definition.grade, FeatureFlagGrade::Adoption);
        let enrolled = HashMap::from([("chat_threads".to_string(), true)]);
        for deployment in [
            DeploymentGrade::Dev,
            DeploymentGrade::Preview,
            DeploymentGrade::Prod,
        ] {
            let available = policy(deployment, definition.grade);
            assert!(available.deployment_flags().chat_threads);
            assert!(!available.for_org(&HashMap::new()).chat_threads);
            assert!(available.for_org(&enrolled).chat_threads);
            assert!(
                !available
                    .for_org(&HashMap::from([("chat_threads".to_string(), false)]))
                    .chat_threads
            );
            assert!(
                !available
                    .with_grade("chat_threads", FeatureFlagGrade::Off)
                    .for_org(&enrolled)
                    .chat_threads
            );
        }
    }

    #[test]
    fn chatgpt_connections_require_deployment_and_org_enrolment() {
        let off = policy(DeploymentGrade::Prod, FeatureFlagGrade::Off);
        let enrolled = HashMap::from([("chatgpt_plan".into(), true)]);
        assert!(!off.for_org(&enrolled).chatgpt_plan);
        let available = off.with_grade("chatgpt_plan", FeatureFlagGrade::Adoption);
        assert!(available.deployment_flags().chatgpt_plan);
        assert!(!available.for_org(&HashMap::new()).chatgpt_plan);
        assert!(available.for_org(&enrolled).chatgpt_plan);
        assert!(
            !available
                .for_org(&HashMap::from([("chatgpt_plan".into(), false)]))
                .chatgpt_plan
        );
    }

    #[test]
    fn playground_is_not_a_feature_flag() {
        assert!(
            API_FEATURE_FLAG_DEFINITIONS
                .iter()
                .all(|flag| flag.name != "playground")
        );
        assert!(
            !FeatureFlagPolicy::from_env(DeploymentGrade::Dev)
                .deployment_flags()
                .to_map()
                .0
                .contains_key("playground")
        );
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
                    "feature_flags::tests::environment_override_replaces_catalog_grade",
                    "--nocapture",
                ])
                .env(CHILD, index.to_string())
                .env_remove("FEATURE_EVALS");
            if let Some(value) = value {
                command.env("FEATURE_EVALS", value);
            }
            assert!(command.status().unwrap().success(), "{value:?}");
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
        let adoption_flags = [
            "chat_threads",
            "notifications",
            "evals",
            "channel_budgets",
            "agent_versions",
            "agent_delegation",
            "observers",
            "webmcp",
        ];
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
        assert_eq!(
            policy.grade("openai_agents_api"),
            FeatureFlagGrade::Internal
        );
        assert!(
            !policy
                .grade("openai_agents_api")
                .org_configurable(DeploymentGrade::Prod)
        );
        assert!(!policy.for_org(&HashMap::new()).openai_agents_api);
        for name in [
            "skills",
            "memory",
            "knowledge",
            "plugins",
            "voice",
            "public_chat",
            "mcp_events",
            "reports",
        ] {
            assert_eq!(policy.grade(name), FeatureFlagGrade::Dev, "{name}");
            assert!(!policy.deployment_flags().is_enabled(name), "{name}");
        }
        for name in [
            "machine_payments",
            "docker_capability",
            "container_sandbox",
            "lua",
        ] {
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
    fn registration_availability_does_not_enroll_an_org() {
        let policy = policy(DeploymentGrade::Prod, FeatureFlagGrade::Internal);
        assert!(policy.deployment_flags().openai_agents_api);
        assert!(!policy.for_org(&HashMap::new()).openai_agents_api);
        assert!(
            policy
                .for_org(&HashMap::from([("openai_agents_api".into(), true)]))
                .openai_agents_api
        );
    }

    #[test]
    fn production_defaults_can_be_disabled_and_off_ignores_stale_enrollment() {
        let policy = policy(DeploymentGrade::Prod, FeatureFlagGrade::Prod);
        assert!(policy.for_org(&HashMap::new()).evals);
        let overrides = HashMap::from([("evals".into(), false), ("skills".into(), true)]);
        assert!(!policy.for_org(&overrides).evals);
        assert!(
            !policy
                .with_grade("skills", FeatureFlagGrade::Off)
                .for_org(&overrides)
                .skills
        );
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
            "knowledge_base",
            "knowledge_index",
            "plugin:example",
            "docker_container",
            "container_sandbox",
            "lua",
            "lua_code_mode",
            "parallel",
            "agent_handoff",
            "a2a_agent_delegation",
            "ag_ui_delegation",
            "openai_agents_api_runtime",
        ] {
            assert!(!disabled.is_capability_enabled(capability), "{capability}");
            assert!(enabled.is_capability_enabled(capability), "{capability}");
        }
        assert!(disabled.is_capability_enabled("unrelated"));
    }

    #[test]
    fn integration_flags_gate_their_capabilities_and_connectors() {
        let disabled = FeatureFlags::default();
        let enabled = FeatureFlags::all_enabled();
        for capability in ["modal", "brave_search", "jev", "resource_discovery"] {
            assert!(!disabled.is_capability_enabled(capability), "{capability}");
            assert!(enabled.is_capability_enabled(capability), "{capability}");
        }
        // Daytona and E2B capabilities are generally available; Daytona's
        // connection is behind its own flag.
        assert!(disabled.is_capability_enabled("daytona"));
        assert!(disabled.is_capability_enabled("e2b"));
        assert!(!disabled.is_connector_enabled("daytona"));
        assert!(enabled.is_connector_enabled("daytona"));
        assert!(disabled.is_connector_enabled("e2b"));
    }

    #[test]
    fn integration_flags_follow_the_same_grade_policy() {
        let prod = FeatureFlagPolicy {
            deployment: DeploymentGrade::Prod,
            grades: feature_flag_definitions()
                .map(|def| (def.name.to_string(), def.grade))
                .collect(),
        };
        assert_eq!(prod.grade("daytona_connection"), FeatureFlagGrade::Dev);
        assert!(!prod.deployment_flags().is_enabled("daytona_connection"));
        let adopted = prod.with_grade("daytona_connection", FeatureFlagGrade::Adoption);
        assert!(adopted.deployment_flags().is_enabled("daytona_connection"));
        assert!(
            !adopted
                .for_org(&HashMap::new())
                .is_enabled("daytona_connection")
        );
        assert!(
            adopted
                .for_org(&HashMap::from([("daytona_connection".into(), true)]))
                .is_connector_enabled("daytona")
        );
        let map = adopted.deployment_flags().to_map();
        assert_eq!(map.0.get("daytona_connection"), Some(&true));
    }
}

// Feature flags system (EVE-878: moved from `everruns-core`).
//
// Decision: org/product feature flags are hosted control-plane management
// state: the server computes system-level flags from env vars + deployment
// grade, layers org opt-in records on top (`org_feature_flags` storage), and
// exposes the result via `GET /v1/feature-flags`. None of that participates
// in a turn, so the records and management logic live here. The execution
// kernel keeps only `everruns_core::execution_features` — the resolved
// registration-time decisions — and receives per-org effective decisions as
// an already-filtered capability list at the server loading seam.
//
// Decision: Feature flags are system-level, computed from env vars + deployment grade.
// Decision: Flags marked "experimental" auto-enable in dev (DeploymentGrade::Dev).
// Decision: Explicit env var (FEATURE_<NAME>=true/false) always takes priority.
// Decision: Struct-based for type safety; `is_enabled(&str)` for dynamic lookup.
// Decision: Future extensibility: per-org/per-user flags, external providers (LaunchDarkly).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use everruns_core::deployment::DeploymentGrade;
use everruns_core::execution_features::{experimental_flag, standard_flag};

/// Feature flags exposed via `GET /v1/feature-flags` and consumed by the frontend.
///
/// Currently backed by environment variables and deployment grade.
/// Future: per-org flags, per-user flags, external providers.
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
    /// When off, these capabilities are not exposed or assignable; deployment-level
    /// disablement also prevents their registration.
    pub agent_delegation: bool,
    /// Observers (online scoring of production sessions). Experimental.
    pub observers: bool,
    /// Public Chat (isolated, public-facing chat web app + `public_chat`
    /// channel). Experimental. Gates the public endpoints, channel creation,
    /// the builder UI, and the public web route. See `knowledge/integrations/public-chat.md`.
    pub public_chat: bool,
    /// Browser-native WebMCP tools exposed by the authenticated Everruns UI.
    /// Experimental remote-control surface; requires deployment enablement and
    /// per-org opt-in. See `knowledge/ui/webmcp.md`.
    pub webmcp: bool,
    /// Session environments: where a session's commands run and what they may
    /// touch. Deployment-controlled and not org-configurable, like
    /// `machine_payments`: it describes the sandbox surface, so turning it on is
    /// a platform decision rather than a per-org preference. Defaults to on
    /// wherever sandboxes are already enabled.
    /// See `knowledge/harnesses/execution-environments.md`.
    pub environments: bool,
    /// Platform Chat v2: the operator chat surface as one shell. Platform
    /// operations are the `everruns` command beside `cat`, `grep` and `jq`
    /// rather than three bespoke tools, over a session filesystem carrying the
    /// product docs and shared operator memory. Experimental and org-opt-in:
    /// it is a different way to do what v1 already does, so an org chooses it
    /// rather than being enrolled. v1 keeps the chat role either way.
    /// See `knowledge/harnesses/platform-chat-v2.md`.
    pub platform_chat_v2: bool,
    /// Machine-payment custody, policy, audit, and paid capability surfaces.
    /// Deployment-controlled and off by default on every grade because spend is
    /// irreversible. Unlike experimental flags, this is not org-configurable.
    pub machine_payments: bool,
    /// Projects: a nested work scope inside an organization (Direction F). The
    /// project switcher takes the sidebar's top slot and the organization moves
    /// into the user menu; resources resolve to the caller's active project.
    /// Experimental and org-opt-in. When off, every resource stays in the org's
    /// seeded default project and the project UI is hidden, so the column
    /// backfill and default-project seeding make turning it on non-breaking.
    pub projects: bool,
}

/// Untyped API representation of feature flags: a generic `{ "<flag>": bool }` map.
///
/// Decision: the public API is intentionally untyped. The set of flags churns
/// frequently; encoding each flag as a named schema property would force a
/// `docs/api/openapi.json` change on every add/remove. A generic string→bool map
/// keeps the API spec stable. The frontend layers its own typed view on top.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
    /// When true, only a platform user may turn this flag on for an
    /// organization. The flag is still org-scoped, so it can be enabled for one
    /// tenant and not another; what changes is who decides. Used for surfaces
    /// whose cost or risk is the platform's rather than the tenant's.
    pub platform_managed: bool,
}

/// Whether this deployment runs sandboxes at all.
///
/// Sandbox enablement is an internal, env-controlled decision
/// (`crates/core/src/execution_features.rs`); the environments surface follows
/// it so an operator who turned sandboxes on does not have to find a second
/// switch to see what those sandboxes can do.
fn sandboxes_enabled() -> bool {
    let internal = everruns_core::InternalFeatureFlags::from_env();
    internal.session_sandbox || internal.container_sandbox
}

/// API-visible flags that organizations may opt into when the deployment allows them.
/// Deployment-only UI gates such as `machine_payments` intentionally stay out of this catalog.
/// Entries marked `platform_managed` are in it, because they are org-scoped; what they withhold
/// is the tenant's ability to turn them on for themselves.
pub const API_FEATURE_FLAG_DEFINITIONS: &[FeatureFlagDefinition] = &[
    FeatureFlagDefinition {
        name: "notifications",
        label: "Notifications",
        description: "Turns on the in-app notification bell, toasts, and live updates. You get \
             alerted in real time when something you care about happens, instead of refreshing \
             or checking back manually.",
        experimental: true,
        platform_managed: false,
    },
    FeatureFlagDefinition {
        name: "evals",
        label: "Evals",
        description: "Lets you define and run behavioral evals against your agents. Use it to \
             confirm an agent responds the way you expect and to catch regressions as you change \
             prompts or models.",
        experimental: true,
        platform_managed: false,
    },
    FeatureFlagDefinition {
        name: "skills",
        label: "Skills",
        description: "Create and manage reusable instruction packages that teach agents \
             specialized workflows.",
        experimental: true,
        platform_managed: false,
    },
    FeatureFlagDefinition {
        name: "memory",
        label: "Memory",
        description: "Manage knowledge stores agents can read, including manual notes and \
             synchronized files.",
        experimental: true,
        platform_managed: false,
    },
    FeatureFlagDefinition {
        name: "knowledge",
        label: "Knowledge indexes",
        description: "Connect external document collections and make them searchable by agents.",
        experimental: true,
        platform_managed: false,
    },
    FeatureFlagDefinition {
        name: "plugins",
        label: "Plugins",
        description: "Install and manage extensions that add integrations, skills, and other \
             capabilities.",
        experimental: true,
        platform_managed: false,
    },
    FeatureFlagDefinition {
        name: "app_budgets",
        label: "App budgets",
        description: "Adds spending limits scoped to individual apps and channels, with automatic \
             resets on a schedule. It helps you cap and control costs so a single app or channel \
             can't run away with your usage.",
        experimental: true,
        platform_managed: false,
    },
    FeatureFlagDefinition {
        name: "agent_versions",
        label: "Agent versions",
        description: "Captures immutable snapshots of your agents so you can fork, roll back, and \
             pin apps to a specific version. This gives you a safety net to experiment freely \
             and return to a known-good agent at any time.",
        experimental: true,
        platform_managed: false,
    },
    FeatureFlagDefinition {
        name: "voice",
        label: "Voice",
        description: "Enables realtime voice in chat with microphone controls. You can talk to \
             your agents and hear responses instead of typing, for a hands-free conversation.",
        experimental: true,
        platform_managed: false,
    },
    FeatureFlagDefinition {
        name: "agent_delegation",
        label: "Agent delegation",
        description: "Enables outbound agent delegation capabilities, including agent handoffs \
             and A2A delegation. When disabled, these capabilities are hidden and cannot be \
             assigned.",
        experimental: true,
        platform_managed: false,
    },
    FeatureFlagDefinition {
        name: "platform_chat_v2",
        label: "Platform Chat v2",
        description: "Runs Platform Chat as one shell: platform operations are the `everruns` \
             command alongside ordinary shell tools, over a filesystem holding the product \
             documentation and notes that outlive a conversation. The current Platform Chat is \
             unchanged and stays the default.",
        experimental: true,
        // An org opts itself in: this is a different way to do what the current
        // surface already does, not a deployment capability an operator runs.
        platform_managed: false,
    },
    FeatureFlagDefinition {
        name: "observers",
        label: "Observers",
        description: "Runs automatic online scoring on your production sessions. It continuously \
             evaluates live conversations so you can monitor quality on real traffic without \
             manually reviewing each one.",
        experimental: true,
        platform_managed: false,
    },
    FeatureFlagDefinition {
        name: "environments",
        label: "Environments",
        description: "Shows where each session's commands run and what that environment can \
             actually do, including whether it can run native processes and whether its files \
             can be recovered.",
        experimental: true,
        // Platform-managed: this describes the sandbox surface an operator runs
        // and pays for, so an org is enrolled in it rather than opting itself in.
        platform_managed: true,
    },
    FeatureFlagDefinition {
        name: "public_chat",
        label: "Public Chat",
        description: "Isolated, public-facing chat web app and the public_chat channel.",
        experimental: true,
        platform_managed: false,
    },
    FeatureFlagDefinition {
        name: "webmcp",
        label: "WebMCP UI tools",
        description: "Exposes a small browser-native tool surface from the authenticated UI so a \
             browser agent can search, navigate, and perform confirmed actions in Everruns.",
        experimental: true,
        platform_managed: false,
    },
    FeatureFlagDefinition {
        name: "projects",
        label: "Projects",
        description: "Adds a project layer nested inside your organization: switch projects from \
             the sidebar and scope agents and other resources to the project you are working in. \
             Existing resources move into a default project, so nothing changes until you opt in.",
        experimental: true,
        platform_managed: false,
    },
];

/// Whether a flag may only be enabled for an org by a platform user.
///
/// Unknown flags are not platform-managed: an unknown name is rejected as
/// unknown, which is a clearer error than "you are not allowed to".
pub fn is_platform_managed(flag: &str) -> bool {
    API_FEATURE_FLAG_DEFINITIONS
        .iter()
        .any(|definition| definition.name == flag && definition.platform_managed)
}

impl FeatureFlags {
    /// Effective flags for an organization: org-configurable deployment/system gates AND
    /// explicit org opt-in, plus deployment-only UI gates unchanged.
    ///
    /// Org overrides default to disabled; a flag is on only when the deployment allows it
    /// and the org has opted in (`enabled: true` in storage).
    pub fn for_org(system: &Self, org_enabled: &std::collections::HashMap<String, bool>) -> Self {
        let opt_in = |name: &str, system_on: bool| -> bool {
            system_on && org_enabled.get(name).copied().unwrap_or(false)
        };
        Self {
            notifications: opt_in("notifications", system.notifications),
            evals: opt_in("evals", system.evals),
            skills: opt_in("skills", system.skills),
            memory: opt_in("memory", system.memory),
            knowledge: opt_in("knowledge", system.knowledge),
            plugins: opt_in("plugins", system.plugins),
            app_budgets: opt_in("app_budgets", system.app_budgets),
            agent_versions: opt_in("agent_versions", system.agent_versions),
            voice: opt_in("voice", system.voice),
            agent_delegation: opt_in("agent_delegation", system.agent_delegation),
            observers: opt_in("observers", system.observers),
            platform_chat_v2: opt_in("platform_chat_v2", system.platform_chat_v2),
            public_chat: opt_in("public_chat", system.public_chat),
            webmcp: opt_in("webmcp", system.webmcp),
            environments: opt_in("environments", system.environments),
            machine_payments: system.machine_payments,
            projects: opt_in("projects", system.projects),
        }
    }

    /// Compute feature flags from environment variables and deployment grade.
    pub fn from_env(grade: &DeploymentGrade) -> Self {
        Self {
            notifications: experimental_flag("FEATURE_NOTIFICATIONS", grade),
            evals: experimental_flag("FEATURE_EVALS", grade),
            skills: experimental_flag("FEATURE_SKILLS", grade),
            memory: experimental_flag("FEATURE_MEMORY", grade),
            knowledge: experimental_flag("FEATURE_KNOWLEDGE", grade),
            plugins: experimental_flag("FEATURE_PLUGINS", grade),
            app_budgets: experimental_flag("FEATURE_APP_BUDGETS", grade),
            agent_versions: experimental_flag("FEATURE_AGENT_VERSIONS", grade),
            voice: experimental_flag("FEATURE_VOICE", grade),
            agent_delegation: experimental_flag("FEATURE_AGENT_DELEGATION", grade),
            observers: experimental_flag("FEATURE_OBSERVERS", grade),
            platform_chat_v2: experimental_flag("FEATURE_PLATFORM_CHAT_V2", grade),
            public_chat: experimental_flag("FEATURE_PUBLIC_CHAT", grade),
            webmcp: experimental_flag("FEATURE_WEBMCP", grade),
            // Environments describe the sandbox surface, so a deployment that
            // has already turned sandboxes on gets them without a second
            // switch. Dev keeps the experimental convenience of being on by
            // default; everywhere else this is an explicit platform decision.
            environments: standard_flag(
                "FEATURE_ENVIRONMENTS",
                sandboxes_enabled() || grade.experimental_features_enabled(),
            ),
            machine_payments: standard_flag("FEATURE_MACHINE_PAYMENTS", false),
            projects: experimental_flag("FEATURE_PROJECTS", grade),
        }
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
        FeatureFlagMap(BTreeMap::from([
            ("notifications".to_string(), self.notifications),
            ("evals".to_string(), self.evals),
            ("skills".to_string(), self.skills),
            ("memory".to_string(), self.memory),
            ("knowledge".to_string(), self.knowledge),
            ("plugins".to_string(), self.plugins),
            ("app_budgets".to_string(), self.app_budgets),
            ("agent_versions".to_string(), self.agent_versions),
            ("voice".to_string(), self.voice),
            ("agent_delegation".to_string(), self.agent_delegation),
            ("observers".to_string(), self.observers),
            ("environments".to_string(), self.environments),
            ("public_chat".to_string(), self.public_chat),
            ("webmcp".to_string(), self.webmcp),
            ("platform_chat_v2".to_string(), self.platform_chat_v2),
            ("machine_payments".to_string(), self.machine_payments),
            ("projects".to_string(), self.projects),
        ]))
    }

    /// Look up a flag by name (for dynamic/string-based access).
    pub fn is_enabled(&self, flag: &str) -> bool {
        match flag {
            "platform_chat_v2" => self.platform_chat_v2,
            "notifications" => self.notifications,
            "evals" => self.evals,
            "skills" => self.skills,
            "memory" => self.memory,
            "knowledge" => self.knowledge,
            "plugins" => self.plugins,
            "app_budgets" => self.app_budgets,
            "agent_versions" => self.agent_versions,
            "voice" => self.voice,
            "agent_delegation" => self.agent_delegation,
            "observers" => self.observers,
            "environments" => self.environments,
            "public_chat" => self.public_chat,
            "webmcp" => self.webmcp,
            "machine_payments" => self.machine_payments,
            "projects" => self.projects,
            _ => false,
        }
    }

    /// Whether a capability is available under these effective flags.
    pub fn is_capability_enabled(&self, capability_id: &str) -> bool {
        Self::required_for_capability(capability_id).is_none_or(|flag| self.is_enabled(flag))
    }

    /// Feature flag required by a capability, when one exists.
    pub fn required_for_capability(capability_id: &str) -> Option<&'static str> {
        match capability_id {
            "skills" => Some("skills"),
            "memory" => Some("memory"),
            "knowledge_index" | "knowledge_base" => Some("knowledge"),
            "a2a_agent_delegation" | "agent_handoff" => Some("agent_delegation"),
            _ if capability_id.starts_with("skill:") => Some("skills"),
            _ if capability_id.starts_with("plugin:") => Some("plugins"),
            _ => None,
        }
    }

    /// All flags enabled (for testing).
    #[cfg(test)]
    pub fn all_enabled() -> Self {
        Self {
            notifications: true,
            evals: true,
            skills: true,
            memory: true,
            knowledge: true,
            plugins: true,
            app_budgets: true,
            agent_versions: true,
            voice: true,
            agent_delegation: true,
            observers: true,
            platform_chat_v2: true,
            environments: true,
            public_chat: true,
            webmcp: true,
            machine_payments: true,
            projects: true,
        }
    }
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
        assert!(!flags.notifications);
    }

    // SAFETY: env var tests must run single-threaded (--test-threads=1).
    // set_var/remove_var are unsafe in edition 2024 due to thread-safety.

    #[test]
    fn test_experimental_enabled_in_dev() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_EVALS") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Dev);
        assert!(flags.evals);
    }

    #[test]
    fn test_experimental_disabled_in_prod() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_EVALS") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(!flags.evals);
    }

    #[test]
    fn test_env_override_enables_in_prod() {
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_EVALS", "true") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(flags.evals);
        unsafe { std::env::remove_var("FEATURE_EVALS") };
    }

    #[test]
    fn test_env_override_disables_in_dev() {
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_EVALS", "false") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Dev);
        assert!(!flags.evals);
        unsafe { std::env::remove_var("FEATURE_EVALS") };
    }

    #[test]
    fn environments_follows_sandbox_enablement_in_prod() {
        let _lock = lock_env();
        let previous_environments = std::env::var("FEATURE_ENVIRONMENTS").ok();
        let previous_sandbox = std::env::var("FEATURE_SESSION_SANDBOX").ok();
        unsafe { std::env::remove_var("FEATURE_ENVIRONMENTS") };

        unsafe { std::env::remove_var("FEATURE_SESSION_SANDBOX") };
        assert!(
            !FeatureFlags::from_env(&DeploymentGrade::Prod).environments,
            "a prod deployment with no sandboxes has nothing to describe"
        );

        unsafe { std::env::set_var("FEATURE_SESSION_SANDBOX", "true") };
        assert!(
            FeatureFlags::from_env(&DeploymentGrade::Prod).environments,
            "turning sandboxes on should not need a second switch"
        );

        // The explicit env var still wins, so a platform admin can opt out.
        unsafe { std::env::set_var("FEATURE_ENVIRONMENTS", "false") };
        assert!(!FeatureFlags::from_env(&DeploymentGrade::Prod).environments);

        restore_env("FEATURE_ENVIRONMENTS", previous_environments);
        restore_env("FEATURE_SESSION_SANDBOX", previous_sandbox);
    }

    #[test]
    fn environments_is_org_scoped_but_platform_managed() {
        // In the catalog, so an org opt-in row counts: the platform can enrol
        // one tenant and not another.
        assert!(
            API_FEATURE_FLAG_DEFINITIONS
                .iter()
                .any(|definition| definition.name == "environments" && definition.platform_managed),
            "environments is org-scoped but not the org's own decision"
        );
        assert!(is_platform_managed("environments"));
        assert!(!is_platform_managed("skills"));

        let system = FeatureFlags {
            environments: true,
            ..FeatureFlags::default()
        };

        // Enrolment is a row, and without one the flag stays off.
        assert!(!FeatureFlags::for_org(&system, &std::collections::HashMap::new()).environments);

        let enrolled = std::collections::HashMap::from([("environments".to_string(), true)]);
        assert!(FeatureFlags::for_org(&system, &enrolled).environments);

        // The deployment gate still comes first.
        assert!(!FeatureFlags::for_org(&FeatureFlags::default(), &enrolled).environments);
    }

    #[test]
    fn test_is_enabled_dynamic() {
        let flags = FeatureFlags {
            platform_chat_v2: false,
            notifications: true,
            evals: true,
            skills: true,
            memory: true,
            knowledge: true,
            plugins: true,
            app_budgets: true,
            agent_versions: true,
            voice: true,
            agent_delegation: true,
            observers: true,
            environments: true,
            public_chat: true,
            webmcp: true,
            machine_payments: true,
            projects: true,
        };
        assert!(flags.is_enabled("notifications"));
        assert!(flags.is_enabled("evals"));
        assert!(flags.is_enabled("skills"));
        assert!(flags.is_enabled("memory"));
        assert!(flags.is_enabled("knowledge"));
        assert!(flags.is_enabled("plugins"));
        assert!(flags.is_enabled("app_budgets"));
        assert!(flags.is_enabled("agent_versions"));
        assert!(flags.is_enabled("voice"));
        assert!(flags.is_enabled("agent_delegation"));
        assert!(flags.is_enabled("observers"));
        assert!(flags.is_enabled("public_chat"));
        assert!(
            !flags.is_enabled("mcp_endpoint"),
            "MCP is a product surface, not a feature flag"
        );
        assert!(flags.is_enabled("webmcp"));
        assert!(flags.is_enabled("machine_payments"));
        assert!(flags.is_enabled("projects"));
        assert!(!flags.is_enabled("nonexistent"));
    }

    #[test]
    fn agent_delegation_capabilities_follow_effective_flag() {
        let disabled = FeatureFlags::default();
        assert!(!disabled.is_capability_enabled("a2a_agent_delegation"));
        assert!(!disabled.is_capability_enabled("agent_handoff"));
        assert!(!disabled.is_capability_enabled("skills"));
        assert!(!disabled.is_capability_enabled("skill:019abc"));
        assert!(!disabled.is_capability_enabled("memory"));
        assert!(!disabled.is_capability_enabled("knowledge_index"));
        assert!(!disabled.is_capability_enabled("knowledge_base"));
        assert!(!disabled.is_capability_enabled("plugin:019abc"));
        assert!(disabled.is_capability_enabled("current_time"));

        let enabled = FeatureFlags {
            agent_delegation: true,
            skills: true,
            memory: true,
            knowledge: true,
            plugins: true,
            ..FeatureFlags::default()
        };
        assert!(enabled.is_capability_enabled("a2a_agent_delegation"));
        assert!(enabled.is_capability_enabled("agent_handoff"));
        assert!(enabled.is_capability_enabled("skills"));
        assert!(enabled.is_capability_enabled("skill:019abc"));
        assert!(enabled.is_capability_enabled("memory"));
        assert!(enabled.is_capability_enabled("knowledge_index"));
        assert!(enabled.is_capability_enabled("knowledge_base"));
        assert!(enabled.is_capability_enabled("plugin:019abc"));
    }

    #[test]
    fn test_serialization() {
        let flags = FeatureFlags {
            platform_chat_v2: false,
            notifications: true,
            evals: true,
            skills: true,
            memory: true,
            knowledge: true,
            plugins: true,
            app_budgets: true,
            agent_versions: true,
            voice: true,
            agent_delegation: true,
            observers: true,
            environments: true,
            public_chat: true,
            webmcp: true,
            machine_payments: true,
            projects: true,
        };
        let json = serde_json::to_string(&flags).unwrap();
        assert!(json.contains("\"notifications\":true"));
        assert!(json.contains("\"app_budgets\":true"));
        assert!(json.contains("\"agent_versions\":true"));
        assert!(json.contains("\"voice\":true"));
        assert!(json.contains("\"agent_delegation\":true"));
        assert!(json.contains("\"observers\":true"));
        assert!(json.contains("\"webmcp\":true"));
        assert!(json.contains("\"machine_payments\":true"));

        let parsed: FeatureFlags = serde_json::from_str(&json).unwrap();
        assert_eq!(flags, parsed);
    }

    #[test]
    fn test_to_map_matches_serialized_flags() {
        // `to_map` must produce the same keys/values as the struct's JSON form
        // (compared as `serde_json::Value`, so key order is ignored). If a field is
        // added to `FeatureFlags` but not to `to_map`, this fails — keeping the
        // untyped API representation in sync with the struct.
        let flags = FeatureFlags::all_enabled();
        let typed: serde_json::Value = serde_json::to_value(&flags).unwrap();
        let map: serde_json::Value = serde_json::to_value(flags.to_map()).unwrap();
        assert_eq!(typed, map);

        let default_typed: serde_json::Value =
            serde_json::to_value(FeatureFlags::default()).unwrap();
        let default_map: serde_json::Value =
            serde_json::to_value(FeatureFlags::default().to_map()).unwrap();
        assert_eq!(default_typed, default_map);
    }

    #[test]
    fn test_mcp_endpoint_is_not_in_api_feature_flag_catalog() {
        assert!(
            API_FEATURE_FLAG_DEFINITIONS
                .iter()
                .all(|definition| definition.name != "mcp_endpoint")
        );
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
            evals: true,
            notifications: true,
            machine_payments: true,
            ..FeatureFlags::default()
        };
        let mut org = std::collections::HashMap::new();
        org.insert("evals".to_string(), true);
        let effective = FeatureFlags::for_org(&system, &org);
        assert!(effective.evals);
        assert!(!effective.notifications);
        assert!(effective.machine_payments);

        let effective_none = FeatureFlags::for_org(&system, &std::collections::HashMap::new());
        assert!(!effective_none.evals);
        assert!(effective_none.machine_payments);
    }

    #[test]
    fn test_optional_ui_modules_require_org_opt_in() {
        let system = FeatureFlags {
            evals: true,
            skills: true,
            memory: true,
            knowledge: true,
            plugins: true,
            ..FeatureFlags::default()
        };
        let disabled = FeatureFlags::for_org(&system, &std::collections::HashMap::new());
        assert!(!disabled.evals);
        assert!(!disabled.skills);
        assert!(!disabled.memory);
        assert!(!disabled.knowledge);
        assert!(!disabled.plugins);

        let org = std::collections::HashMap::from([
            ("evals".to_string(), true),
            ("skills".to_string(), true),
            ("memory".to_string(), true),
            ("knowledge".to_string(), true),
            ("plugins".to_string(), true),
        ]);
        let enabled = FeatureFlags::for_org(&system, &org);
        assert!(enabled.evals);
        assert!(enabled.skills);
        assert!(enabled.memory);
        assert!(enabled.knowledge);
        assert!(enabled.plugins);
    }

    #[test]
    fn test_for_org_cannot_enable_when_system_off() {
        let system = FeatureFlags::default();
        let mut org = std::collections::HashMap::new();
        org.insert("evals".to_string(), true);
        let effective = FeatureFlags::for_org(&system, &org);
        assert!(!effective.evals);
    }

    #[test]
    fn test_notifications_respects_env_override() {
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_NOTIFICATIONS", "true") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(flags.notifications);
        unsafe { std::env::remove_var("FEATURE_NOTIFICATIONS") };
    }

    #[test]
    fn test_machine_payments_disabled_by_default() {
        let _lock = lock_env();
        let prev = std::env::var("FEATURE_MACHINE_PAYMENTS").ok();
        unsafe { std::env::remove_var("FEATURE_MACHINE_PAYMENTS") };
        let flags = FeatureFlags::from_env(&DeploymentGrade::Dev);
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
        let flags = FeatureFlags::from_env(&DeploymentGrade::Prod);
        assert!(flags.machine_payments);
        restore_env("FEATURE_MACHINE_PAYMENTS", prev);
    }
}

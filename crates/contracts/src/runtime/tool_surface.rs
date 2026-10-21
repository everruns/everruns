//! Shared routing contract for native tool calls and command-line adapters.

use crate::runtime::mcp_deferred::deferred_mcp_server_prefix;
use crate::tool_types::{DeferrablePolicy, ToolDefinition};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Builtin entry points and turn-control/image operations that must remain
/// directly callable when a command adapter is enabled. Tool owners can add
/// further exceptions with `ToolHints::stays_direct`.
pub const NATIVE_ONLY_TOOL_NAMES: &[&str] = &[
    "bash",
    "lua",
    "tools",
    "tool_search",
    "computer",
    "browser",
    "spawn_agent",
    "ask_user",
    "request_approval",
    "record_approval",
    "set_approval_mode",
    "generate_image",
    "edit_image",
];

fn native_disclosure_eligible(definition: &ToolDefinition) -> bool {
    !matches!(definition, ToolDefinition::ClientSide(_))
        && *definition.policy() != crate::tool_types::ToolPolicy::ClientSide
        && definition.hints().stays_direct != Some(true)
        && !NATIVE_ONLY_TOOL_NAMES.contains(&definition.name())
        && (*definition.deferrable() != DeferrablePolicy::Never
            || deferred_mcp_server_prefix(definition.name()).is_some())
}

/// Requested top-level tool execution surface for a session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSurfaceMode {
    /// Expose operations as native model tools.
    #[default]
    Native,
    /// Expose eligible operations through the shell command adapter.
    Cli,
    /// Expose eligible operations through both adapters.
    Hybrid,
}

/// How an operation is exposed at the model boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSurface {
    Native,
    Cli,
    Both,
}

impl Default for ToolSurface {
    fn default() -> Self {
        Self::Native
    }
}

/// Native schema loading is independent of whether an operation has a CLI route.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeToolDisclosure {
    #[default]
    Eager,
    Deferred,
}

/// Configured default and per-operation overrides for native schema loading.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeToolDisclosurePolicy {
    #[serde(default)]
    pub default: NativeToolDisclosure,
    #[serde(default)]
    pub overrides: BTreeMap<String, NativeToolDisclosure>,
}

impl NativeToolDisclosurePolicy {
    pub fn disclosure_for(&self, name: &str) -> NativeToolDisclosure {
        self.overrides.get(name).copied().unwrap_or(self.default)
    }

    /// Combine independent contributors. Any deferred default requests
    /// bounded disclosure; explicit eager operation overrides remain eager.
    pub fn merge(&mut self, other: Self) {
        if other.default == NativeToolDisclosure::Deferred {
            self.default = NativeToolDisclosure::Deferred;
        }
        self.overrides.extend(other.overrides);
    }
}

/// Resolved route for one canonical operation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSurfaceRoute {
    pub surface: ToolSurface,
    #[serde(default)]
    pub native_disclosure: NativeToolDisclosure,
}

/// Read-only surface plan shared with capability prompt and definition hooks.
///
/// The mode is resolved before prompt assembly. `routes` may be incomplete while
/// capability-owned and MCP definitions are still being collected; callers must
/// treat a missing route as no additional permission to invoke an operation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSurfacePlan {
    #[serde(default)]
    pub mode: ToolSurfaceMode,
    #[serde(default)]
    pub native_disclosure_policy: NativeToolDisclosurePolicy,
    #[serde(default)]
    pub routes: BTreeMap<String, ToolSurfaceRoute>,
}

impl ToolSurfacePlan {
    pub fn for_mode(mode: ToolSurfaceMode) -> Self {
        Self {
            mode,
            native_disclosure_policy: NativeToolDisclosurePolicy::default(),
            routes: BTreeMap::new(),
        }
    }

    pub fn from_definitions(
        mode: ToolSurfaceMode,
        definitions: &[ToolDefinition],
        cli_eligible: impl FnMut(&ToolDefinition) -> bool,
    ) -> Self {
        let mut plan = Self::for_mode(mode);
        plan.extend_definitions(definitions, cli_eligible);
        plan
    }

    pub fn route_for(&self, name: &str) -> Option<ToolSurfaceRoute> {
        self.routes.get(name).copied()
    }

    /// Resolve a late definition with this plan's mode and disclosure policy.
    /// Existing routes remain authoritative; unknown names are classified from
    /// their canonical definition and the caller's owner-aware CLI predicate.
    pub fn route_for_definition(
        &self,
        definition: &ToolDefinition,
        mut cli_eligible: impl FnMut(&ToolDefinition) -> bool,
    ) -> ToolSurfaceRoute {
        if let Some(route) = self.route_for(definition.name()) {
            return route;
        }
        let mut resolved = self.clone();
        resolved.routes.clear();
        resolved.extend_definitions(std::slice::from_ref(definition), &mut cli_eligible);
        resolved.route_for(definition.name()).unwrap_or_default()
    }

    pub fn extend(&mut self, routes: impl IntoIterator<Item = (String, ToolSurfaceRoute)>) {
        self.routes.extend(routes);
    }

    pub fn set_native_disclosure_policy(&mut self, policy: NativeToolDisclosurePolicy) {
        self.native_disclosure_policy = policy;
    }

    pub fn apply_native_disclosure_policy(&mut self, definitions: &[ToolDefinition]) {
        for definition in definitions {
            let Some(route) = self.routes.get_mut(definition.name()) else {
                continue;
            };
            let native_eligible = native_disclosure_eligible(definition);
            route.native_disclosure = match route.surface {
                ToolSurface::Cli => NativeToolDisclosure::Eager,
                ToolSurface::Native if native_eligible => self
                    .native_disclosure_policy
                    .disclosure_for(definition.name()),
                ToolSurface::Both if native_eligible => self
                    .native_disclosure_policy
                    .disclosure_for(definition.name()),
                ToolSurface::Native | ToolSurface::Both => NativeToolDisclosure::Eager,
            };
        }
    }

    /// Add host-classified operations to the plan. The classifier owns CLI
    /// eligibility and therefore can enforce adapter-specific semantics.
    pub fn extend_definitions(
        &mut self,
        definitions: &[ToolDefinition],
        mut cli_eligible: impl FnMut(&ToolDefinition) -> bool,
    ) {
        for definition in definitions {
            let cli = cli_eligible(definition);
            let native_eligible = native_disclosure_eligible(definition);
            let policy_disclosure = if native_eligible {
                self.native_disclosure_policy
                    .disclosure_for(definition.name())
            } else {
                NativeToolDisclosure::Eager
            };
            let (surface, native_disclosure) = match self.mode {
                ToolSurfaceMode::Native => (ToolSurface::Native, policy_disclosure),
                ToolSurfaceMode::Cli if cli => (ToolSurface::Cli, NativeToolDisclosure::Eager),
                ToolSurfaceMode::Cli => (ToolSurface::Native, NativeToolDisclosure::Eager),
                ToolSurfaceMode::Hybrid if cli => (
                    ToolSurface::Both,
                    if !native_eligible {
                        NativeToolDisclosure::Eager
                    } else {
                        self.native_disclosure_policy
                            .disclosure_for(definition.name())
                    },
                ),
                ToolSurfaceMode::Hybrid => (ToolSurface::Native, NativeToolDisclosure::Eager),
            };
            self.routes.insert(
                definition.name().to_string(),
                ToolSurfaceRoute {
                    surface,
                    native_disclosure,
                },
            );
        }
    }
}

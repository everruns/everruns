//! The capability registry and its builder.

//! Capabilities Module for Agent Loop
//!
//! This module provides the capabilities abstraction that allows composing
//! agent functionality through modular units. Each capability can contribute:
//! - System prompt additions
//! - Tools for the agent
//! - Behavior modifications (future)
//!
//! Design decisions:
//! - Capabilities are defined via the Capability trait for flexibility
//! - CapabilityRegistry holds all available capability implementations
//! - apply_capabilities() merges capability contributions into RuntimeAgent
//! - The agent-loop remains execution-focused; capabilities are applied before execution
//! - System prompt sections use XML tags for clear boundaries between components.
//!   This follows Anthropic's recommendation for multi-component prompts and reduces
//!   misattribution between capability instructions, user-provided AGENTS.md, and the
//!   agent's base system prompt. See knowledge/project/xml-prompt-formatting.md for rationale.
//!
//! Each capability is in its own file with collocated tools.

use std::collections::HashMap;
use std::sync::Arc;

use super::*;

/// Registry that holds all available capability implementations.
///
/// The registry provides access to capabilities by ID and allows
/// applying multiple capabilities to build a RuntimeAgent.
///
/// # Example
///
/// ```
/// use everruns_core::capabilities::CapabilityRegistry;
///
/// let registry = CapabilityRegistry::new();
/// assert!(registry.is_empty());
/// ```
#[derive(Clone)]
pub struct CapabilityRegistry {
    capabilities: HashMap<String, Arc<dyn Capability>>,
    /// Canonical-id/alias bookkeeping delegated to the neutral capability
    /// contract so the Framework and product resolve identity identically
    /// (see [`Capability::aliases`]).
    index: everruns_capability::CapabilityIdIndex,
}

impl CapabilityRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            capabilities: HashMap::new(),
            index: everruns_capability::CapabilityIdIndex::new(),
        }
    }

    /// Register a capability
    pub fn register(&mut self, capability: impl Capability + 'static) {
        self.register_arc(Arc::new(capability));
    }

    /// Register a boxed capability
    pub fn register_boxed(&mut self, capability: Box<dyn Capability>) {
        self.register_arc(Arc::from(capability));
    }

    /// Register an Arc-wrapped capability.
    ///
    /// Re-registering the same canonical ID replaces the previous
    /// implementation (legacy override semantics); use
    /// [`CapabilityRegistry::try_register_arc`] to reject collisions instead.
    pub fn register_arc(&mut self, capability: Arc<dyn Capability>) {
        let canonical = capability.id().to_string();
        self.index
            .insert_or_replace(canonical.clone(), &capability.aliases());
        self.capabilities.insert(canonical, capability);
    }

    /// Register an Arc-wrapped capability, rejecting duplicate IDs and alias
    /// collisions via the neutral contract's registry rules.
    pub fn try_register_arc(
        &mut self,
        capability: Arc<dyn Capability>,
    ) -> Result<(), everruns_capability::CapabilityError> {
        let canonical = capability.id().to_string();
        self.index
            .insert(canonical.clone(), &capability.aliases())?;
        self.capabilities.insert(canonical, capability);
        Ok(())
    }

    /// Register integration plugins accepted by a caller-owned policy.
    ///
    /// Core owns the registry mutation algorithm; host and product composition
    /// own the catalog supplied by `plugins` and every deployment-grade and
    /// feature decision supplied by `include`.
    pub fn register_plugins<'a>(
        &mut self,
        plugins: impl IntoIterator<Item = &'a IntegrationPlugin>,
        mut include: impl FnMut(&IntegrationPlugin) -> bool,
    ) {
        for plugin in plugins {
            if include(plugin) {
                self.register_boxed((plugin.factory)());
            }
        }
    }

    /// Get a capability by ID or alias
    pub fn get(&self, id: &str) -> Option<&Arc<dyn Capability>> {
        self.capabilities.get(self.index.canonical_of(id)?)
    }

    /// Resolve an ID or alias to the canonical capability ID.
    ///
    /// Returns `None` for IDs that are neither registered nor an alias of a
    /// registered capability (e.g. declarative or MCP refs).
    pub fn canonical_id<'a>(&'a self, id: &'a str) -> Option<&'a str> {
        self.index.canonical_of(id)
    }

    /// Remove a capability from the registry by ID or alias.
    pub fn unregister(&mut self, id: &str) -> Option<Arc<dyn Capability>> {
        let canonical = self.index.remove(id)?;
        self.capabilities.remove(&canonical)
    }

    /// Check if a capability is registered (by ID or alias)
    pub fn has(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    /// Get all registered capabilities
    pub fn list(&self) -> Vec<&Arc<dyn Capability>> {
        self.capabilities.values().collect()
    }

    /// Get the number of registered capabilities
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }

    /// Create a builder for fluent capability registration
    pub fn builder() -> CapabilityRegistryBuilder {
        CapabilityRegistryBuilder::new()
    }

    /// Find a blueprint by ID across all registered capabilities.
    ///
    /// Returns a fresh `AgentBlueprint` (with new tool instances) each time.
    pub fn blueprint(&self, id: &str) -> Option<AgentBlueprint> {
        for cap in self.capabilities.values() {
            for bp in cap.agent_blueprints() {
                if bp.id == id {
                    return Some(bp);
                }
            }
        }
        None
    }

    /// Find a blueprint and the capability that registered it.
    ///
    /// Returns `(capability_id, blueprint)` with fresh tool instances.
    pub fn blueprint_with_capability(&self, id: &str) -> Option<(String, AgentBlueprint)> {
        for (capability_id, cap) in &self.capabilities {
            for bp in cap.agent_blueprints() {
                if bp.id == id {
                    return Some((capability_id.clone(), bp));
                }
            }
        }
        None
    }

    /// Collect all blueprints from all registered capabilities.
    pub fn all_blueprints(&self) -> Vec<AgentBlueprint> {
        self.capabilities
            .values()
            .flat_map(|cap| cap.agent_blueprints())
            .collect()
    }
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CapabilityRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<_> = self.capabilities.keys().collect();
        f.debug_struct("CapabilityRegistry")
            .field("capabilities", &ids)
            .finish()
    }
}

/// Builder for creating a CapabilityRegistry with a fluent API
pub struct CapabilityRegistryBuilder {
    registry: CapabilityRegistry,
}

impl CapabilityRegistryBuilder {
    /// Create a new builder with an empty registry
    pub fn new() -> Self {
        Self {
            registry: CapabilityRegistry::new(),
        }
    }

    /// Add a capability
    pub fn capability(mut self, capability: impl Capability + 'static) -> Self {
        self.registry.register(capability);
        self
    }

    /// Build the registry
    pub fn build(self) -> CapabilityRegistry {
        self.registry
    }
}

impl Default for CapabilityRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Collect Capabilities Helper
// ============================================================================

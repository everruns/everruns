//! Resolving capability dependencies, configs and derived features.

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

use everruns_capability::is_plugin_capability;

use super::*;

/// Maximum number of capabilities after dependency resolution.
/// This prevents runaway dependency chains and resource exhaustion.
pub const MAX_RESOLVED_CAPABILITIES: usize = 100;

/// Error type for dependency resolution failures
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyError {
    /// Circular dependency detected in the capability graph
    CircularDependency {
        /// The capability where the cycle was detected
        capability_id: String,
        /// The dependency chain leading to the cycle
        chain: Vec<String>,
    },
    /// Too many capabilities after resolution
    TooManyCapabilities {
        /// Number of capabilities requested
        count: usize,
        /// Maximum allowed
        max: usize,
    },
}

impl std::fmt::Display for DependencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DependencyError::CircularDependency {
                capability_id,
                chain,
            } => {
                write!(
                    f,
                    "Circular dependency detected: {} depends on itself via chain: {} -> {}",
                    capability_id,
                    chain.join(" -> "),
                    capability_id
                )
            }
            DependencyError::TooManyCapabilities { count, max } => {
                write!(
                    f,
                    "Too many capabilities after resolution: {} (max: {})",
                    count, max
                )
            }
        }
    }
}

impl std::error::Error for DependencyError {}

/// Result of resolving capability dependencies
#[derive(Debug, Clone)]
pub struct ResolvedCapabilities {
    /// All capability IDs after resolving dependencies (in topological order)
    /// Dependencies come before dependents.
    pub resolved_ids: Vec<String>,
    /// IDs that were added as dependencies (not in the original selection)
    pub added_as_dependencies: Vec<String>,
    /// Original user-selected capability IDs
    pub user_selected: Vec<String>,
}

/// Resolve capability dependencies, returning all required capability IDs.
///
/// This function:
/// 1. Takes the user-selected capability IDs
/// 2. Recursively collects all dependencies
/// 3. Returns them in topological order (dependencies before dependents)
/// 4. Detects circular dependencies and returns an error
/// 5. Enforces a maximum capability limit
///
/// # Arguments
///
/// * `selected_ids` - User-selected capability IDs
/// * `registry` - The capability registry to look up dependencies
///
/// # Returns
///
/// `Ok(ResolvedCapabilities)` with all required capabilities in order,
/// or `Err(DependencyError)` if circular dependencies are detected or
/// the limit is exceeded.
pub fn resolve_dependencies(
    selected_ids: &[String],
    registry: &CapabilityRegistry,
) -> Result<ResolvedCapabilities, DependencyError> {
    use std::collections::HashSet;

    // Canonicalize so capabilities selected via alias match their resolved IDs.
    let user_selected: HashSet<String> = selected_ids
        .iter()
        .map(|id| registry.canonical_id(id).unwrap_or(id).to_string())
        .collect();
    let mut resolved: Vec<String> = Vec::new();
    let mut resolved_set: HashSet<String> = HashSet::new();
    let mut added_as_dependencies: Vec<String> = Vec::new();

    // Process each selected capability and its dependencies using DFS
    for cap_id in selected_ids {
        resolve_single_capability(
            cap_id,
            registry,
            &mut resolved,
            &mut resolved_set,
            &mut added_as_dependencies,
            &user_selected,
            &mut Vec::new(), // visiting chain for cycle detection
        )?;
    }

    // Check max limit
    if resolved.len() > MAX_RESOLVED_CAPABILITIES {
        return Err(DependencyError::TooManyCapabilities {
            count: resolved.len(),
            max: MAX_RESOLVED_CAPABILITIES,
        });
    }

    Ok(ResolvedCapabilities {
        resolved_ids: resolved,
        added_as_dependencies,
        user_selected: selected_ids.to_vec(),
    })
}

/// Resolve dependency-expanded capability configs, preserving explicit config on selected IDs.
///
/// Dependencies are inserted with empty configs. If the same capability is provided more than
/// once, the last explicit config wins.
pub fn resolve_capability_configs(
    selected_configs: &[AgentCapabilityConfig],
    registry: &CapabilityRegistry,
) -> Result<Vec<AgentCapabilityConfig>, DependencyError> {
    let mut selected_ids: Vec<String> = Vec::new();
    for config in selected_configs {
        // Both `declarative:` and `plugin:` carry a `DeclarativeCapabilityDefinition`
        // config that may declare dependencies.
        if (is_declarative_capability(config.capability_id())
            || is_plugin_capability(config.capability_id()))
            && let Ok(definition) = serde_json::from_value::<DeclarativeCapabilityDefinition>(
                config.config_value().clone(),
            )
        {
            selected_ids.extend(definition.dependencies);
        }
        selected_ids.push(config.capability_id().to_string());
    }
    let resolved = resolve_dependencies(&selected_ids, registry)?;

    // Key explicit configs by canonical ID so config supplied under an alias
    // still attaches to the (canonical) resolved capability ID.
    let explicit_configs: std::collections::HashMap<String, serde_json::Value> = selected_configs
        .iter()
        .map(|config| {
            let id = config.capability_id();
            let id = registry.canonical_id(id).unwrap_or(id);
            (id.to_string(), config.config_value().clone())
        })
        .collect();

    Ok(resolved
        .resolved_ids
        .into_iter()
        .map(|capability_id| {
            explicit_configs
                .get(&capability_id)
                .cloned()
                .map(|config| AgentCapabilityConfig::with_config(capability_id.clone(), config))
                .unwrap_or_else(|| AgentCapabilityConfig::new(capability_id))
        })
        .collect())
}

/// Helper function to resolve a single capability and its dependencies recursively.
pub(crate) fn resolve_single_capability(
    cap_id: &str,
    registry: &CapabilityRegistry,
    resolved: &mut Vec<String>,
    resolved_set: &mut std::collections::HashSet<String>,
    added_as_dependencies: &mut Vec<String>,
    user_selected: &std::collections::HashSet<String>,
    visiting: &mut Vec<String>,
) -> Result<(), DependencyError> {
    // Normalize aliases to the canonical ID so an alias and its canonical ID
    // resolve (and dedupe) to the same capability. Unknown IDs (declarative,
    // MCP, skill refs) pass through unchanged.
    let cap_id = registry.canonical_id(cap_id).unwrap_or(cap_id);

    // Already resolved
    if resolved_set.contains(cap_id) {
        return Ok(());
    }

    // Check for circular dependency
    if visiting.contains(&cap_id.to_string()) {
        return Err(DependencyError::CircularDependency {
            capability_id: cap_id.to_string(),
            chain: visiting.clone(),
        });
    }

    // Get capability from registry
    let capability = match registry.get(cap_id) {
        Some(cap) => cap,
        None => {
            // `declarative:` and `plugin:` refs carry their full definition in
            // the config payload — they don't need a registry entry. Pass them
            // through so `collect_capabilities_with_configs` can process them.
            if (is_declarative_capability(cap_id) || is_plugin_capability(cap_id))
                && !resolved_set.contains(cap_id)
            {
                resolved.push(cap_id.to_string());
                resolved_set.insert(cap_id.to_string());
                if !user_selected.contains(cap_id) {
                    added_as_dependencies.push(cap_id.to_string());
                }
            }
            return Ok(());
        }
    };

    // Mark as visiting
    visiting.push(cap_id.to_string());

    // Resolve dependencies first (depth-first)
    for dep_id in capability.dependencies() {
        resolve_single_capability(
            dep_id,
            registry,
            resolved,
            resolved_set,
            added_as_dependencies,
            user_selected,
            visiting,
        )?;
    }

    // Remove from visiting
    visiting.pop();

    // Add to resolved
    if !resolved_set.contains(cap_id) {
        resolved.push(cap_id.to_string());
        resolved_set.insert(cap_id.to_string());

        // Track if this was added as a dependency (not user-selected)
        if !user_selected.contains(cap_id) {
            added_as_dependencies.push(cap_id.to_string());
        }
    }

    Ok(())
}

/// Compute the aggregated set of UI features from a list of capability IDs.
///
/// Resolves dependencies, collects features from all resolved capabilities,
/// and returns deduplicated feature strings.
pub fn compute_features(capability_ids: &[String], registry: &CapabilityRegistry) -> Vec<String> {
    use std::collections::HashSet;

    let resolved_ids = match resolve_dependencies(capability_ids, registry) {
        Ok(resolved) => resolved.resolved_ids,
        Err(_) => capability_ids.to_vec(),
    };

    let mut seen = HashSet::new();
    let mut features = Vec::new();
    for cap_id in &resolved_ids {
        if let Some(cap) = registry.get(cap_id) {
            for feature in cap.features() {
                if seen.insert(feature) {
                    features.push(feature.to_string());
                }
            }
        }
    }
    features
}

/// Get direct dependencies for a capability ID.
/// Returns empty vec if capability not found.
pub fn get_dependencies(cap_id: &str, registry: &CapabilityRegistry) -> Vec<String> {
    registry
        .get(cap_id)
        .map(|cap| cap.dependencies().iter().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

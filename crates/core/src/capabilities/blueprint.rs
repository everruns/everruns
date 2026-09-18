//! Agent blueprints and the errors raised validating their config.

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

use crate::tool_types::ToolDefinition;
use crate::tools::Tool;
use serde::{Deserialize, Serialize};

/// Risk classification for capabilities (TM-AGENT-005).
///
/// Used to enforce approval requirements when assigning capabilities.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "low"))]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// No special approval needed
    Low,
    /// Logged but allowed for org members
    Medium,
    /// Requires org admin role to assign
    High,
}

// ============================================================================
// Agent Blueprints
// ============================================================================

/// Model selection strategy for agent blueprints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintModel {
    /// Always use this model. Host cannot override.
    Fixed(String),
    /// Use this model unless host provides override via config.
    Default(String),
    /// Use whatever model the host agent uses.
    Inherit,
}

/// Pre-built agent definition with private tools, baked-in prompt, and model selection.
///
/// Contributed by capabilities via `agent_blueprints()`. Spawned via
/// `spawn_agent` with a subagent target and `blueprint`. Blueprint tools never appear in the
/// host agent's tool list — they exist only inside the spawned child session.
pub struct AgentBlueprint {
    /// Unique identifier (e.g. `"github_scout"`)
    pub id: &'static str,
    /// Human-readable display name
    pub name: &'static str,
    /// When to use this blueprint (LLM reads this for delegation decisions)
    pub description: &'static str,
    /// Model selection strategy
    pub model: BlueprintModel,
    /// Baked-in system prompt for the child agent
    pub system_prompt: &'static str,
    /// Private tools — only available inside the blueprint's session
    pub tools: Vec<Box<dyn Tool>>,
    /// Iteration limit (default: 20)
    pub max_turns: Option<usize>,
    /// JSON Schema for allowed host-provided config. `None` = no config accepted.
    pub config_schema: Option<serde_json::Value>,
}

impl AgentBlueprint {
    /// Convert blueprint tools to tool definitions (for RuntimeAgent building).
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|t| t.to_definition()).collect()
    }

    /// Validate host-provided config against this blueprint's `config_schema`.
    ///
    /// Spawn paths call this before creating a child session, so a blueprint's
    /// declared schema is an enforced contract rather than prompt-level advice.
    /// Blueprints that declare no schema accept no config at all.
    pub fn validate_config(
        &self,
        config: Option<&serde_json::Value>,
    ) -> Result<(), BlueprintConfigError> {
        let Some(schema) = self.config_schema.as_ref() else {
            return match config {
                Some(_) => Err(BlueprintConfigError::NotAccepted { id: self.id }),
                None => Ok(()),
            };
        };

        let Some(config) = config else {
            // Only a schema with required properties makes config mandatory.
            let required = schema
                .get("required")
                .and_then(|r| r.as_array())
                .is_some_and(|required| !required.is_empty());
            return if required {
                Err(BlueprintConfigError::Required { id: self.id })
            } else {
                Ok(())
            };
        };

        let validator = jsonschema::validator_for(schema).map_err(|error| {
            BlueprintConfigError::InvalidSchema {
                id: self.id,
                reason: error.to_string(),
            }
        })?;

        let issues: Vec<String> = validator
            .iter_errors(config)
            .map(|error| {
                let path = error.instance_path().to_string();
                if path.is_empty() {
                    error.to_string()
                } else {
                    format!("{path}: {error}")
                }
            })
            .collect();

        if issues.is_empty() {
            Ok(())
        } else {
            Err(BlueprintConfigError::Invalid {
                id: self.id,
                issues,
            })
        }
    }
}

/// Why host-provided blueprint configuration was rejected.
///
/// `#[non_exhaustive]` keeps new rejection reasons a patch-sized addition for
/// downstream crates rather than a breaking change.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlueprintConfigError {
    /// The blueprint declares no config schema, so it accepts no config.
    NotAccepted {
        /// The blueprint that was asked to take config.
        id: &'static str,
    },
    /// The schema has required properties but no config was supplied.
    Required {
        /// The blueprint whose schema requires config.
        id: &'static str,
    },
    /// The config did not validate against the schema.
    Invalid {
        /// The blueprint the config was addressed to.
        id: &'static str,
        /// One entry per schema violation, prefixed with its instance path.
        issues: Vec<String>,
    },
    /// The blueprint's own schema is not a usable JSON Schema document.
    InvalidSchema {
        /// The blueprint carrying the unusable schema.
        id: &'static str,
        /// Why the schema could not be compiled.
        reason: String,
    },
}

impl std::fmt::Display for BlueprintConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAccepted { id } => {
                write!(f, "Blueprint \"{id}\" accepts no config.")
            }
            Self::Required { id } => {
                write!(f, "Blueprint \"{id}\" requires config.")
            }
            Self::Invalid { id, issues } => {
                write!(
                    f,
                    "Blueprint \"{id}\" received invalid config: {}",
                    issues.join("; ")
                )
            }
            Self::InvalidSchema { id, reason } => {
                write!(
                    f,
                    "Blueprint \"{id}\" has an invalid config schema: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for BlueprintConfigError {}

impl std::fmt::Debug for AgentBlueprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentBlueprint")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("model", &self.model)
            .field("tool_count", &self.tools.len())
            .field("max_turns", &self.max_turns)
            .finish()
    }
}

// ============================================================================
// Capability Registry
// ============================================================================

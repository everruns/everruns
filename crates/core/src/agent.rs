// Agent domain types
//
// These types represent the Agent entity and its status.
// Used by both API and worker crates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::capability_types::AgentCapabilityConfig;
use crate::events::TokenUsage;
use crate::typed_id::{AgentId, ModelId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Agent lifecycle status.
/// - `active`: Agent is available for use
/// - `archived`: Agent is soft-deleted and hidden from listings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    /// Agent is available for use.
    Active,
    /// Agent is soft-deleted and hidden from listings.
    Archived,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Active => write!(f, "active"),
            AgentStatus::Archived => write!(f, "archived"),
        }
    }
}

impl From<&str> for AgentStatus {
    fn from(s: &str) -> Self {
        match s {
            "archived" => AgentStatus::Archived,
            _ => AgentStatus::Active,
        }
    }
}

/// Agent configuration for agentic loop.
/// An agent defines the behavior and capabilities of an AI assistant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Agent {
    /// Unique identifier for the agent (format: agt_{32-hex}).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "agt_01933b5a00007000800000000000001"))]
    pub id: AgentId,
    /// Display name of the agent.
    pub name: String,
    /// Human-readable description of what the agent does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// System prompt that defines the agent's behavior.
    /// Sent as the first message in every conversation.
    pub system_prompt: String,
    /// Default LLM model ID for this agent.
    /// Can be overridden at the session level.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "mod_01933b5a00007000800000000000001"))]
    pub default_model_id: Option<ModelId>,
    /// Tags for organizing and filtering agents.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Capabilities enabled for this agent with per-agent configuration.
    /// Capabilities add tools and system prompt modifications.
    #[serde(default)]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Current lifecycle status of the agent.
    pub status: AgentStatus,
    /// Timestamp when the agent was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the agent was last updated.
    pub updated_at: DateTime<Utc>,
    /// Cumulative token usage across all sessions for this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

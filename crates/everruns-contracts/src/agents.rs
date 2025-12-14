// Agent-related DTOs for public API

use crate::tools::ToolDefinition;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Agent is a configured AI assistant with a specific model and behavior
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Agent {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub default_model_id: String,
    pub definition: serde_json::Value,
    pub status: AgentStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Status of an agent
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Active,
    Disabled,
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Active => write!(f, "active"),
            AgentStatus::Disabled => write!(f, "disabled"),
        }
    }
}

impl std::str::FromStr for AgentStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(AgentStatus::Active),
            "disabled" => Ok(AgentStatus::Disabled),
            _ => Err(format!("Unknown agent status: {}", s)),
        }
    }
}

/// Typed agent definition (stored as JSON in database)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    /// System prompt for the agent
    pub system: String,
    /// LLM configuration (model, temperature, etc.)
    #[serde(default)]
    pub llm: Option<LlmConfig>,
    /// Tool definitions available to the agent
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
}

/// LLM configuration for agent execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Model ID to use (overrides agent's default_model_id if set)
    pub model: Option<String>,
    /// Temperature (0.0-2.0)
    pub temperature: Option<f32>,
    /// Maximum tokens to generate
    pub max_tokens: Option<u32>,
    /// Top-p sampling
    pub top_p: Option<f32>,
}

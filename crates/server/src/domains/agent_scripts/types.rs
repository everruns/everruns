// Agent-scripts domain types: request shapes for the HTTP/MCP/CLI surface.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

pub use crate::storage::{AgentScriptRow, CreateAgentScriptRow, UpdateAgentScript};

/// Request to create a saved script on an agent.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct CreateAgentScriptRequest {
    /// Script name: lowercase letters, digits, `_` and `-`, starting with a
    /// letter, at most 64 characters. Unique among the agent's active scripts.
    #[schema(example = "daily-digest")]
    pub name: String,
    /// One-line description, 1 to 300 characters.
    #[schema(example = "Summarize yesterday's open issues")]
    pub description: String,
    /// JSON Schema of the input; must be an object schema (`"type": "object"`).
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub input_schema: Option<Value>,
    /// Shell script source, 1 to 65536 bytes.
    #[schema(example = "echo hello")]
    pub body: String,
}

/// Request to update a saved script. Only provided fields change; the name is
/// immutable.
#[derive(Debug, Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct UpdateAgentScriptRequest {
    /// Replacement description.
    #[serde(default)]
    pub description: Option<String>,
    /// Replacement input schema (an object schema).
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub input_schema: Option<Value>,
    /// Replacement script source.
    #[serde(default)]
    pub body: Option<String>,
}

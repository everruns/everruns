// Agent domain types
//
// Design Decision: Dual-ID pattern (see specs/id-schema.md)
// - public_id: AgentId (external, API-facing, client-supplied or auto-generated)
// - internal_id: Uuid (internal PK, used for FK references, never exposed in API)
//
// These types represent the Agent entity and its status.
// Used by both API and worker crates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
    /// External identifier (agent_<32-hex>). Shown as "id" in API.
    /// Client-supplied or auto-generated.
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "agent_01933b5a000070008000000000000001"))]
    pub public_id: AgentId,
    /// Internal UUID primary key. Used for FK references. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
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
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001"))]
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

/// Generate a new agent public_id using UUIDv7.
pub fn generate_agent_public_id() -> AgentId {
    AgentId::new()
}

/// Validate an agent public_id string.
/// Must match format: agent_<32-lowercase-hex-chars>
pub fn validate_agent_public_id(s: &str) -> bool {
    s.parse::<AgentId>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_agent_public_id() {
        let id = generate_agent_public_id();
        let s = id.to_string();
        assert!(s.starts_with("agent_"));
        assert_eq!(s.len(), 38); // "agent_" (6) + 32 hex chars
        assert!(validate_agent_public_id(&s));
    }

    #[test]
    fn test_validate_agent_public_id() {
        assert!(validate_agent_public_id(
            "agent_01933b5a000070008000000000000001"
        ));
        assert!(validate_agent_public_id(
            "agent_4ab3e8452f1442e9865e11d2032a579c"
        ));

        // Invalid cases
        assert!(!validate_agent_public_id(""));
        assert!(!validate_agent_public_id("agent_"));
        assert!(!validate_agent_public_id(
            "session_01933b5a000070008000000000000001"
        ));
        assert!(!validate_agent_public_id(
            "agent_4AB3E8452F1442E9865E11D2032A579C"
        )); // uppercase
        assert!(!validate_agent_public_id("agent_short"));
        assert!(!validate_agent_public_id("my-custom-agent"));
    }

    #[test]
    fn test_agent_serde_public_id_as_id() {
        let agent = Agent {
            public_id: "agent_01933b5a000070008000000000000001".parse().unwrap(),
            internal_id: Uuid::nil(),
            name: "Test".to_string(),
            description: None,
            system_prompt: "test".to_string(),
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            status: AgentStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            usage: None,
        };

        let json = serde_json::to_value(&agent).unwrap();
        // public_id should serialize as "id"
        assert_eq!(json["id"], "agent_01933b5a000070008000000000000001");
        // internal_id should be skipped
        assert!(json.get("internal_id").is_none());
    }

    #[test]
    fn test_agent_deserialize_from_api_json() {
        let json = serde_json::json!({
            "id": "agent_01933b5a000070008000000000000001",
            "name": "Test",
            "system_prompt": "test",
            "status": "active",
            "tags": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        });

        let agent: Agent = serde_json::from_value(json).unwrap();
        assert_eq!(
            agent.public_id.to_string(),
            "agent_01933b5a000070008000000000000001"
        );
        // internal_id defaults to nil when not present in JSON
        assert_eq!(agent.internal_id, Uuid::nil());
    }
}

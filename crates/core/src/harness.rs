// Harness domain types
//
// Harness defines base rules and capabilities for sessions.
// Agent (optional) provides domain-specific customizations on top.
// Hierarchy: Harness → Agent → Session

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::capability_types::AgentCapabilityConfig;
use crate::typed_id::{HarnessId, ModelId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Harness lifecycle status.
/// - `active`: Harness is available for use
/// - `archived`: Harness is soft-deleted and hidden from listings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum HarnessStatus {
    /// Harness is available for use.
    Active,
    /// Harness is soft-deleted and hidden from listings.
    Archived,
}

impl std::fmt::Display for HarnessStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessStatus::Active => write!(f, "active"),
            HarnessStatus::Archived => write!(f, "archived"),
        }
    }
}

impl From<&str> for HarnessStatus {
    fn from(s: &str) -> Self {
        match s {
            "archived" => HarnessStatus::Archived,
            _ => HarnessStatus::Active,
        }
    }
}

/// Harness configuration for sessions.
/// A harness defines the base behavior and capabilities that apply to all sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Harness {
    /// Unique identifier for the harness (format: harness_{32-hex}).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "harness_01933b5a00007000800000000000001"))]
    pub id: HarnessId,
    /// Display name of the harness.
    pub name: String,
    /// Human-readable description of what the harness does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// System prompt that defines the harness's base behavior.
    /// Forms the foundation of the prompt stack.
    pub system_prompt: String,
    /// Default LLM model ID for this harness.
    /// Lowest priority in chain: controls > session > agent > harness.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001"))]
    pub default_model_id: Option<ModelId>,
    /// Tags for organizing and filtering harnesses.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Capabilities enabled for this harness with per-harness configuration.
    #[serde(default)]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Whether this harness is built-in (system-managed, readonly).
    /// Built-in harnesses are provisioned during org initialization and
    /// cannot be modified or deleted via the API. Users can copy them.
    #[serde(default)]
    pub is_built_in: bool,
    /// Current lifecycle status of the harness.
    pub status: HarnessStatus,
    /// Timestamp when the harness was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the harness was last updated.
    pub updated_at: DateTime<Utc>,
}

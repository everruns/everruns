// Harness DTOs for public API (M2)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Harness represents the configuration for an agentic loop
/// A harness can have many concurrent sessions
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Harness {
    pub id: Uuid,
    pub slug: String,
    pub display_name: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub default_model_id: Option<Uuid>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<i32>,
    pub tags: Vec<String>,
    pub status: HarnessStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Status of a harness
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HarnessStatus {
    Active,
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

impl std::str::FromStr for HarnessStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(HarnessStatus::Active),
            "archived" => Ok(HarnessStatus::Archived),
            _ => Err(format!("Unknown harness status: {}", s)),
        }
    }
}

/// Request to create a harness
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateHarnessRequest {
    pub slug: String,
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub system_prompt: String,
    #[serde(default)]
    pub default_model_id: Option<Uuid>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<i32>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Request to update a harness
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateHarnessRequest {
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub default_model_id: Option<Uuid>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<i32>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub status: Option<HarnessStatus>,
}

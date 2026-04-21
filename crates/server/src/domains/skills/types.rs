// Skills domain types — canonical definitions for request shapes.
//
// Storage row types are re-exported from `storage::models` so domain code
// has a single import path.

use everruns_core::SkillStatus;
use serde::Deserialize;
use utoipa::ToSchema;

pub use crate::storage::models::{CreateSkillRow, SkillRow, UpdateSkill};

/// Request to create a skill from SKILL.md content
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateSkillRequest {
    /// Full SKILL.md content (YAML frontmatter + markdown body)
    #[schema(
        example = "---\nname: pdf-processing\ndescription: Extract text from PDFs.\n---\n\n# PDF Processing\n\nUse pdfplumber..."
    )]
    pub skill_md: String,
}

/// Request to update a skill
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateSkillRequest {
    /// Updated SKILL.md content (re-parses frontmatter)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_md: Option<String>,
    /// Update status
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<SkillStatus>,
}

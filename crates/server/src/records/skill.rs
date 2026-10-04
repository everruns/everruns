//! Stored skill catalog records; the SKILL.md parser remains portable in core.
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::SkillId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
fn default_true() -> bool {
    true
}
/// Skill source type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SkillSourceType {
    /// Single SKILL.md file (instructions only)
    Markdown,
    /// ZIP archive with SKILL.md + scripts/references/assets
    Archive,
}

impl std::fmt::Display for SkillSourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillSourceType::Markdown => write!(f, "markdown"),
            SkillSourceType::Archive => write!(f, "archive"),
        }
    }
}

impl From<&str> for SkillSourceType {
    fn from(s: &str) -> Self {
        match s {
            "archive" => SkillSourceType::Archive,
            _ => SkillSourceType::Markdown,
        }
    }
}

/// Skill lifecycle status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SkillStatus {
    Active,
    Disabled,
    Archived,
    Deleted,
}

impl std::fmt::Display for SkillStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillStatus::Active => write!(f, "active"),
            SkillStatus::Disabled => write!(f, "disabled"),
            SkillStatus::Archived => write!(f, "archived"),
            SkillStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for SkillStatus {
    fn from(s: &str) -> Self {
        match s {
            "disabled" => SkillStatus::Disabled,
            "archived" => SkillStatus::Archived,
            "deleted" => SkillStatus::Deleted,
            _ => SkillStatus::Active,
        }
    }
}

/// Skill entity (API response type)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Skill {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    #[schema(value_type = String, example = "skill_01933b5a00007000800000000000001")]
    pub id: SkillId,
    /// Stable kebab-case slug used to invoke the skill (e.g. `/pdf-processing` in chat). Safe to render in user-facing messages.
    #[schema(example = "pdf-processing")]
    pub name: String,
    /// Short, agent- and user-readable summary of what the skill does and when to use it.
    #[schema(example = "Extract text and tables from PDF files.")]
    pub description: String,
    /// License string as declared by the skill author (e.g. `MIT`, `Apache-2.0`). Informational; not enforced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Compatibility marker describing host-runtime requirements declared by the skill (e.g. min platform version). Informational.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    /// Free-form metadata declared by the skill author.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
    /// Comma-separated list of tool patterns this skill may invoke. `None` means inherit from the harness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<String>,
    /// How the skill content is sourced (filesystem, URL, embedded). Determines reload semantics.
    pub source_type: SkillSourceType,
    /// Current lifecycle status (`active`, `archived`, `deleted`).
    pub status: SkillStatus,
    /// Semver string declared by the skill author. Free-form; sorted lexicographically when comparing.
    pub version: String,
    /// Whether this skill appears as a `/`-prefixed slash command for end users in chat UIs.
    #[serde(default = "default_true")]
    pub user_invocable: bool,
    /// When `true`, the LLM is prevented from auto-invoking this skill; only the user can trigger it explicitly.
    #[serde(default)]
    pub disable_model_invocation: bool,
    /// Timestamp when this skill was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this skill was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
    /// Timestamp when this skill was archived, if any (RFC 3339). Archived skills are hidden from default list views.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Timestamp when this skill was hard-deleted, if any (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Number of agents and harnesses that reference a skill via its
/// `skill:{uuid}` capability id. The `/v1/skills/usage` endpoint returns this
/// keyed by public `SkillId`; skills with no references are omitted from the
/// map and the UI defaults missing entries to zero.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct SkillUsage {
    pub agents: u64,
    pub harnesses: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skill_wire_values_and_unknown_string_fallbacks_are_explicit() {
        for (value, wire) in [
            (SkillSourceType::Markdown, "markdown"),
            (SkillSourceType::Archive, "archive"),
        ] {
            assert_eq!(value.to_string(), wire);
            assert_eq!(
                serde_json::to_value(&value).unwrap(),
                serde_json::json!(wire)
            );
            assert_eq!(
                serde_json::from_value::<SkillSourceType>(serde_json::json!(wire)).unwrap(),
                value
            );
            assert_eq!(SkillSourceType::from(wire), value);
        }
        for (value, wire) in [
            (SkillStatus::Active, "active"),
            (SkillStatus::Disabled, "disabled"),
            (SkillStatus::Archived, "archived"),
            (SkillStatus::Deleted, "deleted"),
        ] {
            assert_eq!(value.to_string(), wire);
            assert_eq!(
                serde_json::to_value(&value).unwrap(),
                serde_json::json!(wire)
            );
            assert_eq!(
                serde_json::from_value::<SkillStatus>(serde_json::json!(wire)).unwrap(),
                value
            );
            assert_eq!(SkillStatus::from(wire), value);
        }
        assert_eq!(SkillSourceType::from("other"), SkillSourceType::Markdown);
        assert_eq!(SkillStatus::from("other"), SkillStatus::Active);
        assert!(serde_json::from_str::<SkillSourceType>("\"other\"").is_err());
        assert!(serde_json::from_str::<SkillStatus>("\"other\"").is_err());
    }
}

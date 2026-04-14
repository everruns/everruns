// Skill query helpers — shared by commands and other domains.
//
// No policy checks, no input validation. Pure data access + mapping.

use crate::storage::models::SkillRow;
use everruns_core::{Skill, SkillSourceType, SkillStatus};
use std::collections::HashMap;

// ============================================================================
// Row mapping
// ============================================================================

pub fn row_to_skill(row: &SkillRow) -> Skill {
    let metadata: HashMap<String, serde_json::Value> =
        serde_json::from_value(row.metadata.clone()).unwrap_or_default();

    // user_invocable defaults to true; only false if explicitly set in metadata
    let user_invocable = metadata
        .get("user_invocable")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // disable_model_invocation defaults to false; only true if explicitly set
    let disable_model_invocation = metadata
        .get("disable_model_invocation")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    Skill {
        id: row.id,
        name: row.name.clone(),
        description: row.description.clone(),
        license: row.license.clone(),
        compatibility: row.compatibility.clone(),
        metadata,
        allowed_tools: row.allowed_tools.clone(),
        source_type: SkillSourceType::from(row.source_type.as_str()),
        status: SkillStatus::from(row.status.as_str()),
        version: row.version.clone(),
        user_invocable,
        disable_model_invocation,
        created_at: row.created_at,
        updated_at: row.updated_at,
        archived_at: row.archived_at,
        deleted_at: row.deleted_at,
    }
}

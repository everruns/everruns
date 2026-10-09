// Rows the skills repository reads and writes.

use crate::kernel_imports::contracts::typed_id::SkillId;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Skill row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct SkillRow {
    pub id: SkillId,
    pub public_id: String,
    pub org_id: i64,
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: sqlx::types::JsonValue,
    pub allowed_tools: Option<String>,
    pub instructions: String,
    pub source_type: String,
    pub archive_data: Option<Vec<u8>>,
    pub status: String,
    pub version: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Input for creating a skill
#[derive(Debug, Clone)]
pub struct CreateSkillRow {
    pub public_id: String,
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: serde_json::Value,
    pub allowed_tools: Option<String>,
    pub instructions: String,
    pub source_type: String,
    pub archive_data: Option<Vec<u8>>,
    pub version: String,
}

/// Input for updating a skill
#[derive(Debug, Clone, Default)]
pub struct UpdateSkill {
    pub name: Option<String>,
    pub description: Option<String>,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub allowed_tools: Option<String>,
    pub instructions: Option<String>,
    pub status: Option<String>,
    pub version: Option<String>,
    pub archive_data: Option<Vec<u8>>,
    pub source_type: Option<String>,
}

/// Skill file row from database (extracted archive files)
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct SkillFileRow {
    pub id: Uuid,
    pub skill_id: Uuid,
    pub path: String,
    pub content: Option<String>,
    pub content_binary: Option<Vec<u8>>,
    pub is_binary: bool,
    pub size_bytes: i64,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a skill file
#[derive(Debug, Clone)]
pub struct CreateSkillFileRow {
    pub skill_id: Uuid,
    pub path: String,
    pub content: Option<String>,
    pub content_binary: Option<Vec<u8>>,
    pub is_binary: bool,
    pub size_bytes: i64,
}

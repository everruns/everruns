// Skill commands — user-facing operations.
//
// Each struct is the request type, catalog entry, and execution logic.
// inventory::submit! auto-registers for MCP catalog.
//
// Design note: SkillService owns a moka cache for list operations.
// Commands that benefit from the cache (ListSkills) delegate to the
// service. Direct DB operations use ctx.db.

use super::queries as q;
use super::types::{CreateSkillRequest, CreateSkillRow, UpdateSkill, UpdateSkillRequest};
use crate::domains::common::*;
use crate::services::skill::{SKILL_DANGEROUS, SKILL_MANAGE, SKILL_VIEW};
use everruns_core::{Policy, Skill, SkillContent, SkillFileEntry, SkillId, parse_skill_md};
use serde::Deserialize;
use std::collections::HashMap;

// ============================================================================
// CreateSkill
// ============================================================================

/// Create a new skill from SKILL.md content.
#[derive(Debug, Deserialize)]
pub struct CreateSkill(pub CreateSkillRequest);

impl Command for CreateSkill {
    type Output = Skill;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_skill",
            category: "skills",
            description: "Create a new skill from SKILL.md content.",
            method: "POST",
            path: "/v1/skills",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&SKILL_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Skill, CommandError> {
        let req = self.0;

        let parsed = parse_skill_md(&req.skill_md).map_err(|errors| {
            CommandError::bad_request(format!("Invalid SKILL.md: {}", errors.join("; ")))
        })?;

        // Check duplicate name
        if ctx
            .db
            .get_skill_by_name(ctx.org_id(), &parsed.name)
            .await
            .map_err(classify_anyhow)?
            .is_some()
        {
            return Err(CommandError::conflict(format!(
                "Skill with name '{}' already exists",
                parsed.name
            )));
        }

        let public_id = SkillId::new().to_string();
        let mut metadata_map = parsed.metadata.clone();
        if !parsed.user_invocable {
            metadata_map.insert("user_invocable".to_string(), serde_json::Value::Bool(false));
        }
        if parsed.disable_model_invocation {
            metadata_map.insert(
                "disable_model_invocation".to_string(),
                serde_json::Value::Bool(true),
            );
        }
        let metadata =
            serde_json::to_value(&metadata_map).map_err(|e| CommandError::Internal(e.into()))?;

        let input = CreateSkillRow {
            public_id,
            name: parsed.name,
            description: parsed.description,
            license: parsed.license,
            compatibility: parsed.compatibility,
            metadata,
            allowed_tools: parsed.allowed_tools,
            instructions: parsed.instructions,
            source_type: "markdown".to_string(),
            archive_data: None,
            version: parsed.version,
        };

        let row = ctx
            .db
            .create_skill(ctx.org_id(), input)
            .await
            .map_err(classify_anyhow)?;
        Ok(q::row_to_skill(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<CreateSkill>() }

// ============================================================================
// ListSkills
// ============================================================================

/// List skills. Supports search and include_archived.
#[derive(Debug, Deserialize)]
pub struct ListSkills {
    pub search: Option<String>,
    #[serde(default)]
    pub include_archived: bool,
}

impl Command for ListSkills {
    type Output = Vec<Skill>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_skills",
            category: "skills",
            description: "List all active skills. Use search for name search, include_archived=true to include archived.",
            method: "GET",
            path: "/v1/skills",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&SKILL_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Skill>, CommandError> {
        let rows = ctx
            .db
            .list_skills(ctx.org_id(), self.search.as_deref(), self.include_archived)
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.iter().map(q::row_to_skill).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListSkills>() }

// ============================================================================
// GetSkill
// ============================================================================

/// Get a single skill by ID.
#[derive(Debug, Deserialize)]
pub struct GetSkill {
    pub id: String,
}

impl Command for GetSkill {
    type Output = Skill;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_skill",
            category: "skills",
            description: "Get a single skill by ID.",
            method: "GET",
            path: "/v1/skills/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&SKILL_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Skill, CommandError> {
        let skill_id: SkillId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid skill ID: {e}")))?;

        let row = ctx
            .db
            .get_skill(ctx.org_id(), skill_id.uuid())
            .await
            .map_err(classify_anyhow)?
            .filter(|r| r.status != "deleted")
            .ok_or_else(|| CommandError::not_found("Skill"))?;

        Ok(q::row_to_skill(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<GetSkill>() }

// ============================================================================
// GetSkillContent
// ============================================================================

/// Get full skill content (SKILL.md + files).
#[derive(Debug, Deserialize)]
pub struct GetSkillContent {
    pub id: String,
}

impl Command for GetSkillContent {
    type Output = SkillContent;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_skill_content",
            category: "skills",
            description: "Get full skill content (SKILL.md + files).",
            method: "GET",
            path: "/v1/skills/{id}/content",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&SKILL_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SkillContent, CommandError> {
        let skill_id: SkillId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid skill ID: {e}")))?;

        let row = ctx
            .db
            .get_skill(ctx.org_id(), skill_id.uuid())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Skill"))?;

        // Reconstruct SKILL.md with frontmatter
        let metadata: HashMap<String, serde_json::Value> =
            serde_json::from_value(row.metadata.clone()).unwrap_or_default();
        let mut frontmatter = format!("---\nname: {}\ndescription: {}", row.name, row.description);
        if let Some(ref license) = row.license {
            frontmatter.push_str(&format!("\nlicense: {license}"));
        }
        if let Some(ref compat) = row.compatibility {
            frontmatter.push_str(&format!("\ncompatibility: {compat}"));
        }
        if !metadata.is_empty() {
            frontmatter.push_str("\nmetadata:");
            for (k, v) in &metadata {
                frontmatter.push_str(&format!("\n  {k}: {v}"));
            }
        }
        if let Some(ref tools) = row.allowed_tools {
            frontmatter.push_str(&format!("\nallowed-tools: {tools}"));
        }
        frontmatter.push_str("\n---\n\n");
        let skill_md = format!("{frontmatter}{}", row.instructions);

        // Get files for archive-based skills
        let files = if row.source_type == "archive" {
            let file_rows = ctx
                .db
                .list_skill_files(row.id.uuid())
                .await
                .map_err(classify_anyhow)?;
            file_rows
                .into_iter()
                .filter_map(|f| {
                    if f.is_binary {
                        None // Skip binary files in content response
                    } else {
                        Some(SkillFileEntry {
                            path: f.path,
                            content: f.content.unwrap_or_default(),
                        })
                    }
                })
                .collect()
        } else {
            vec![]
        };

        Ok(SkillContent { skill_md, files })
    }
}

inventory::submit! { CommandDescriptor::of::<GetSkillContent>() }

// ============================================================================
// UpdateSkill
// ============================================================================

/// Update a skill. Only provided fields are changed.
#[derive(Debug, Deserialize)]
pub struct UpdateSkillCmd {
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateSkillRequest,
}

impl Command for UpdateSkillCmd {
    type Output = Skill;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_skill",
            category: "skills",
            description: "Update a skill. Only provided fields are changed.",
            method: "PATCH",
            path: "/v1/skills/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&SKILL_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Skill, CommandError> {
        let skill_id: SkillId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid skill ID: {e}")))?;

        let req = self.req;
        let id = skill_id.uuid();

        // Check existing status
        if let Some(existing) = ctx
            .db
            .get_skill(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            && !matches!(existing.status.as_str(), "active" | "disabled")
        {
            return Err(CommandError::bad_request(
                "Archived or deleted skills cannot be edited",
            ));
        }

        let mut input = UpdateSkill::default();

        // If skill_md is provided, re-parse it
        if let Some(ref skill_md) = req.skill_md {
            let parsed = parse_skill_md(skill_md).map_err(|errors| {
                CommandError::bad_request(format!("Invalid SKILL.md: {}", errors.join("; ")))
            })?;

            // Check name uniqueness if name changed
            if let Some(existing) = ctx
                .db
                .get_skill(ctx.org_id(), id)
                .await
                .map_err(classify_anyhow)?
                && existing.name != parsed.name
                && ctx
                    .db
                    .get_skill_by_name(ctx.org_id(), &parsed.name)
                    .await
                    .map_err(classify_anyhow)?
                    .is_some()
            {
                return Err(CommandError::conflict(format!(
                    "Skill with name '{}' already exists",
                    parsed.name
                )));
            }

            input.name = Some(parsed.name);
            input.description = Some(parsed.description);
            input.license = parsed.license;
            input.compatibility = parsed.compatibility;
            input.metadata = Some(
                serde_json::to_value(&parsed.metadata)
                    .map_err(|e| CommandError::Internal(e.into()))?,
            );
            input.allowed_tools = parsed.allowed_tools;
            input.instructions = Some(parsed.instructions);
            input.version = Some(parsed.version);
        }

        if let Some(status) = &req.status {
            input.status = Some(status.to_string());
        }

        let row = ctx
            .db
            .update_skill(ctx.org_id(), id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Skill"))?;

        Ok(q::row_to_skill(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateSkillCmd>() }

// ============================================================================
// DeleteSkill
// ============================================================================

/// Archive a skill (soft delete).
#[derive(Debug, Deserialize)]
pub struct DeleteSkill {
    pub id: String,
}

impl Command for DeleteSkill {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_skill",
            category: "skills",
            description: "Archive a skill (soft delete). Can be restored.",
            method: "DELETE",
            path: "/v1/skills/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&SKILL_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let skill_id: SkillId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid skill ID: {e}")))?;

        let deleted = ctx
            .db
            .delete_skill(ctx.org_id(), skill_id.uuid())
            .await
            .map_err(classify_anyhow)?;

        if !deleted {
            return Err(CommandError::not_found("Skill"));
        }

        Ok(serde_json::json!({"deleted": true}))
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteSkill>() }

// ============================================================================
// DestroySkill (hard delete)
// ============================================================================

/// Permanently delete an archived skill.
#[derive(Debug, Deserialize)]
pub struct DestroySkill {
    pub id: String,
}

impl Command for DestroySkill {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "destroy_skill",
            category: "skills",
            description: "Permanently delete an archived skill.",
            method: "POST",
            path: "/v1/skills/{id}/delete",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&SKILL_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let skill_id: SkillId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid skill ID: {e}")))?;

        let id = skill_id.uuid();

        let existing = ctx
            .db
            .get_skill(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Skill"))?;

        if existing.status != "archived" {
            return Err(CommandError::bad_request(
                "Skill must be archived before deletion",
            ));
        }

        ctx.db
            .destroy_skill(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?;

        Ok(serde_json::json!({"destroyed": true}))
    }
}

inventory::submit! { CommandDescriptor::of::<DestroySkill>() }

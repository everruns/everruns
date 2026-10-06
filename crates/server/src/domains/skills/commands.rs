// Skill commands — user-facing operations.
//
// Each struct is the request type, catalog entry, and execution logic.
// `#[command]` registers it for MCP, gRPC and the CLI contract and, with
// `http = ..`, serves it over REST at its declared path.
//
// Note: SkillService retains a moka cache for list operations used by
// the capability registry. Domain commands read from DB directly.

use super::queries as q;
use super::types::{CreateSkillRequest, CreateSkillRow, UpdateSkill, UpdateSkillRequest};
use super::{SKILL_DANGEROUS, SKILL_MANAGE, SKILL_VIEW};
use crate::domains::common::*;
use crate::kernel_imports::{
    SKILL_CAPABILITY_PREFIX, Skill, SkillContent, SkillFileEntry, SkillStatus, SkillUsage,
    contracts::typed_id::SkillId, parse_skill_md,
};
use serde::Deserialize;
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};

// ============================================================================
// CreateSkill
// ============================================================================

/// Create a new skill from SKILL.md content.
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct CreateSkill(pub CreateSkillRequest);

impl CommandSchema for CreateSkill {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateSkillRequest>()
    }
}

#[command(
    name = "create_skill",
    category = "skills",
    description = "Create a new skill from SKILL.md content.",
    method = "POST",
    path = "/v1/skills",
    policy = SKILL_MANAGE,
    cli = CliRoute::new(&["skills"], "create").with_examples(&[CliExample::new("Add a skill from a local file", "everruns skills create --skill-md \"$(cat SKILL.md)\"",)]),
    http = created_with_urls,
    request_body(CreateSkillRequest),
    responses(
        (status = 409, description = "Duplicate skill name", body = crate::api::common::ErrorResponse),
        (status = 422, description = "Invalid SKILL.md", body = crate::api::common::ErrorResponse),
    ),
)]
impl Command for CreateSkill {
    type Output = Skill;

    async fn execute(self, ctx: &Ctx) -> Result<Skill, CommandError> {
        let req = self.0;

        // Malformed SKILL.md is a validation failure (422), matching the
        // documented OpenAPI contract for this endpoint.
        let parsed = parse_skill_md(&req.skill_md).map_err(|errors| {
            CommandError::unprocessable(format!("Invalid SKILL.md: {}", errors.join("; ")))
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
            serde_json::to_value(&metadata_map).map_err(|e| CommandError::internal(e.into()))?;

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
        ctx.capability_service
            .invalidate_skills_cache(ctx.org_id())
            .await;
        Ok(q::row_to_skill(&row))
    }
}

// ============================================================================
// ListSkills
// ============================================================================

/// List skills. Supports search and include_archived.
#[derive(Debug, Deserialize, ToSchema, IntoParams, serde::Serialize)]
#[into_params(parameter_in = Query)]
pub struct ListSkills {
    /// Search by name or description (case-insensitive substring match).
    pub search: Option<String>,
    /// Include archived skills. Deleted skills never appear in lists.
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub include_archived: bool,
}

#[command(
    name = "list_skills",
    category = "skills",
    description = "List all active skills. Use search for name search, include_archived=true to include archived.",
    method = "GET",
    path = "/v1/skills",
    policy = SKILL_VIEW,
    cli = CliRoute::new(&["skills"], "list").with_examples(&[CliExample::new("Find skills by name when you do not know the id", "everruns skills list --search code-review",)]),
    http = list_with_urls,
    params(ListSkills),
)]
impl Command for ListSkills {
    type Output = Vec<Skill>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Skill>, CommandError> {
        let rows = ctx
            .db
            .list_skills(ctx.org_id(), self.search.as_deref(), self.include_archived)
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.iter().map(q::row_to_skill).collect())
    }
}

// ============================================================================
// GetSkill
// ============================================================================

/// Get a single skill by ID.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetSkill {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "get_skill",
    category = "skills",
    description = "Get a single skill by ID.",
    method = "GET",
    path = "/v1/skills/{id}",
    policy = SKILL_VIEW,
    positional = "id",
    cli = CliRoute::new(&["skills"], "get") .with_args(&[CliArg::new("id").at(1)]) .with_examples(&[CliExample::new("Show one skill's metadata without its body", "everruns skills get skl_01h9",)]),
    http = with_urls,
    responses((status = 404, description = "Skill not found")),
)]
impl Command for GetSkill {
    type Output = Skill;

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

// ============================================================================
// GetSkillContent
// ============================================================================

/// Get full skill content (SKILL.md + files).
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetSkillContent {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "get_skill_content",
    category = "skills",
    description = "Get full skill content (SKILL.md + files).",
    method = "GET",
    path = "/v1/skills/{id}/content",
    policy = SKILL_VIEW,
    positional = "id",
    cli = CliRoute::new(&["skills"], "content") .with_args(&[CliArg::new("id").at(1)]) .with_examples(&[CliExample::new("Read a skill's body to see what it instructs", "everruns skills content skl_01h9",)]),
    http = plain,
    responses((status = 404, description = "Skill not found")),
)]
impl Command for GetSkillContent {
    type Output = SkillContent;

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
            .filter(|r| r.status != "deleted")
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

// ============================================================================
// UpdateSkill
// ============================================================================

/// Update a skill. Only provided fields are changed.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateSkillCmd {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateSkillRequest,
}

#[command(
    name = "update_skill",
    category = "skills",
    description = "Update a skill. Only provided fields are changed.",
    method = "PATCH",
    path = "/v1/skills/{id}",
    policy = SKILL_MANAGE,
    positional = "id",
    cli = CliRoute::new(&["skills"], "update") .with_args(&[CliArg::new("id").at(1)]) .with_examples(&[CliExample::new("Replace a skill's content from a local file", "everruns skills update skl_01h9 --skill-md \"$(cat SKILL.md)\"",)]),
    http = with_urls,
    request_body(UpdateSkillRequest),
    responses(
        (status = 404, description = "Skill not found"),
        (status = 409, description = "Duplicate skill name", body = crate::api::common::ErrorResponse),
        (status = 422, description = "Invalid SKILL.md", body = crate::api::common::ErrorResponse),
    ),
)]
impl Command for UpdateSkillCmd {
    type Output = Skill;

    async fn execute(self, ctx: &Ctx) -> Result<Skill, CommandError> {
        let skill_id: SkillId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid skill ID: {e}")))?;

        let req = self.req;
        let id = skill_id.uuid();
        if matches!(req.status, Some(SkillStatus::Deleted)) {
            return Err(CommandError::forbidden(
                "Setting status=deleted requires dangerous delete permission".to_string(),
            ));
        }

        // Check existing status. Archiving is documented as restorable
        // (see DeleteSkill), so archived skills accept a status-only PATCH
        // (the restore path) but no content edits; deleted skills stay
        // immutable.
        if let Some(existing) = ctx
            .db
            .get_skill(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
        {
            match existing.status.as_str() {
                "active" | "disabled" => {}
                "archived" => {
                    if req.skill_md.is_some() || req.status.is_none() {
                        return Err(CommandError::bad_request(
                            "Archived skills only accept a status-only update (restore); \
                             restore the skill before editing its content",
                        ));
                    }
                }
                _ => {
                    return Err(CommandError::bad_request("Deleted skills cannot be edited"));
                }
            }
        }

        let mut input = UpdateSkill::default();

        // If skill_md is provided, re-parse it
        if let Some(ref skill_md) = req.skill_md {
            let parsed = parse_skill_md(skill_md).map_err(|errors| {
                CommandError::unprocessable(format!("Invalid SKILL.md: {}", errors.join("; ")))
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

            input.name = Some(parsed.name);
            input.description = Some(parsed.description);
            input.license = parsed.license;
            input.compatibility = parsed.compatibility;
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
            input.metadata = Some(
                serde_json::to_value(&metadata_map)
                    .map_err(|e| CommandError::internal(e.into()))?,
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

        ctx.capability_service
            .invalidate_skills_cache(ctx.org_id())
            .await;
        Ok(q::row_to_skill(&row))
    }
}

// ============================================================================
// DeleteSkill
// ============================================================================

/// Archive a skill (soft delete).
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteSkill {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "delete_skill",
    category = "skills",
    description = "Archive a skill (soft delete). Can be restored.",
    method = "DELETE",
    path = "/v1/skills/{id}",
    policy = SKILL_MANAGE,
    positional = "id",
    cli = CliRoute::new(&["skills"], "delete") .with_args(&[CliArg::new("id").at(1)]) .with_examples(&[CliExample::new("Archive a skill, keeping it restorable", "everruns skills delete skl_01h9",)]),
    http = no_content,
    responses((status = 404, description = "Skill not found")),
)]
impl Command for DeleteSkill {
    type Output = serde_json::Value;

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

        ctx.capability_service
            .invalidate_skills_cache(ctx.org_id())
            .await;
        Ok(serde_json::json!({"deleted": true}))
    }
}

// ============================================================================
// DestroySkill (hard delete)
// ============================================================================

/// Permanently delete an archived skill.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DestroySkill {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "destroy_skill",
    category = "skills",
    description = "Permanently delete an archived skill.",
    method = "POST",
    path = "/v1/skills/{id}/delete",
    policy = SKILL_DANGEROUS,
    positional = "id",
    cli = CliRoute::new(&["skills"], "destroy") .with_args(&[CliArg::new("id").at(1)]) .with_examples(&[CliExample::new("Permanently remove an already-archived skill", "everruns skills destroy skl_01h9",)]),
    http = no_content,
    responses(
        (status = 400, description = "Skill is not archived", body = crate::api::common::ErrorResponse),
        (status = 404, description = "Skill not found"),
    ),
)]
impl Command for DestroySkill {
    type Output = serde_json::Value;

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

        ctx.capability_service
            .invalidate_skills_cache(ctx.org_id())
            .await;
        Ok(serde_json::json!({"destroyed": true}))
    }
}

// ============================================================================
// ListSkillsUsage
// ============================================================================

/// List skill usage counts: how many active agents and harnesses reference each
/// skill via its `skill:{uuid}` capability id. Returned map is keyed by the
/// public SkillId string (e.g. `skill_<32hex>`). Skills with zero references
/// are omitted; the UI defaults missing entries to zero.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListSkillsUsage {}

#[command(
    name = "list_skills_usage",
    category = "skills",
    description = "Count agents and harnesses referencing each skill capability.",
    method = "GET",
    path = "/v1/skills/usage",
    policy = SKILL_VIEW,
    cli = CliRoute::new(&["skills"], "usage").with_examples(&[CliExample::new("See which agents use which skills", "everruns skills usage",)]),
    http = plain,
)]
impl Command for ListSkillsUsage {
    type Output = HashMap<String, SkillUsage>;

    async fn execute(self, ctx: &Ctx) -> Result<HashMap<String, SkillUsage>, CommandError> {
        let (visible_skill_ids, agent_counts, harness_counts) = tokio::try_join!(
            ctx.db.list_non_deleted_skill_ids(ctx.org_id()),
            ctx.db.count_agent_capability_references(ctx.org_id()),
            ctx.db.count_harness_capability_references(ctx.org_id()),
        )
        .map_err(classify_anyhow)?;
        let visible_skill_ids = visible_skill_ids
            .into_iter()
            .collect::<std::collections::HashSet<_>>();

        let mut usage: HashMap<String, SkillUsage> = HashMap::new();
        for (cap_id, count) in agent_counts {
            if let Some(uuid_str) = cap_id.strip_prefix(SKILL_CAPABILITY_PREFIX)
                && let Ok(uuid) = uuid::Uuid::parse_str(uuid_str)
                && visible_skill_ids.contains(&uuid)
            {
                let key = SkillId::from_uuid(uuid).to_string();
                usage.entry(key).or_default().agents = count;
            }
        }
        for (cap_id, count) in harness_counts {
            if let Some(uuid_str) = cap_id.strip_prefix(SKILL_CAPABILITY_PREFIX)
                && let Ok(uuid) = uuid::Uuid::parse_str(uuid_str)
                && visible_skill_ids.contains(&uuid)
            {
                let key = SkillId::from_uuid(uuid).to_string();
                usage.entry(key).or_default().harnesses = count;
            }
        }
        Ok(usage)
    }
}

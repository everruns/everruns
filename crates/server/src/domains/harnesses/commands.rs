// Harness commands — user-facing operations.
//
// Each struct is the request type, catalog entry, and execution logic.
// inventory::submit! auto-registers for MCP catalog.

use super::queries as q;
use super::types::{CreateHarnessRequest, CreateHarnessRow, UpdateHarness, UpdateHarnessRequest};
use crate::domains::common::*;
use crate::services::harness::{HARNESS_DANGEROUS, HARNESS_MANAGE, HARNESS_VIEW};
use everruns_core::{AgentCapabilityConfig, Harness, HarnessId, Policy, ToolDefinition};
use serde::Deserialize;

// ============================================================================
// Input validation
// ============================================================================

use crate::api::validation::{
    MAX_AGENT_CAPABILITIES, MAX_AGENT_DESCRIPTION_BYTES, MAX_AGENT_NAME_BYTES,
    MAX_AGENT_SYSTEM_PROMPT_BYTES, MAX_INITIAL_FILES, MAX_INITIAL_FILES_TOTAL_BYTES,
};
use everruns_core::InitialFile;

fn validate_create_limits(req: &CreateHarnessRequest) -> Result<(), CommandError> {
    if req.name.len() > MAX_AGENT_NAME_BYTES
        || req
            .display_name
            .as_ref()
            .is_some_and(|d| d.len() > MAX_AGENT_NAME_BYTES)
        || req
            .description
            .as_ref()
            .is_some_and(|d| d.len() > MAX_AGENT_DESCRIPTION_BYTES)
        || req.system_prompt.len() > MAX_AGENT_SYSTEM_PROMPT_BYTES
        || req.capabilities.len() > MAX_AGENT_CAPABILITIES
        || req.initial_files.len() > MAX_INITIAL_FILES
        || initial_files_total_bytes(&req.initial_files) > MAX_INITIAL_FILES_TOTAL_BYTES
    {
        return Err(CommandError::bad_request("Input exceeds allowed limits"));
    }
    Ok(())
}

fn validate_update_limits(req: &UpdateHarnessRequest) -> Result<(), CommandError> {
    if req
        .display_name
        .as_ref()
        .is_some_and(|d| d.len() > MAX_AGENT_NAME_BYTES)
        || req
            .description
            .as_ref()
            .is_some_and(|d| d.len() > MAX_AGENT_DESCRIPTION_BYTES)
        || req
            .system_prompt
            .as_ref()
            .is_some_and(|s| s.len() > MAX_AGENT_SYSTEM_PROMPT_BYTES)
        || req
            .capabilities
            .as_ref()
            .is_some_and(|c| c.len() > MAX_AGENT_CAPABILITIES)
        || req
            .initial_files
            .as_ref()
            .is_some_and(|f| f.len() > MAX_INITIAL_FILES)
        || req
            .initial_files
            .as_ref()
            .is_some_and(|f| initial_files_total_bytes(f) > MAX_INITIAL_FILES_TOTAL_BYTES)
    {
        return Err(CommandError::bad_request("Input exceeds allowed limits"));
    }
    Ok(())
}

fn initial_files_total_bytes(files: &[InitialFile]) -> usize {
    files.iter().map(|f| f.content.len()).sum()
}

// ============================================================================
// Shared persistence helpers
// ============================================================================

async fn persist_capabilities(
    db: &crate::storage::StorageBackend,
    harness_uuid: uuid::Uuid,
    caps: &[AgentCapabilityConfig],
) -> Result<(), CommandError> {
    if !caps.is_empty() {
        db.set_harness_capabilities(harness_uuid, q::cap_tuples(caps))
            .await?;
    }
    Ok(())
}

// ============================================================================
// CreateHarness
// ============================================================================

/// Create a new harness with a name, system prompt, and optional capabilities.
#[derive(Debug, Deserialize)]
pub struct CreateHarness(pub CreateHarnessRequest);

impl Command for CreateHarness {
    type Output = Harness;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_harness",
            category: "harnesses",
            description: "Create a new harness with a name, system prompt, and optional capabilities.",
            method: "POST",
            path: "/v1/harnesses",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&HARNESS_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Harness, CommandError> {
        let req = self.0;

        // Validate
        q::validate_harness_name(&req.name)?;
        validate_create_limits(&req)?;

        // Business rules
        q::ensure_name_available(&ctx.db, ctx.org_id(), &req.name, None).await?;
        let caps = q::ensure_file_system_capability(
            req.capabilities.clone(),
            !req.initial_files.is_empty(),
        );
        crate::services::capability_validation::validate_capability_refs(
            &ctx.db,
            ctx.org_id(),
            &caps,
        )
        .await
        .map_err(classify_anyhow)?;
        let parent_harness_id =
            q::validate_parent_harness(&ctx.db, ctx.org_id(), None, req.parent_harness_id)
                .await
                .map_err(classify_anyhow)?;
        let default_model_id = q::validate_model_id(&ctx.db, ctx.org_id(), req.default_model_id)
            .await
            .map_err(classify_anyhow)?;

        // Persist
        let input = CreateHarnessRow {
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            system_prompt: req.system_prompt,
            parent_harness_id,
            default_model_id,
            tags: req.tags,
            initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
            network_access: req
                .network_access
                .as_ref()
                .map(|na| serde_json::to_value(na).unwrap()),
            is_built_in: false,
        };
        let row = ctx
            .db
            .create_harness(ctx.org_id(), input)
            .await
            .map_err(classify_anyhow)?;
        let harness_uuid = row.id.uuid();

        persist_capabilities(&ctx.db, harness_uuid, &caps).await?;
        Ok(q::row_to_harness(row, caps))
    }
}

inventory::submit! { CommandDescriptor::of::<CreateHarness>() }

// ============================================================================
// ListHarnesses
// ============================================================================

/// List harnesses. Supports search and include_archived.
#[derive(Debug, Deserialize)]
pub struct ListHarnesses {
    pub search: Option<String>,
    #[serde(default)]
    pub include_archived: bool,
}

impl Command for ListHarnesses {
    type Output = Vec<Harness>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_harnesses",
            category: "harnesses",
            description: "List all active harnesses. Use search for name search, include_archived=true to include archived.",
            method: "GET",
            path: "/v1/harnesses",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&HARNESS_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Harness>, CommandError> {
        let rows = ctx
            .db
            .list_harnesses(ctx.org_id(), self.search.as_deref(), self.include_archived)
            .await
            .map_err(classify_anyhow)?;
        q::load_harnesses_list(&ctx.db, rows)
            .await
            .map_err(classify_anyhow)
    }
}

inventory::submit! { CommandDescriptor::of::<ListHarnesses>() }

// ============================================================================
// GetHarness
// ============================================================================

/// Get a single harness by ID or name.
#[derive(Debug, Deserialize)]
pub struct GetHarness {
    pub id: String,
}

impl Command for GetHarness {
    type Output = Harness;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_harness",
            category: "harnesses",
            description: "Get a single harness by ID or name.",
            method: "GET",
            path: "/v1/harnesses/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&HARNESS_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Harness, CommandError> {
        q::resolve(&ctx.db, ctx.org_id(), &self.id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Harness"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetHarness>() }

// ============================================================================
// UpdateHarness
// ============================================================================

/// Update a harness. Only provided fields are changed.
#[derive(Debug, Deserialize)]
pub struct UpdateHarnessCmd {
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateHarnessRequest,
}

impl Command for UpdateHarnessCmd {
    type Output = Harness;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_harness",
            category: "harnesses",
            description: "Update a harness. Only provided fields are changed.",
            method: "PATCH",
            path: "/v1/harnesses/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&HARNESS_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Harness, CommandError> {
        let harness_id: HarnessId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid harness ID: {e}")))?;

        let req = self.req;

        if let Some(ref name) = req.name {
            q::validate_harness_name(name)?;
        }
        validate_update_limits(&req)?;

        // Reject updates to built-in harnesses
        if q::is_built_in(&ctx.db, ctx.org_id(), harness_id)
            .await
            .map_err(classify_anyhow)?
        {
            return Err(CommandError::bad_request(
                "Cannot modify built-in harness. Copy it first to create an editable version.",
            ));
        }

        // Resolve existing
        let existing = ctx
            .db
            .get_harness(ctx.org_id(), harness_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Harness"))?;

        if existing.status != "active" {
            return Err(CommandError::bad_request(
                "Archived or deleted harnesses cannot be edited",
            ));
        }

        if let Some(ref name) = req.name {
            q::ensure_name_available(&ctx.db, ctx.org_id(), name, Some(harness_id)).await?;
        }

        // Resolve capabilities
        let existing_initial_files: Vec<InitialFile> =
            serde_json::from_value(existing.initial_files.clone()).unwrap_or_default();
        let final_has_initial_files = req
            .initial_files
            .as_ref()
            .map(|f| !f.is_empty())
            .unwrap_or(!existing_initial_files.is_empty());

        let capabilities_override = match req.capabilities.clone() {
            Some(caps) => Some(q::ensure_file_system_capability(
                caps,
                final_has_initial_files,
            )),
            None if final_has_initial_files => Some(q::ensure_file_system_capability(
                q::get_capabilities(&ctx.db, harness_id.uuid())
                    .await
                    .map_err(classify_anyhow)?,
                true,
            )),
            None => None,
        };
        if let Some(ref caps) = capabilities_override {
            crate::services::capability_validation::validate_capability_refs(
                &ctx.db,
                ctx.org_id(),
                caps,
            )
            .await
            .map_err(classify_anyhow)?;
        }
        let default_model_id = q::validate_model_id(&ctx.db, ctx.org_id(), req.default_model_id)
            .await
            .map_err(classify_anyhow)?;
        let parent_harness_id = q::validate_parent_harness(
            &ctx.db,
            ctx.org_id(),
            Some(harness_id),
            req.parent_harness_id.flatten(),
        )
        .await
        .map_err(classify_anyhow)?;

        // Persist
        let input = UpdateHarness {
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            system_prompt: req.system_prompt,
            parent_harness_id: req.parent_harness_id.map(|_| parent_harness_id),
            default_model_id,
            tags: req.tags,
            initial_files: req
                .initial_files
                .map(|files| serde_json::to_value(&files).unwrap_or_default()),
            network_access: req
                .network_access
                .map(|na| Some(serde_json::to_value(na).unwrap())),
            status: req.status.map(|s| s.to_string()),
        };
        let row = ctx
            .db
            .update_harness(ctx.org_id(), harness_id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Harness"))?;

        let caps = if let Some(caps) = capabilities_override {
            persist_capabilities(&ctx.db, harness_id.uuid(), &caps).await?;
            caps
        } else {
            q::get_capabilities(&ctx.db, harness_id.uuid())
                .await
                .map_err(classify_anyhow)?
        };

        Ok(q::row_to_harness(row, caps))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateHarnessCmd>() }

// ============================================================================
// DeleteHarness
// ============================================================================

/// Archive a harness (soft delete).
#[derive(Debug, Deserialize)]
pub struct DeleteHarness {
    pub id: String,
}

impl Command for DeleteHarness {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_harness",
            category: "harnesses",
            description: "Archive a harness (soft delete). Can be restored.",
            method: "DELETE",
            path: "/v1/harnesses/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&HARNESS_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let harness_id: HarnessId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid harness ID: {e}")))?;

        // Reject deletion of built-in harnesses
        if q::is_built_in(&ctx.db, ctx.org_id(), harness_id)
            .await
            .map_err(classify_anyhow)?
        {
            return Err(CommandError::bad_request("Cannot delete built-in harness."));
        }

        q::ensure_no_child_harnesses(&ctx.db, ctx.org_id(), harness_id)
            .await
            .map_err(classify_anyhow)?;

        let deleted = ctx
            .db
            .delete_harness(ctx.org_id(), harness_id)
            .await
            .map_err(classify_anyhow)?;

        if deleted {
            Ok(serde_json::json!({"deleted": true}))
        } else {
            Err(CommandError::not_found("Harness"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteHarness>() }

// ============================================================================
// DestroyHarness (hard delete)
// ============================================================================

/// Permanently delete an archived harness.
#[derive(Debug, Deserialize)]
pub struct DestroyHarness {
    pub id: String,
}

impl Command for DestroyHarness {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "destroy_harness",
            category: "harnesses",
            description: "Permanently delete an archived harness.",
            method: "POST",
            path: "/v1/harnesses/{id}/delete",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&HARNESS_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let harness_id: HarnessId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid harness ID: {e}")))?;

        // Reject deletion of built-in harnesses
        if q::is_built_in(&ctx.db, ctx.org_id(), harness_id)
            .await
            .map_err(classify_anyhow)?
        {
            return Err(CommandError::bad_request("Cannot delete built-in harness."));
        }

        q::ensure_no_child_harnesses(&ctx.db, ctx.org_id(), harness_id)
            .await
            .map_err(classify_anyhow)?;

        let existing = ctx
            .db
            .get_harness(ctx.org_id(), harness_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Harness"))?;

        if existing.status != "archived" {
            return Err(CommandError::bad_request(
                "Harness must be archived before deletion",
            ));
        }

        let destroyed = ctx
            .db
            .destroy_harness(ctx.org_id(), harness_id)
            .await
            .map_err(classify_anyhow)?;

        if destroyed {
            Ok(serde_json::json!({"destroyed": true}))
        } else {
            Err(CommandError::not_found("Harness"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DestroyHarness>() }

// ============================================================================
// CopyHarness
// ============================================================================

/// Copy a harness. Generates a unique name ({name}-copy, -copy-2, etc.)
#[derive(Debug, Deserialize)]
pub struct CopyHarness {
    pub id: String,
}

impl Command for CopyHarness {
    type Output = Harness;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "copy_harness",
            category: "harnesses",
            description: "Copy a harness. Generates a unique name.",
            method: "POST",
            path: "/v1/harnesses/{id}/copy",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&HARNESS_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Harness, CommandError> {
        let source = q::resolve(&ctx.db, ctx.org_id(), &self.id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Harness"))?;

        let copy_name =
            q::find_unique_name(&ctx.db, ctx.org_id(), &format!("{}-copy", source.name))
                .await
                .map_err(classify_anyhow)?;

        let req = CreateHarnessRequest {
            name: copy_name,
            display_name: source.display_name.as_ref().map(|d| format!("{d} (copy)")),
            description: source.description,
            system_prompt: source.system_prompt,
            parent_harness_id: source.parent_harness_id,
            default_model_id: source.default_model_id,
            tags: source.tags,
            capabilities: source.capabilities,
            initial_files: source.initial_files,
            network_access: None,
        };

        CreateHarness(req).execute(ctx).await
    }
}

inventory::submit! { CommandDescriptor::of::<CopyHarness>() }

// ============================================================================
// PreviewHarness
// ============================================================================

/// Preview the final harness shape with capabilities applied.
#[derive(Debug, Deserialize)]
pub struct PreviewHarness {
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub parent_harness_id: Option<HarnessId>,
    #[serde(default)]
    pub capabilities: Vec<AgentCapabilityConfig>,
}

#[derive(Debug, serde::Serialize)]
pub struct HarnessPreview {
    pub system_prompt: String,
    pub tools: Vec<ToolDefinition>,
}

impl Command for PreviewHarness {
    type Output = HarnessPreview;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "preview_harness",
            category: "harnesses",
            description: "Preview the final harness shape with capabilities applied.",
            method: "POST",
            path: "/v1/harnesses/preview",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<HarnessPreview, CommandError> {
        let parent = match self.parent_harness_id {
            Some(parent_id) => crate::services::harness::resolve_effective_harness(
                ctx.db.as_ref(),
                ctx.org_id(),
                parent_id,
            )
            .await
            .map_err(classify_anyhow)?,
            None => None,
        };
        let (system_prompt, capabilities) = crate::services::harness::merge_preview_layer(
            parent.as_ref(),
            &self.system_prompt.unwrap_or_default(),
            &self.capabilities,
        );
        let (system_prompt, tools) = ctx
            .capability_service
            .preview(ctx.org_id(), &system_prompt, &capabilities)
            .await
            .map_err(classify_anyhow)?;
        Ok(HarnessPreview {
            system_prompt,
            tools,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<PreviewHarness>() }

// ============================================================================
// CheckHarnessName
// ============================================================================

/// Check whether a harness name is available.
#[derive(Debug, Deserialize)]
pub struct CheckHarnessName {
    pub name: String,
    pub exclude_id: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct NameAvailability {
    pub available: bool,
}

impl Command for CheckHarnessName {
    type Output = NameAvailability;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "check_harness_name",
            category: "harnesses",
            description: "Check whether a harness name is available.",
            method: "GET",
            path: "/v1/harnesses/check-name",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&HARNESS_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<NameAvailability, CommandError> {
        // If name is invalid or reserved, it's not "available"
        if q::validate_harness_name(&self.name).is_err() {
            return Ok(NameAvailability { available: false });
        }

        let exclude_id = self
            .exclude_id
            .map(|id| {
                id.parse::<HarnessId>()
                    .map_err(|e| CommandError::bad_request(format!("Invalid exclude_id: {e}")))
            })
            .transpose()?;

        let existing = ctx
            .db
            .get_harness_by_name(ctx.org_id(), &self.name)
            .await
            .map_err(classify_anyhow)?;

        let available = match existing {
            Some(row) => exclude_id == Some(row.id),
            None => true,
        };

        Ok(NameAvailability { available })
    }
}

inventory::submit! { CommandDescriptor::of::<CheckHarnessName>() }

// Harness commands — user-facing operations.
//
// Each struct is the request type, catalog entry, and execution logic.
// inventory::submit! auto-registers for MCP catalog.

use super::queries as q;
use super::types::{CreateHarnessRequest, CreateHarnessRow, UpdateHarness, UpdateHarnessRequest};
use super::{HARNESS_DANGEROUS, HARNESS_MANAGE, HARNESS_VIEW};
use crate::domains::common::*;
use crate::kernel_imports::{
    AgentCapabilityConfig, Policy, ScopedMcpServers,
    everruns_provider::openresponses_types::{
        MAX_METADATA_KEY_LENGTH, MAX_METADATA_KEYS, MAX_METADATA_VALUE_LENGTH,
    },
    everruns_provider::tool_types::ToolDefinition,
    everruns_provider::typed_id::HarnessId,
    merge_scoped_mcp_servers,
};
use everruns_platform::{Harness, HarnessStatus};
use serde::Deserialize;
use utoipa::ToSchema;

// ============================================================================
// Input validation
// ============================================================================

use crate::api::validation::{
    MAX_AGENT_CAPABILITIES, MAX_AGENT_DESCRIPTION_BYTES, MAX_AGENT_NAME_BYTES,
    MAX_AGENT_SYSTEM_PROMPT_BYTES, MAX_INITIAL_FILES, MAX_INITIAL_FILES_TOTAL_BYTES,
};
use everruns_core::InitialFile;

const SYSTEM_LLM_METADATA_KEYS: &[&str] = &[
    "session_id",
    "harness_id",
    "turn_id",
    "exec_id",
    "org_id",
    "agent_id",
    "model_id",
];

const MAX_EMBEDDER_METADATA_KEYS: usize = MAX_METADATA_KEYS - SYSTEM_LLM_METADATA_KEYS.len();

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
        || req
            .system_prompt
            .as_ref()
            .is_some_and(|s| s.len() > MAX_AGENT_SYSTEM_PROMPT_BYTES)
        || req.capabilities.len() > MAX_AGENT_CAPABILITIES
        || req.initial_files.len() > MAX_INITIAL_FILES
        || initial_files_total_bytes(&req.initial_files) > MAX_INITIAL_FILES_TOTAL_BYTES
    {
        return Err(CommandError::bad_request("Input exceeds allowed limits"));
    }
    validate_embedder_metadata_limits(&req.embedder_metadata)?;
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
    if let Some(metadata) = &req.embedder_metadata {
        validate_embedder_metadata_limits(metadata)?;
    }
    Ok(())
}

fn validate_embedder_metadata_limits(
    metadata: &std::collections::HashMap<String, String>,
) -> Result<(), CommandError> {
    if metadata.len() > MAX_EMBEDDER_METADATA_KEYS
        || metadata.iter().any(|(key, value)| {
            key.len() > MAX_METADATA_KEY_LENGTH || value.len() > MAX_METADATA_VALUE_LENGTH
        })
    {
        return Err(CommandError::bad_request(
            "Embedder metadata exceeds allowed limits",
        ));
    }
    Ok(())
}

fn initial_files_total_bytes(files: &[InitialFile]) -> usize {
    files.iter().map(|f| f.content.len()).sum()
}

async fn normalize_capability_refs(
    ctx: &Ctx,
    caps: Vec<AgentCapabilityConfig>,
) -> Result<Vec<AgentCapabilityConfig>, CommandError> {
    let caps = crate::domains::capabilities::validation::normalize_capability_refs(
        &ctx.db,
        ctx.org_id(),
        caps,
    )
    .await
    .map_err(classify_anyhow)?;
    crate::domains::capabilities::validation::validate_feature_gated_capability_refs(
        &ctx.feature_flags,
        &caps,
    )?;
    crate::domains::capabilities::validation::validate_hydrated_capability_size_for_org(
        &ctx.db,
        ctx.org_id(),
        &caps,
    )
    .await?;
    Ok(caps)
}

// ============================================================================
// Shared persistence helpers
// ============================================================================

async fn persist_capabilities(
    db: &crate::storage::StorageBackend,
    harness_uuid: uuid::Uuid,
    caps: &[AgentCapabilityConfig],
) -> Result<(), CommandError> {
    db.set_harness_capabilities(harness_uuid, q::cap_tuples(caps))
        .await?;
    Ok(())
}

// ============================================================================
// CreateHarness
// ============================================================================

/// Create a new harness with a name, system prompt, and optional capabilities.
#[derive(Debug, Deserialize)]
pub struct CreateHarness(pub CreateHarnessRequest);

impl CommandSchema for CreateHarness {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateHarnessRequest>()
    }
}

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

        // Enforce per-org harness cap before insert. The count excludes
        // soft-deleted rows and system-seeded built-in harnesses, so only
        // user-created harnesses consume the budget.
        let max = ctx.resource_limits.max_harnesses_per_org;
        let count = ctx
            .db
            .count_harnesses_for_org(ctx.org_id())
            .await
            .map_err(classify_anyhow)?;
        if count >= max {
            return Err(CommandError::conflict(format!(
                "Harness limit reached (max {max})"
            )));
        }

        // Business rules
        q::ensure_name_available(&ctx.db, ctx.org_id(), &req.name, None).await?;
        let caps = normalize_capability_refs(
            ctx,
            q::ensure_file_system_capability(
                req.capabilities.clone(),
                !req.initial_files.is_empty(),
            ),
        )
        .await?;
        crate::domains::capabilities::validation::validate_capability_refs(
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
        let parent = match parent_harness_id {
            Some(parent_id) => q::resolve_effective(ctx.db.as_ref(), ctx.org_id(), parent_id)
                .await
                .map_err(classify_anyhow)?,
            None => None,
        };
        let mut scoped_mcp_layers = Vec::new();
        if let Some(ref parent) = parent {
            scoped_mcp_layers.push(&parent.mcp_servers);
        }
        scoped_mcp_layers.push(&req.mcp_servers);
        crate::domains::mcp_servers::scoped_mcp::validate_merged_scoped_mcp_servers(
            scoped_mcp_layers,
        )
        .map_err(classify_anyhow)?;
        let default_model_id = q::validate_model_id(&ctx.db, ctx.org_id(), req.default_model_id)
            .await
            .map_err(classify_anyhow)?;

        // Persist
        let input = CreateHarnessRow {
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            // Normalize an empty/whitespace-only prompt to "no base prompt" so
            // storage matches the documented semantics (the composition layer
            // trims it anyway).
            system_prompt: req.system_prompt.filter(|s| !s.trim().is_empty()),
            parent_harness_id,
            default_model_id,
            tags: req.tags,
            initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
            mcp_servers: serde_json::to_value(&req.mcp_servers).unwrap_or_default(),
            network_access: req
                .network_access
                .as_ref()
                .map(|na| serde_json::to_value(na).unwrap_or_default()),
            embedder_metadata: serde_json::to_value(&req.embedder_metadata).unwrap_or_default(),
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
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListHarnesses {
    pub search: Option<String>,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
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

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<Harness>())
    }

    fn output_shape() -> &'static str {
        "array"
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
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetHarness {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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

    fn positional_arg() -> Option<&'static str> {
        Some("id")
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
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateHarnessCmd {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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

    fn positional_arg() -> Option<&'static str> {
        Some("id")
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
        if matches!(req.status, Some(HarnessStatus::Deleted)) {
            return Err(CommandError::forbidden(
                "Setting status=deleted requires dangerous delete permission".to_string(),
            ));
        }

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
            Some(caps) => Some(
                normalize_capability_refs(
                    ctx,
                    q::ensure_file_system_capability(caps, final_has_initial_files),
                )
                .await?,
            ),
            None if final_has_initial_files => Some(q::ensure_file_system_capability(
                q::get_capabilities(&ctx.db, ctx.org_id(), harness_id.uuid())
                    .await
                    .map_err(classify_anyhow)?,
                true,
            )),
            None => None,
        };
        if let Some(ref caps) = capabilities_override {
            crate::domains::capabilities::validation::validate_capability_refs(
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
        let updated_mcp_servers = req.mcp_servers.clone().unwrap_or_else(|| {
            serde_json::from_value(existing.mcp_servers.clone()).unwrap_or_default()
        });
        let effective_parent_harness_id = req
            .parent_harness_id
            .map(|_| parent_harness_id)
            .unwrap_or(existing.parent_harness_id);
        let parent = match effective_parent_harness_id {
            Some(parent_id) => q::resolve_effective(ctx.db.as_ref(), ctx.org_id(), parent_id)
                .await
                .map_err(classify_anyhow)?,
            None => None,
        };
        let mut scoped_mcp_layers = Vec::new();
        if let Some(ref parent) = parent {
            scoped_mcp_layers.push(&parent.mcp_servers);
        }
        scoped_mcp_layers.push(&updated_mcp_servers);
        crate::domains::mcp_servers::scoped_mcp::validate_merged_scoped_mcp_servers(
            scoped_mcp_layers,
        )
        .map_err(classify_anyhow)?;

        // Persist
        let input = UpdateHarness {
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            // Omitted = leave unchanged; present empty/whitespace = clear to no
            // base prompt; present text = set. Mirrors create-path normalization.
            system_prompt: req
                .system_prompt
                .map(|s| (!s.trim().is_empty()).then_some(s)),
            parent_harness_id: req.parent_harness_id.map(|_| parent_harness_id),
            default_model_id,
            tags: req.tags,
            initial_files: req
                .initial_files
                .map(|files| serde_json::to_value(&files).unwrap_or_default()),
            mcp_servers: req
                .mcp_servers
                .map(|servers| serde_json::to_value(&servers).unwrap_or_default()),
            network_access: req
                .network_access
                .map(|na| Some(serde_json::to_value(na).unwrap_or_default())),
            embedder_metadata: req
                .embedder_metadata
                .map(|m| serde_json::to_value(&m).unwrap_or_default()),
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
            q::get_capabilities(&ctx.db, ctx.org_id(), harness_id.uuid())
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
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteHarness {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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

    fn positional_arg() -> Option<&'static str> {
        Some("id")
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
            .map_err(|err| CommandError::conflict(err.to_string()))?;
        q::ensure_not_org_default_harness(&ctx.db, ctx.org_id(), harness_id).await?;
        crate::domains::apps::queries::ensure_no_app_references_to_harness(
            &ctx.db,
            ctx.org_id(),
            harness_id.uuid(),
        )
        .await?;

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
#[derive(Debug, Deserialize, ToSchema)]
pub struct DestroyHarness {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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

    fn positional_arg() -> Option<&'static str> {
        Some("id")
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
            .map_err(|err| CommandError::conflict(err.to_string()))?;
        q::ensure_not_org_default_harness(&ctx.db, ctx.org_id(), harness_id).await?;
        crate::domains::apps::queries::ensure_no_app_references_to_harness(
            &ctx.db,
            ctx.org_id(),
            harness_id.uuid(),
        )
        .await?;

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
#[derive(Debug, Deserialize, ToSchema)]
pub struct CopyHarness {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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

    fn positional_arg() -> Option<&'static str> {
        Some("id")
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
            mcp_servers: source.mcp_servers,
            network_access: None,
            embedder_metadata: source.embedder_metadata,
        };

        CreateHarness(req).execute(ctx).await
    }
}

inventory::submit! { CommandDescriptor::of::<CopyHarness>() }

// ============================================================================
// PreviewHarness
// ============================================================================

/// Preview the final harness shape with capabilities applied.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PreviewHarness {
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub parent_harness_id: Option<HarnessId>,
    #[serde(default)]
    #[schema(value_type = Vec<everruns_platform::CapabilityRefSchema>)]
    pub capabilities: Vec<AgentCapabilityConfig>,
    #[serde(default)]
    pub mcp_servers: ScopedMcpServers,
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

    fn read_only() -> bool {
        true
    }

    fn policy() -> Option<&'static Policy> {
        Some(&HARNESS_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<HarnessPreview, CommandError> {
        let parent = match self.parent_harness_id {
            Some(parent_id) => q::resolve_effective(ctx.db.as_ref(), ctx.org_id(), parent_id)
                .await
                .map_err(classify_anyhow)?,
            None => None,
        };
        crate::domains::mcp_servers::scoped_mcp::validate_scoped_mcp_servers(&self.mcp_servers)
            .map_err(classify_anyhow)?;
        let (system_prompt, capabilities) = q::merge_preview_layer(
            parent.as_ref(),
            &self.system_prompt.unwrap_or_default(),
            &self.capabilities,
        );
        let effective_mcp_servers = merge_scoped_mcp_servers(
            &parent
                .as_ref()
                .map(|h| h.mcp_servers.clone())
                .unwrap_or_default(),
            &self.mcp_servers,
        );
        crate::domains::mcp_servers::scoped_mcp::validate_scoped_mcp_servers(
            &effective_mcp_servers,
        )
        .map_err(classify_anyhow)?;
        let (system_prompt, mut tools) = ctx
            .capability_service
            .preview(ctx.org_id(), &system_prompt, &capabilities)
            .await
            .map_err(classify_anyhow)?;
        tools.extend(
            crate::domains::mcp_servers::scoped_mcp::build_scoped_mcp_tool_definitions(
                &effective_mcp_servers,
                None,
                None,
                ctx.capability_service.egress_service().as_ref(),
            )
            .await
            .map_err(classify_anyhow)?,
        );
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
#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckHarnessName {
    /// Human-readable name. Safe to render in user-facing messages.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_imports::{
        Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, DefaultPermissionResolver, OrgRole,
    };
    use crate::services::CapabilityService;
    use crate::storage::StorageBackend;
    use std::sync::Arc;
    use uuid::Uuid;

    fn test_ctx(db: Arc<StorageBackend>, max_harnesses_per_org: i64) -> Ctx {
        let capability_service = Arc::new(CapabilityService::new(db.clone(), None));
        let mut ctx = Ctx::new(
            Caller {
                org_id: DEFAULT_ORG_ID,
                org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
                user_id: Some(Uuid::nil()),
                role: OrgRole::Owner,
                is_platform_user: false,
                is_internal: false,
            },
            db,
            capability_service,
            None,
            Arc::new(DefaultPermissionResolver),
        );
        ctx.resource_limits.max_harnesses_per_org = max_harnesses_per_org;
        ctx
    }

    fn basic_request(name: &str) -> CreateHarnessRequest {
        CreateHarnessRequest {
            name: name.to_string(),
            display_name: None,
            description: None,
            system_prompt: Some("prompt".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
        }
    }

    fn update_request_with_metadata(
        embedder_metadata: std::collections::HashMap<String, String>,
    ) -> UpdateHarnessRequest {
        UpdateHarnessRequest {
            name: None,
            display_name: None,
            description: None,
            system_prompt: None,
            parent_harness_id: None,
            default_model_id: None,
            tags: None,
            capabilities: None,
            initial_files: None,
            mcp_servers: None,
            network_access: None,
            status: None,
            embedder_metadata: Some(embedder_metadata),
        }
    }

    fn metadata_entries(count: usize) -> std::collections::HashMap<String, String> {
        (0..count)
            .map(|i| (format!("key_{i}"), format!("value_{i}")))
            .collect()
    }

    #[test]
    fn create_rejects_embedder_metadata_without_system_key_headroom() {
        let mut req = basic_request("metadata-heavy");
        req.embedder_metadata = metadata_entries(MAX_EMBEDDER_METADATA_KEYS + 1);

        let err = validate_create_limits(&req)
            .expect_err("embedder metadata must leave room for system metadata");

        assert_eq!(err.status().as_u16(), 400);
        assert!(err.message().contains("Embedder metadata"));
    }

    #[test]
    fn create_rejects_overlong_embedder_metadata_key_or_value() {
        let mut req = basic_request("metadata-overlong-key");
        req.embedder_metadata
            .insert("k".repeat(MAX_METADATA_KEY_LENGTH + 1), "value".to_string());
        assert!(validate_create_limits(&req).is_err());

        let mut req = basic_request("metadata-overlong-value");
        req.embedder_metadata
            .insert("key".to_string(), "v".repeat(MAX_METADATA_VALUE_LENGTH + 1));
        assert!(validate_create_limits(&req).is_err());
    }

    #[test]
    fn update_rejects_embedder_metadata_provider_limit_violations() {
        let err = validate_update_limits(&update_request_with_metadata(metadata_entries(
            MAX_EMBEDDER_METADATA_KEYS + 1,
        )))
        .expect_err("update must apply the same metadata key limit");
        assert_eq!(err.status().as_u16(), 400);

        let mut metadata = metadata_entries(1);
        metadata.insert("key".to_string(), "v".repeat(MAX_METADATA_VALUE_LENGTH + 1));
        assert!(validate_update_limits(&update_request_with_metadata(metadata)).is_err());
    }

    #[test]
    fn accepts_embedder_metadata_that_leaves_system_key_headroom() {
        let mut req = basic_request("metadata-ok");
        req.embedder_metadata = metadata_entries(MAX_EMBEDDER_METADATA_KEYS);

        validate_create_limits(&req).expect("metadata within reserved provider limits");
    }

    #[tokio::test]
    async fn harness_creation_rejected_at_limit_and_allowed_below() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = test_ctx(db, 2);

        CreateHarness(basic_request("h1"))
            .execute(&ctx)
            .await
            .expect("first harness below limit");
        CreateHarness(basic_request("h2"))
            .execute(&ctx)
            .await
            .expect("second harness at limit boundary");

        let err = CreateHarness(basic_request("h3"))
            .execute(&ctx)
            .await
            .expect_err("third harness exceeds the cap");
        assert_eq!(err.status().as_u16(), 409);
        assert!(err.message().contains("Harness limit reached"));
    }

    #[tokio::test]
    async fn soft_deleted_harnesses_do_not_count_toward_limit() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = test_ctx(db, 1);

        let h1 = CreateHarness(basic_request("h1"))
            .execute(&ctx)
            .await
            .expect("first harness below limit");

        // At the cap: a second create is rejected.
        let err = CreateHarness(basic_request("h2"))
            .execute(&ctx)
            .await
            .expect_err("second harness exceeds the cap");
        assert_eq!(err.status().as_u16(), 409);

        // Soft-delete h1 (archive then mark deleted). A deleted row must not
        // count toward the cap, so creation succeeds again.
        DeleteHarness {
            id: h1.id.to_string(),
        }
        .execute(&ctx)
        .await
        .expect("archive h1");
        DestroyHarness {
            id: h1.id.to_string(),
        }
        .execute(&ctx)
        .await
        .expect("mark h1 deleted");

        CreateHarness(basic_request("h2"))
            .execute(&ctx)
            .await
            .expect("creation allowed once the deleted harness is excluded");
    }

    #[tokio::test]
    async fn built_in_harnesses_do_not_count_toward_limit() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = test_ctx(db.clone(), 1);

        // Seed a system harness (is_built_in = true), as platform bootstrap does.
        db.create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: "seeded-system".to_string(),
                display_name: None,
                description: None,
                system_prompt: None,
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                embedder_metadata: serde_json::json!({}),
                is_built_in: true,
            },
        )
        .await
        .expect("seed built-in harness");

        // The built-in must not consume the cap, so a user harness still fits.
        CreateHarness(basic_request("h1"))
            .execute(&ctx)
            .await
            .expect("user harness allowed despite a built-in present");

        // ...but the next user harness is now at the cap.
        let err = CreateHarness(basic_request("h2"))
            .execute(&ctx)
            .await
            .expect_err("second user harness exceeds the cap");
        assert_eq!(err.status().as_u16(), 409);
    }
}

// Agent commands — user-facing operations.
//
// Each struct is the request type, catalog entry, and execution logic.
// inventory::submit! auto-registers for MCP catalog.

use super::queries as q;
use super::types::{CreateAgentRequest, CreateAgentRow, UpdateAgent, UpdateAgentRequest};
use super::{AGENT_DANGEROUS, AGENT_MANAGE, AGENT_VIEW};
use crate::domains::common::*;
use crate::max_iterations;
use everruns_core::typed_id::AgentId;
use everruns_core::{
    Agent, AgentCapabilityConfig, AgentStatus, InitialFile, OrgRole, Policy, ScopedMcpServers,
    ToolDefinition,
};
use serde::Deserialize;
use utoipa::ToSchema;

// ============================================================================
// Input validation
// ============================================================================

use crate::api::validation::{
    MAX_AGENT_CAPABILITIES, MAX_AGENT_DESCRIPTION_BYTES, MAX_AGENT_NAME_BYTES,
    MAX_AGENT_SYSTEM_PROMPT_BYTES, MAX_INITIAL_FILES, MAX_INITIAL_FILES_TOTAL_BYTES,
};

fn validate_create_limits(req: &CreateAgentRequest) -> Result<(), CommandError> {
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

fn validate_update_limits(req: &UpdateAgentRequest) -> Result<(), CommandError> {
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

fn check_high_risk_caps(ctx: &Ctx, caps: &[AgentCapabilityConfig]) -> Result<(), CommandError> {
    if caps.is_empty() || ctx.caller.role.has_permission(OrgRole::Admin) {
        return Ok(());
    }
    let refs: Vec<&str> = caps.iter().map(|c| c.capability_ref.as_str()).collect();
    let high = ctx.capability_service.high_risk_ids(&refs);
    if !high.is_empty() {
        return Err(CommandError::Forbidden(format!(
            "Admin role required to assign high-risk capabilities: {}",
            high.join(", ")
        )));
    }
    Ok(())
}

// ============================================================================
// Shared persistence helpers
// ============================================================================

async fn persist_capabilities(
    db: &crate::storage::StorageBackend,
    agent_uuid: uuid::Uuid,
    caps: &[AgentCapabilityConfig],
) -> Result<(), CommandError> {
    db.set_agent_capabilities(agent_uuid, q::cap_tuples(caps))
        .await?;
    Ok(())
}

// ============================================================================
// CreateAgent
// ============================================================================

/// Create a new agent with a name, system prompt, and optional capabilities.
#[derive(Debug, Deserialize)]
pub struct CreateAgent(pub CreateAgentRequest);

impl CommandSchema for CreateAgent {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateAgentRequest>()
    }
}

impl Command for CreateAgent {
    type Output = Agent;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_agent",
            category: "agents",
            description: "Create a new agent with a name, system prompt, and optional capabilities.",
            method: "POST",
            path: "/v1/agents",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        let req = self.0;

        // Validate
        validate_name("Agent", &req.name)?;
        validate_create_limits(&req)?;
        check_high_risk_caps(ctx, &req.capabilities)?;

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
        crate::services::scoped_mcp::validate_scoped_mcp_servers(&req.mcp_servers)
            .map_err(classify_anyhow)?;
        let default_model_id = q::validate_model_id(&ctx.db, ctx.org_id(), req.default_model_id)
            .await
            .map_err(classify_anyhow)?;

        // Persist
        let client_id = req.id;
        let (row, agent_uuid) = if let Some(client_id) = client_id {
            let input = CreateAgentRow {
                public_id: client_id.to_string(),
                name: req.name,
                display_name: req.display_name,
                description: req.description,
                system_prompt: req.system_prompt,
                default_model_id,
                tags: req.tags,
                initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
                tools: serde_json::to_value(&req.tools).unwrap_or_default(),
                mcp_servers: serde_json::to_value(&req.mcp_servers).unwrap_or_default(),
                network_access: req
                    .network_access
                    .as_ref()
                    .map(|na| serde_json::to_value(na).unwrap()),
                max_iterations: max_iterations::to_db(req.max_iterations)
                    .map_err(classify_anyhow)?,
            };
            let row = ctx
                .db
                .create_agent(ctx.org_id(), input)
                .await
                .map_err(classify_anyhow)?;
            let uuid = row.id.uuid();
            (row, uuid)
        } else {
            let internal_uuid = uuid::Uuid::now_v7();
            let public_id = AgentId::from_uuid(internal_uuid);
            let input = CreateAgentRow {
                public_id: public_id.to_string(),
                name: req.name,
                display_name: req.display_name,
                description: req.description,
                system_prompt: req.system_prompt,
                default_model_id,
                tags: req.tags,
                initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
                tools: serde_json::to_value(&req.tools).unwrap_or_default(),
                mcp_servers: serde_json::to_value(&req.mcp_servers).unwrap_or_default(),
                network_access: req
                    .network_access
                    .as_ref()
                    .map(|na| serde_json::to_value(na).unwrap()),
                max_iterations: max_iterations::to_db(req.max_iterations)
                    .map_err(classify_anyhow)?,
            };
            let row = ctx
                .db
                .create_agent_with_id(ctx.org_id(), AgentId::from_uuid(internal_uuid), input)
                .await
                .map_err(classify_anyhow)?
                .ok_or_else(|| CommandError::Conflict("Agent UUID collision".into()))?;
            (row, internal_uuid)
        };

        persist_capabilities(&ctx.db, agent_uuid, &caps).await?;
        Ok(q::row_to_agent(row, caps))
    }
}

inventory::submit! { CommandDescriptor::of::<CreateAgent>() }

// ============================================================================
// ListAgents
// ============================================================================

/// List agents. Supports search, include_archived, pagination.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListAgents {
    pub search: Option<String>,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub include_archived: bool,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    pub offset: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    pub limit: Option<u32>,
}

impl Command for ListAgents {
    type Output = Paginated<Agent>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_agents",
            category: "agents",
            description: "List all active agents. Use search for name search, include_archived=true to include archived. Supports pagination (limit/offset).",
            method: "GET",
            path: "/v1/agents",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Paginated<Agent>, CommandError> {
        let pg = pagination(self.offset, self.limit);
        let (rows, total) = ctx
            .db
            .list_agents(
                ctx.org_id(),
                self.search.as_deref(),
                self.include_archived,
                pg,
            )
            .await
            .map_err(classify_anyhow)?;
        let agents = q::load_agents_list(&ctx.db, rows)
            .await
            .map_err(classify_anyhow)?;
        Ok(Paginated {
            data: agents,
            total,
            offset: pg.offset,
            limit: pg.limit,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<ListAgents>() }

// ============================================================================
// GetAgent
// ============================================================================

/// Get a single agent by ID or name.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetAgent {
    pub id: String,
}

impl Command for GetAgent {
    type Output = Agent;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_agent",
            category: "agents",
            description: "Get a single agent by ID or name.",
            method: "GET",
            path: "/v1/agents/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        q::resolve(&ctx.db, ctx.org_id(), &self.id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetAgent>() }

// ============================================================================
// UpdateAgent
// ============================================================================

/// Update an agent. Only provided fields are changed.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAgentCmd {
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateAgentRequest,
}

impl Command for UpdateAgentCmd {
    type Output = Agent;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_agent",
            category: "agents",
            description: "Update an agent. Only provided fields are changed.",
            method: "PATCH",
            path: "/v1/agents/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        let agent_id: AgentId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid agent ID: {e}")))?;

        let req = self.req;

        if let Some(ref name) = req.name {
            validate_name("Agent", name)?;
        }
        validate_update_limits(&req)?;
        if matches!(req.status, Some(AgentStatus::Deleted)) {
            return Err(CommandError::forbidden(
                "Setting status=deleted requires dangerous delete permission",
            ));
        }
        if let Some(ref caps) = req.capabilities {
            check_high_risk_caps(ctx, caps)?;
        }

        // Resolve existing
        let existing = ctx
            .db
            .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        if existing.status != "active" {
            return Err(CommandError::bad_request(
                "Archived or deleted agents cannot be edited",
            ));
        }

        let internal_id = existing.id;
        if let Some(ref name) = req.name {
            q::ensure_name_available(&ctx.db, ctx.org_id(), name, Some(internal_id)).await?;
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
                q::get_capabilities(&ctx.db, internal_id.uuid())
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
        if let Some(ref servers) = req.mcp_servers {
            crate::services::scoped_mcp::validate_scoped_mcp_servers(servers)
                .map_err(classify_anyhow)?;
        }
        let default_model_id = q::validate_model_id(&ctx.db, ctx.org_id(), req.default_model_id)
            .await
            .map_err(classify_anyhow)?;

        // Persist
        let input = UpdateAgent {
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id,
            tags: req.tags,
            status: req.status.map(|s| s.to_string()),
            initial_files: req
                .initial_files
                .map(|files| serde_json::to_value(&files).unwrap_or_default()),
            tools: req
                .tools
                .map(|t| serde_json::to_value(&t).unwrap_or_default()),
            mcp_servers: req
                .mcp_servers
                .map(|servers| serde_json::to_value(&servers).unwrap_or_default()),
            max_iterations: req
                .max_iterations
                .map(|v| max_iterations::to_db(Some(v)))
                .transpose()
                .map_err(classify_anyhow)?,
            network_access: req
                .network_access
                .map(|na| Some(serde_json::to_value(na).unwrap())),
        };
        let row = ctx
            .db
            .update_agent(ctx.org_id(), internal_id, input)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        let caps = if let Some(caps) = capabilities_override {
            persist_capabilities(&ctx.db, internal_id.uuid(), &caps).await?;
            caps
        } else {
            q::get_capabilities(&ctx.db, internal_id.uuid())
                .await
                .map_err(classify_anyhow)?
        };

        Ok(q::row_to_agent(row, caps))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateAgentCmd>() }

// ============================================================================
// DeleteAgent
// ============================================================================

/// Archive an agent (soft delete).
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteAgent {
    pub id: String,
}

impl Command for DeleteAgent {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_agent",
            category: "agents",
            description: "Archive an agent (soft delete). Can be restored.",
            method: "DELETE",
            path: "/v1/agents/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let agent_id: AgentId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid agent ID: {e}")))?;

        let row = ctx
            .db
            .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        ctx.db
            .delete_agent(ctx.org_id(), row.id)
            .await
            .map_err(classify_anyhow)?;

        Ok(serde_json::json!({"deleted": true}))
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteAgent>() }

// ============================================================================
// UpsertAgent
// ============================================================================

/// Upsert agent — create or update by ID.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertAgent {
    pub id: String,
    #[serde(flatten)]
    pub req: CreateAgentRequest,
}

/// Result of an upsert operation, including whether the agent was created or updated.
#[derive(Debug, serde::Serialize)]
pub struct UpsertResult {
    #[serde(flatten)]
    pub agent: Agent,
    pub was_created: bool,
}

impl Command for UpsertAgent {
    type Output = UpsertResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "upsert_agent",
            category: "agents",
            description: "Upsert agent — create (201) or update (200) by ID.",
            method: "PUT",
            path: "/v1/agents/{id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<UpsertResult, CommandError> {
        let req = self.req;

        // Validate (same checks as CreateAgent)
        validate_name("Agent", &req.name)?;
        validate_create_limits(&req)?;
        check_high_risk_caps(ctx, &req.capabilities)?;

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
        crate::services::scoped_mcp::validate_scoped_mcp_servers(&req.mcp_servers)
            .map_err(classify_anyhow)?;
        let default_model_id = q::validate_model_id(&ctx.db, ctx.org_id(), req.default_model_id)
            .await
            .map_err(classify_anyhow)?;

        let input = CreateAgentRow {
            public_id: self.id,
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id,
            tags: req.tags,
            initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
            tools: serde_json::to_value(&req.tools).unwrap_or_default(),
            mcp_servers: serde_json::to_value(&req.mcp_servers).unwrap_or_default(),
            max_iterations: max_iterations::to_db(req.max_iterations).map_err(classify_anyhow)?,
            network_access: req
                .network_access
                .as_ref()
                .map(|na| serde_json::to_value(na).unwrap()),
        };
        let (row, was_created) = ctx
            .db
            .upsert_agent(ctx.org_id(), input)
            .await
            .map_err(classify_anyhow)?;
        let agent_uuid = row.id.uuid();

        let final_caps = if !caps.is_empty() {
            persist_capabilities(&ctx.db, agent_uuid, &caps).await?;
            caps
        } else if was_created {
            vec![]
        } else {
            q::get_capabilities(&ctx.db, agent_uuid)
                .await
                .map_err(classify_anyhow)?
        };

        Ok(UpsertResult {
            agent: q::row_to_agent(row, final_caps),
            was_created,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<UpsertAgent>() }

// ============================================================================
// CopyAgent
// ============================================================================

/// Copy an agent. Generates a unique name ({name}-copy, -copy-2, etc.)
#[derive(Debug, Deserialize, ToSchema)]
pub struct CopyAgent {
    pub id: String,
}

impl Command for CopyAgent {
    type Output = Agent;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "copy_agent",
            category: "agents",
            description: "Copy an agent. Generates a unique name.",
            method: "POST",
            path: "/v1/agents/{id}/copy",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_MANAGE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        let source = q::resolve(&ctx.db, ctx.org_id(), &self.id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        let copy_name =
            q::find_unique_name(&ctx.db, ctx.org_id(), &format!("{}-copy", source.name))
                .await
                .map_err(classify_anyhow)?;

        let req = CreateAgentRequest {
            id: None,
            name: copy_name,
            display_name: source.display_name.map(|d| format!("{d} (copy)")),
            description: source.description,
            system_prompt: source.system_prompt,
            default_model_id: source.default_model_id,
            tags: source.tags,
            capabilities: source.capabilities,
            initial_files: source.initial_files,
            tools: source.tools,
            mcp_servers: source.mcp_servers,
            network_access: None,
            max_iterations: source.max_iterations,
        };

        CreateAgent(req).execute(ctx).await
    }
}

inventory::submit! { CommandDescriptor::of::<CopyAgent>() }

// ============================================================================
// ExportAgent
// ============================================================================

/// Get agent data for export.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ExportAgent {
    pub id: String,
}

impl Command for ExportAgent {
    type Output = Agent;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "export_agent",
            category: "agents",
            description: "Export agent as JSON.",
            method: "GET",
            path: "/v1/agents/{id}/export",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_VIEW)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        q::get_by_public_id(&ctx.db, ctx.org_id(), &self.id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))
    }
}

inventory::submit! { CommandDescriptor::of::<ExportAgent>() }

// ============================================================================
// ImportAgent
// ============================================================================

/// Import agent from JSON.
#[derive(Debug, Deserialize)]
pub struct ImportAgent(pub CreateAgentRequest);

impl CommandSchema for ImportAgent {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateAgentRequest>()
    }
}

impl Command for ImportAgent {
    type Output = Agent;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "import_agent",
            category: "agents",
            description: "Import an agent from a definition.",
            method: "POST",
            path: "/v1/agents/import",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        CreateAgent(self.0).execute(ctx).await
    }
}

inventory::submit! { CommandDescriptor::of::<ImportAgent>() }

// ============================================================================
// PreviewAgent
// ============================================================================

/// Preview the final agent shape with capabilities applied.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PreviewAgent {
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<AgentCapabilityConfig>,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub mcp_servers: ScopedMcpServers,
}

#[derive(Debug, serde::Serialize)]
pub struct AgentPreview {
    pub system_prompt: String,
    pub tools: Vec<ToolDefinition>,
}

impl Command for PreviewAgent {
    type Output = AgentPreview;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "preview_agent",
            category: "agents",
            description: "Preview the final agent shape with capabilities applied.",
            method: "POST",
            path: "/v1/agents/preview",
        }
    }

    fn read_only() -> bool {
        true
    }

    async fn execute(self, ctx: &Ctx) -> Result<AgentPreview, CommandError> {
        crate::services::scoped_mcp::validate_scoped_mcp_servers(&self.mcp_servers)
            .map_err(classify_anyhow)?;
        let (prompt, mut tools) = ctx
            .capability_service
            .preview(
                ctx.org_id(),
                &self.system_prompt.unwrap_or_default(),
                &self.capabilities,
            )
            .await
            .map_err(classify_anyhow)?;
        tools.extend(
            crate::services::scoped_mcp::build_scoped_mcp_tool_definitions(&self.mcp_servers)
                .await
                .map_err(classify_anyhow)?,
        );
        tools.extend(self.tools);
        Ok(AgentPreview {
            system_prompt: prompt,
            tools,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<PreviewAgent>() }

// ============================================================================
// CheckAgentName
// ============================================================================

/// Check whether an agent name is available.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckAgentName {
    pub name: String,
    pub exclude_id: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct NameAvailability {
    pub available: bool,
}

impl Command for CheckAgentName {
    type Output = NameAvailability;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "check_agent_name",
            category: "agents",
            description: "Check whether an agent name is available.",
            method: "GET",
            path: "/v1/agents/check-name",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<NameAvailability, CommandError> {
        if everruns_core::validate_addressable_name(&self.name).is_err() {
            return Ok(NameAvailability { available: false });
        }

        let exclude_id = self
            .exclude_id
            .map(|id| {
                id.parse::<AgentId>()
                    .map_err(|e| CommandError::bad_request(format!("Invalid exclude_id: {e}")))
            })
            .transpose()?;

        let existing = ctx
            .db
            .get_agent_by_name(ctx.org_id(), &self.name)
            .await
            .map_err(classify_anyhow)?;

        let available = match existing {
            Some(row) => exclude_id == Some(row.id),
            None => true,
        };

        Ok(NameAvailability { available })
    }
}

inventory::submit! { CommandDescriptor::of::<CheckAgentName>() }

// ============================================================================
// DestroyAgent (hard delete)
// ============================================================================

/// Permanently delete an archived agent.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DestroyAgent {
    pub id: String,
}

impl Command for DestroyAgent {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "destroy_agent",
            category: "agents",
            description: "Permanently delete an archived agent.",
            method: "POST",
            path: "/v1/agents/{id}/delete",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_DANGEROUS)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let agent_id: AgentId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid agent ID: {e}")))?;

        let row = ctx
            .db
            .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        if row.status != "archived" {
            return Err(CommandError::bad_request(
                "Agent must be archived before deletion",
            ));
        }

        ctx.db
            .destroy_agent(ctx.org_id(), row.id)
            .await
            .map_err(classify_anyhow)?;

        Ok(serde_json::json!({"destroyed": true}))
    }
}

inventory::submit! { CommandDescriptor::of::<DestroyAgent>() }

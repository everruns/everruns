// Agent commands — user-facing operations.
//
// Each struct is the request type, catalog entry, and execution logic.
// inventory::submit! auto-registers for MCP catalog.

use super::queries as q;
use super::types::{
    AgentRow, AgentVersionDiffResponse, CreateAgentRequest, CreateAgentRow,
    CreateAgentVersionRequest, ForkAgentVersionRequest, RollbackAgentVersionRequest,
    SetDefaultAgentVersionRequest, UpdateAgent, UpdateAgentRequest,
};
use super::{AGENT_DANGEROUS, AGENT_MANAGE, AGENT_VIEW};
use crate::domains::common::*;
use crate::kernel_imports::{
    AgentCapabilityConfig, InitialFile, OrgRole, Policy, ScopedMcpServers,
    everruns_provider::tool_types::ToolDefinition,
};
use crate::max_iterations;
use everruns_platform::{Agent, AgentStatus, AgentVersion, AgentVersionChangeKind};
use everruns_provider::typed_id::{AgentId, AgentVersionId, HarnessId};
use serde::Deserialize;
use utoipa::ToSchema;

// ============================================================================
// Input validation
// ============================================================================

use crate::api::validation::{
    MAX_AGENT_CAPABILITIES, MAX_AGENT_DESCRIPTION_BYTES, MAX_AGENT_NAME_BYTES,
    MAX_AGENT_SYSTEM_PROMPT_BYTES, MAX_INITIAL_FILES, MAX_INITIAL_FILES_TOTAL_BYTES,
};

const MAX_AUTO_SNAPSHOTS_PER_AGENT: i64 = 50;

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

async fn check_high_risk_caps(
    ctx: &Ctx,
    caps: &[AgentCapabilityConfig],
) -> Result<(), CommandError> {
    if caps.is_empty() || ctx.caller.role.has_permission(OrgRole::Admin) {
        return Ok(());
    }
    let refs: Vec<&str> = caps.iter().map(|c| c.capability_id()).collect();
    let high = ctx
        .capability_service
        .high_risk_ids_for_org(ctx.org_id(), &refs)
        .await
        .map_err(classify_anyhow)?;
    if !high.is_empty() {
        return Err(CommandError::forbidden(format!(
            "Admin role required to assign high-risk capabilities: {}",
            high.join(", ")
        )));
    }
    Ok(())
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
    Ok(caps)
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

async fn persist_harness_source(
    ctx: &Ctx,
    row: AgentRow,
    source: &str,
) -> Result<AgentRow, CommandError> {
    if row.harness_source == source {
        return Ok(row);
    }

    ctx.db
        .update_agent(
            ctx.org_id(),
            row.id,
            UpdateAgent {
                harness_source: Some(source.to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Agent"))
}

async fn resolve_create_harness_id(
    ctx: &Ctx,
    harness_id: Option<HarnessId>,
    harness_name: Option<&str>,
) -> Result<HarnessId, CommandError> {
    resolve_harness_id(ctx, harness_id, harness_name, true)
        .await?
        .ok_or_else(|| CommandError::not_found("Harness"))
}

async fn resolve_update_harness_id(
    ctx: &Ctx,
    harness_id: Option<HarnessId>,
    harness_name: Option<&str>,
) -> Result<Option<HarnessId>, CommandError> {
    resolve_harness_id(ctx, harness_id, harness_name, false).await
}

async fn resolve_harness_id(
    ctx: &Ctx,
    harness_id: Option<HarnessId>,
    harness_name: Option<&str>,
    default_when_omitted: bool,
) -> Result<Option<HarnessId>, CommandError> {
    if harness_id.is_some() && harness_name.is_some() {
        return Err(CommandError::bad_request(
            "harness_id and harness_name are mutually exclusive",
        ));
    }

    let row = if let Some(id) = harness_id {
        ctx.db
            .get_harness(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
    } else if let Some(name) = harness_name {
        ctx.db
            .get_harness_by_name(ctx.org_id(), name)
            .await
            .map_err(classify_anyhow)?
    } else if default_when_omitted {
        let id = crate::domains::sessions::queries::resolve_session_harness_id(
            &ctx.db,
            ctx.org_id(),
            None,
            None,
            ctx.fallback_harness_name.as_deref().or(Some("generic")),
        )
        .await
        .map_err(classify_anyhow)?;
        ctx.db
            .get_harness(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
    } else {
        None
    };

    let Some(row) = row else {
        return if default_when_omitted || harness_id.is_some() || harness_name.is_some() {
            Err(CommandError::not_found("Harness"))
        } else {
            Ok(None)
        };
    };

    if row.status != "active" {
        return Err(CommandError::bad_request(
            "Archived or deleted harnesses cannot be assigned to agents",
        ));
    }
    Ok(Some(row.id))
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
        check_high_risk_caps(ctx, &req.capabilities).await?;

        // Enforce per-org agent cap (excludes soft-deleted) before insert.
        let max = ctx.resource_limits.max_agents_per_org;
        let count = ctx
            .db
            .count_agents_for_org(ctx.org_id())
            .await
            .map_err(classify_anyhow)?;
        if count >= max {
            return Err(CommandError::conflict(format!(
                "Agent limit reached (max {max})"
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
        crate::domains::mcp_servers::scoped_mcp::validate_scoped_mcp_servers(&req.mcp_servers)
            .map_err(classify_anyhow)?;
        let default_model_id = q::validate_model_id(&ctx.db, ctx.org_id(), req.default_model_id)
            .await
            .map_err(classify_anyhow)?;
        let harness_source = if req.harness_id.is_some() || req.harness_name.is_some() {
            "explicit"
        } else {
            "organization_default"
        };
        let harness_id =
            resolve_create_harness_id(ctx, req.harness_id, req.harness_name.as_deref()).await?;

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
                harness_id,
                tags: req.tags,
                initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
                tools: serde_json::to_value(&req.tools).unwrap_or_default(),
                mcp_servers: serde_json::to_value(&req.mcp_servers).unwrap_or_default(),
                network_access: req
                    .network_access
                    .as_ref()
                    .map(|na| serde_json::to_value(na).unwrap_or_default()),
                max_iterations: max_iterations::to_db(req.max_iterations)
                    .map_err(classify_anyhow)?,
                parallel_tool_calls: req.parallel_tool_calls,
                // Built-in agents come from the platform definition via org
                // bootstrap. No API-facing creation path may mint one.
                is_built_in: false,
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
                harness_id,
                tags: req.tags,
                initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
                tools: serde_json::to_value(&req.tools).unwrap_or_default(),
                mcp_servers: serde_json::to_value(&req.mcp_servers).unwrap_or_default(),
                network_access: req
                    .network_access
                    .as_ref()
                    .map(|na| serde_json::to_value(na).unwrap_or_default()),
                max_iterations: max_iterations::to_db(req.max_iterations)
                    .map_err(classify_anyhow)?,
                parallel_tool_calls: req.parallel_tool_calls,
                // Built-in agents come from the platform definition via org
                // bootstrap. No API-facing creation path may mint one.
                is_built_in: false,
            };
            let row = ctx
                .db
                .create_agent_with_id(ctx.org_id(), AgentId::from_uuid(internal_uuid), input)
                .await
                .map_err(classify_anyhow)?
                .ok_or_else(|| {
                    CommandError::conflict("Agent UUID collision").with_code("agent_id_taken")
                })?;
            (row, internal_uuid)
        };

        let row = persist_harness_source(ctx, row, harness_source).await?;
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
    /// Zero-based offset into the result set.
    pub offset: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    /// Maximum number of items returned in this page.
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

    fn output_schema() -> serde_json::Value {
        paginated_output_schema(output_schema_for::<Agent>())
    }

    fn output_shape() -> &'static str {
        "paginated"
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
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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

        // Covers archive too: archiving is `status: archived` through here.
        q::ensure_not_built_in(&ctx.db, ctx.org_id(), &self.id, "modify").await?;

        let req = self.req;

        if let Some(ref name) = req.name {
            validate_name("Agent", name)?;
        }
        validate_update_limits(&req)?;
        if matches!(req.status, Some(AgentStatus::Deleted)) {
            return Err(CommandError::forbidden(
                "Setting status=deleted requires dangerous delete permission".to_string(),
            ));
        }
        if let Some(ref caps) = req.capabilities {
            check_high_risk_caps(ctx, caps).await?;
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
        let previous_config_hash = if ctx.feature_flags.agent_versions {
            let caps = q::get_capabilities(&ctx.db, ctx.org_id(), internal_id.uuid())
                .await
                .map_err(classify_anyhow)?;
            let agent = q::row_to_agent(existing.clone(), caps);
            Some(q::config_hash(&q::authored_config(&agent)))
        } else {
            None
        };
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
            Some(caps) => Some(
                normalize_capability_refs(
                    ctx,
                    q::ensure_file_system_capability(caps, final_has_initial_files),
                )
                .await?,
            ),
            None if final_has_initial_files => Some(q::ensure_file_system_capability(
                q::get_capabilities(&ctx.db, ctx.org_id(), internal_id.uuid())
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
        if let Some(ref servers) = req.mcp_servers {
            crate::domains::mcp_servers::scoped_mcp::validate_scoped_mcp_servers(servers)
                .map_err(classify_anyhow)?;
        }
        let default_model_id = q::validate_model_id(&ctx.db, ctx.org_id(), req.default_model_id)
            .await
            .map_err(classify_anyhow)?;
        let harness_id =
            resolve_update_harness_id(ctx, req.harness_id, req.harness_name.as_deref()).await?;

        // Persist
        let input = UpdateAgent {
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id,
            harness_id,
            harness_source: harness_id.map(|_| "explicit".to_string()),
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
                .map(|na| Some(serde_json::to_value(na).unwrap_or_default())),
            parallel_tool_calls: req.parallel_tool_calls.map(Some),
            ..Default::default()
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
            q::get_capabilities(&ctx.db, ctx.org_id(), internal_id.uuid())
                .await
                .map_err(classify_anyhow)?
        };

        let agent = q::row_to_agent(row, caps);
        let current_config_hash = q::config_hash(&q::authored_config(&agent));
        if previous_config_hash
            .as_ref()
            .is_none_or(|hash| hash != &current_config_hash)
        {
            create_auto_snapshot_from_agent(ctx, &agent).await?;
        }

        Ok(agent)
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateAgentCmd>() }

// ============================================================================
// DeleteAgent
// ============================================================================

/// Archive an agent (soft delete).
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteAgent {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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

        q::ensure_not_built_in(&ctx.db, ctx.org_id(), &self.id, "delete").await?;

        let row = ctx
            .db
            .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        crate::domains::apps::queries::ensure_no_app_references_to_agent(
            &ctx.db,
            ctx.org_id(),
            row.id.uuid(),
        )
        .await?;

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
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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
        let public_id = self.id;

        // Upsert addresses an agent by public id, so it reaches an existing
        // built-in the same way update does. An id that resolves to nothing
        // falls through to the create path, which cannot mint a built-in.
        q::ensure_not_built_in(&ctx.db, ctx.org_id(), &public_id, "modify").await?;

        // Validate (same checks as CreateAgent)
        validate_name("Agent", &req.name)?;
        validate_create_limits(&req)?;
        check_high_risk_caps(ctx, &req.capabilities).await?;

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
        crate::domains::mcp_servers::scoped_mcp::validate_scoped_mcp_servers(&req.mcp_servers)
            .map_err(classify_anyhow)?;
        let default_model_id = q::validate_model_id(&ctx.db, ctx.org_id(), req.default_model_id)
            .await
            .map_err(classify_anyhow)?;
        let harness_source = if req.harness_id.is_some() || req.harness_name.is_some() {
            "explicit"
        } else {
            "organization_default"
        };
        let harness_id =
            resolve_create_harness_id(ctx, req.harness_id, req.harness_name.as_deref()).await?;
        let previous_config_hash = if ctx.feature_flags.agent_versions {
            if let Some(existing) = ctx
                .db
                .get_agent_by_public_id(ctx.org_id(), &public_id)
                .await
                .map_err(classify_anyhow)?
            {
                let caps = q::get_capabilities(&ctx.db, ctx.org_id(), existing.id.uuid())
                    .await
                    .map_err(classify_anyhow)?;
                let agent = q::row_to_agent(existing, caps);
                Some(q::config_hash(&q::authored_config(&agent)))
            } else {
                None
            }
        } else {
            None
        };

        let input = CreateAgentRow {
            public_id: public_id.clone(),
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            system_prompt: req.system_prompt,
            default_model_id,
            harness_id,
            tags: req.tags,
            initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
            tools: serde_json::to_value(&req.tools).unwrap_or_default(),
            mcp_servers: serde_json::to_value(&req.mcp_servers).unwrap_or_default(),
            max_iterations: max_iterations::to_db(req.max_iterations).map_err(classify_anyhow)?,
            parallel_tool_calls: req.parallel_tool_calls,
            network_access: req
                .network_access
                .as_ref()
                .map(|na| serde_json::to_value(na).unwrap_or_default()),
            // See CreateAgent: only org bootstrap mints built-in agents.
            is_built_in: false,
        };
        let (row, was_created) = ctx
            .db
            .upsert_agent(ctx.org_id(), input)
            .await
            .map_err(classify_anyhow)?;
        let row = persist_harness_source(ctx, row, harness_source).await?;
        let agent_uuid = row.id.uuid();

        let final_caps = if !caps.is_empty() {
            persist_capabilities(&ctx.db, agent_uuid, &caps).await?;
            caps
        } else if was_created {
            vec![]
        } else {
            q::get_capabilities(&ctx.db, ctx.org_id(), agent_uuid)
                .await
                .map_err(classify_anyhow)?
        };

        let agent = q::row_to_agent(row, final_caps);
        let current_config_hash = q::config_hash(&q::authored_config(&agent));
        if !was_created
            && previous_config_hash
                .as_ref()
                .is_none_or(|hash| hash != &current_config_hash)
        {
            create_auto_snapshot_from_agent(ctx, &agent).await?;
        }

        Ok(UpsertResult { agent, was_created })
    }
}

inventory::submit! { CommandDescriptor::of::<UpsertAgent>() }

// ============================================================================
// CopyAgent
// ============================================================================

/// Copy an agent. Generates a unique name ({name}-copy, -copy-2, etc.)
#[derive(Debug, Deserialize, ToSchema)]
pub struct CopyAgent {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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
            harness_id: Some(source.harness_id),
            harness_name: None,
            tags: source.tags,
            capabilities: source.capabilities,
            initial_files: source.initial_files,
            tools: source.tools,
            mcp_servers: source.mcp_servers,
            network_access: None,
            max_iterations: source.max_iterations,
            parallel_tool_calls: source.parallel_tool_calls,
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
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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
// Agent versions
// ============================================================================

async fn resolve_agent(ctx: &Ctx, id: &str) -> Result<Agent, CommandError> {
    q::resolve(&ctx.db, ctx.org_id(), id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Agent"))
}

/// Resolve an agent for a command that mutates its version history.
///
/// Versions are part of the definition, so they are protected: a platform
/// upgrade ships a new built-in version, and an org that rolled its own would
/// silently diverge. Read-only version commands use [`resolve_agent`] instead.
async fn resolve_agent_for_mutation(ctx: &Ctx, id: &str) -> Result<Agent, CommandError> {
    let agent = resolve_agent(ctx, id).await?;
    q::ensure_not_built_in(&ctx.db, ctx.org_id(), id, "modify").await?;
    Ok(agent)
}

async fn resolve_agent_version(
    ctx: &Ctx,
    version_id: AgentVersionId,
) -> Result<AgentVersion, CommandError> {
    ctx.db
        .get_agent_version(ctx.org_id(), version_id)
        .await
        .map_err(classify_anyhow)?
        .map(q::row_to_agent_version)
        .ok_or_else(|| CommandError::not_found("Agent version"))
}

async fn build_resolved_config(
    ctx: &Ctx,
    agent: &Agent,
) -> Result<serde_json::Value, CommandError> {
    let preview = PreviewAgent {
        system_prompt: Some(agent.system_prompt.clone()),
        capabilities: agent.capabilities.clone(),
        tools: agent.tools.clone(),
        mcp_servers: agent.mcp_servers.clone(),
    }
    .execute(ctx)
    .await?;
    Ok(serde_json::json!({
        "system_prompt": preview.system_prompt,
        "tools": preview.tools,
        "capabilities": agent.capabilities,
        "mcp_servers": agent.mcp_servers,
        "default_model_id": agent.default_model_id.map(|id| id.to_string()),
        "harness_id": agent.harness_id.to_string(),
        "max_iterations": agent.max_iterations,
        "parallel_tool_calls": agent.parallel_tool_calls,
    }))
}

fn bump_published_version(
    previous: Option<&crate::storage::models::AgentVersionRow>,
    change_kind: &AgentVersionChangeKind,
) -> (i32, i32, i32, String) {
    if previous.is_none() {
        return (0, 1, 0, "0.1.0".to_string());
    }

    let (mut major, mut minor, mut patch) = previous
        .map(|v| (v.semver_major, v.semver_minor, v.semver_patch))
        .unwrap_or((0, 0, 0));
    match change_kind {
        AgentVersionChangeKind::Major => {
            major += 1;
            minor = 0;
            patch = 0;
        }
        AgentVersionChangeKind::Minor | AgentVersionChangeKind::Fork => {
            minor += 1;
            patch = 0;
        }
        _ => patch += 1,
    }
    let version = format!("{major}.{minor}.{patch}");
    (major, minor, patch, version)
}

async fn create_version_from_agent(
    ctx: &Ctx,
    agent: &Agent,
    change_kind: AgentVersionChangeKind,
    summary: Option<String>,
    source_version_id: Option<AgentVersionId>,
    is_published: bool,
) -> Result<AgentVersion, CommandError> {
    let agent_id = AgentId::from_uuid(agent.internal_id);
    let previous_snapshot = ctx
        .db
        .get_latest_agent_snapshot(ctx.org_id(), agent_id)
        .await
        .map_err(classify_anyhow)?;
    let previous_published = ctx
        .db
        .get_latest_agent_version(ctx.org_id(), agent_id)
        .await
        .map_err(classify_anyhow)?;
    let version_number = previous_snapshot
        .as_ref()
        .map_or(1, |row| row.version_number + 1);
    let (semver_major, semver_minor, semver_patch, version) = if is_published {
        bump_published_version(previous_published.as_ref(), &change_kind)
    } else {
        (0, 0, 0, format!("draft.{version_number}"))
    };
    let authored_config = q::authored_config(agent);
    let resolved_config = build_resolved_config(ctx, agent).await?;
    let version_id = AgentVersionId::new();
    let row = ctx
        .db
        .create_agent_version(crate::storage::models::CreateAgentVersionRow {
            id: version_id,
            public_id: version_id.to_string(),
            org_id: ctx.org_id(),
            agent_id,
            version_number,
            semver_major,
            semver_minor,
            semver_patch,
            version,
            is_published,
            parent_version_id: if is_published {
                previous_published.map(|row| row.id)
            } else {
                previous_snapshot.map(|row| row.id)
            },
            source_version_id,
            created_by_principal_id: None,
            change_kind: change_kind.to_string(),
            summary,
            config_hash: q::config_hash(&authored_config),
            authored_config,
            resolved_config,
        })
        .await
        .map_err(classify_anyhow)?;
    if is_published && agent.default_version_id.is_none() {
        ctx.db
            .update_agent(
                ctx.org_id(),
                agent_id,
                UpdateAgent {
                    default_version_id: Some(row.id),
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?;
    }
    Ok(q::row_to_agent_version(row))
}

async fn create_auto_snapshot_from_agent(ctx: &Ctx, agent: &Agent) -> Result<(), CommandError> {
    if !ctx.feature_flags.agent_versions {
        return Ok(());
    }

    let agent_id = AgentId::from_uuid(agent.internal_id);
    let authored_config = q::authored_config(agent);
    let config_hash = q::config_hash(&authored_config);
    if ctx
        .db
        .get_latest_agent_snapshot(ctx.org_id(), agent_id)
        .await
        .map_err(classify_anyhow)?
        .is_some_and(|row| row.config_hash == config_hash)
    {
        return Ok(());
    }

    // THREAT[TM-DOS-013]: Repeated no-op Agent updates can grow hidden snapshot rows.
    // Mitigation: automatic snapshots are feature-gated, deduplicated by latest config hash, and retained to a bounded per-Agent window.
    let mut last_conflict = None;
    for _ in 0..3 {
        match create_version_from_agent(ctx, agent, AgentVersionChangeKind::Auto, None, None, false)
            .await
        {
            Ok(_) => {
                ctx.db
                    .prune_agent_auto_snapshots(
                        ctx.org_id(),
                        agent_id,
                        MAX_AUTO_SNAPSHOTS_PER_AGENT,
                    )
                    .await
                    .map_err(classify_anyhow)?;
                return Ok(());
            }
            Err(CommandError {
                kind: CommandErrorKind::Conflict(message),
                ..
            }) => {
                last_conflict = Some(message);
            }
            Err(error) => return Err(error),
        }
    }
    Err(CommandError::conflict(last_conflict.unwrap_or_else(|| {
        "Agent version number conflict".to_string()
    })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListAgentVersions {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
}

impl Command for ListAgentVersions {
    type Output = Vec<AgentVersion>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_agent_versions",
            category: "agents",
            description: "List immutable versions for an agent.",
            method: "GET",
            path: "/v1/agents/{agent_id}/versions",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<AgentVersion>, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        let rows = ctx
            .db
            .list_agent_versions(ctx.org_id(), AgentId::from_uuid(agent.internal_id))
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.into_iter().map(q::row_to_agent_version).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListAgentVersions>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAgentVersionCmd {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
    pub req: CreateAgentVersionRequest,
}

impl Command for CreateAgentVersionCmd {
    type Output = AgentVersion;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_agent_version",
            category: "agents",
            description: "Save the current agent draft as an immutable version.",
            method: "POST",
            path: "/v1/agents/{agent_id}/versions",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AgentVersion, CommandError> {
        let agent = resolve_agent_for_mutation(ctx, &self.agent_id).await?;
        let change_kind = self
            .req
            .change_kind
            .unwrap_or(AgentVersionChangeKind::Manual);
        if change_kind == AgentVersionChangeKind::Auto {
            return Err(CommandError::bad_request(
                "Automatic change kind is reserved for draft snapshots",
            ));
        }
        create_version_from_agent(ctx, &agent, change_kind, self.req.summary, None, true).await
    }
}

inventory::submit! { CommandDescriptor::of::<CreateAgentVersionCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetDefaultAgentVersion {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
    pub req: SetDefaultAgentVersionRequest,
}

impl Command for SetDefaultAgentVersion {
    type Output = Agent;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "set_default_agent_version",
            category: "agents",
            description: "Set an agent's default immutable version.",
            method: "POST",
            path: "/v1/agents/{agent_id}/versions/default",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        let agent = resolve_agent_for_mutation(ctx, &self.agent_id).await?;
        let version = resolve_agent_version(ctx, self.req.version_id).await?;
        if version.agent_id.uuid() != agent.internal_id {
            return Err(CommandError::bad_request(
                "Agent version belongs to another agent",
            ));
        }
        if !version.is_published {
            return Err(CommandError::bad_request(
                "Default agent version must be published",
            ));
        }
        let version_agent = q::version_to_agent(&agent, &version);
        check_high_risk_caps(ctx, &version_agent.capabilities).await?;
        let row = ctx
            .db
            .update_agent(
                ctx.org_id(),
                AgentId::from_uuid(agent.internal_id),
                UpdateAgent {
                    default_version_id: Some(version.public_id),
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))?;
        let caps = q::get_capabilities(&ctx.db, row.org_id, row.id.uuid())
            .await
            .map_err(classify_anyhow)?;
        Ok(q::row_to_agent(row, caps))
    }
}

inventory::submit! { CommandDescriptor::of::<SetDefaultAgentVersion>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct RollbackAgentVersion {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
    /// Agent version's prefixed public identifier.
    pub version_id: AgentVersionId,
    pub req: RollbackAgentVersionRequest,
}

impl Command for RollbackAgentVersion {
    type Output = Agent;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "rollback_agent_version",
            category: "agents",
            description: "Copy an immutable version back into the editable agent draft.",
            method: "POST",
            path: "/v1/agents/{agent_id}/versions/{version_id}/rollback",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        let current = resolve_agent_for_mutation(ctx, &self.agent_id).await?;
        let version = resolve_agent_version(ctx, self.version_id).await?;
        if version.agent_id.uuid() != current.internal_id {
            return Err(CommandError::bad_request(
                "Agent version belongs to another agent",
            ));
        }
        let restored = q::version_to_agent(&current, &version);
        check_high_risk_caps(ctx, &restored.capabilities).await?;
        let restored_harness_id =
            resolve_update_harness_id(ctx, Some(restored.harness_id), None).await?;
        let row = ctx
            .db
            .update_agent(
                ctx.org_id(),
                AgentId::from_uuid(current.internal_id),
                UpdateAgent {
                    name: Some(restored.name.clone()),
                    display_name: restored.display_name.clone(),
                    description: restored.description.clone(),
                    system_prompt: Some(restored.system_prompt.clone()),
                    default_model_id: restored.default_model_id,
                    harness_id: restored_harness_id,
                    tags: Some(restored.tags.clone()),
                    initial_files: Some(serde_json::to_value(&restored.initial_files).unwrap()),
                    tools: Some(serde_json::to_value(&restored.tools).unwrap()),
                    mcp_servers: Some(serde_json::to_value(&restored.mcp_servers).unwrap()),
                    network_access: Some(
                        restored
                            .network_access
                            .as_ref()
                            .map(|value| serde_json::to_value(value).unwrap()),
                    ),
                    max_iterations: Some(
                        max_iterations::to_db(restored.max_iterations).map_err(classify_anyhow)?,
                    ),
                    parallel_tool_calls: Some(restored.parallel_tool_calls),
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))?;
        persist_capabilities(&ctx.db, current.internal_id, &restored.capabilities).await?;
        let caps = q::get_capabilities(&ctx.db, row.org_id, row.id.uuid())
            .await
            .map_err(classify_anyhow)?;
        let agent = q::row_to_agent(row, caps);
        if self.req.save_version {
            create_version_from_agent(
                ctx,
                &agent,
                AgentVersionChangeKind::Rollback,
                self.req
                    .summary
                    .or_else(|| Some(format!("Rollback to {}", version.version))),
                Some(version.public_id),
                true,
            )
            .await?;
        }
        Ok(agent)
    }
}

inventory::submit! { CommandDescriptor::of::<RollbackAgentVersion>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DiffAgentVersions {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
    pub from_version_id: AgentVersionId,
    pub to_version_id: AgentVersionId,
}

impl Command for DiffAgentVersions {
    type Output = AgentVersionDiffResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "diff_agent_versions",
            category: "agents",
            description: "Compare two immutable agent versions.",
            method: "GET",
            path: "/v1/agents/{agent_id}/versions/{from_version_id}/diff/{to_version_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AgentVersionDiffResponse, CommandError> {
        let agent = resolve_agent(ctx, &self.agent_id).await?;
        let from = resolve_agent_version(ctx, self.from_version_id).await?;
        let to = resolve_agent_version(ctx, self.to_version_id).await?;
        if from.agent_id.uuid() != agent.internal_id || to.agent_id.uuid() != agent.internal_id {
            return Err(CommandError::bad_request(
                "Agent version belongs to another agent",
            ));
        }
        Ok(AgentVersionDiffResponse {
            from_version_id: from.public_id,
            to_version_id: to.public_id,
            authored_diff: json_diff(&from.authored_config, &to.authored_config),
            resolved_diff: json_diff(&from.resolved_config, &to.resolved_config),
        })
    }
}

fn json_diff(from: &serde_json::Value, to: &serde_json::Value) -> serde_json::Value {
    let mut changes = serde_json::Map::new();
    if let (Some(a), Some(b)) = (from.as_object(), to.as_object()) {
        let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
        for key in keys {
            let before = a.get(key).cloned().unwrap_or(serde_json::Value::Null);
            let after = b.get(key).cloned().unwrap_or(serde_json::Value::Null);
            if before != after {
                changes.insert(
                    key.clone(),
                    serde_json::json!({ "from": before, "to": after }),
                );
            }
        }
        return serde_json::Value::Object(changes);
    }
    serde_json::json!({ "from": from, "to": to })
}

inventory::submit! { CommandDescriptor::of::<DiffAgentVersions>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ForkAgentVersion {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
    /// Agent version's prefixed public identifier.
    pub version_id: AgentVersionId,
    pub req: ForkAgentVersionRequest,
}

impl Command for ForkAgentVersion {
    type Output = Agent;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "fork_agent_version",
            category: "agents",
            description: "Fork an agent version into a new editable agent.",
            method: "POST",
            path: "/v1/agents/{agent_id}/versions/{version_id}/fork",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        validate_name("Agent", &self.req.name)?;
        q::ensure_name_available(&ctx.db, ctx.org_id(), &self.req.name, None).await?;
        let source = resolve_agent(ctx, &self.agent_id).await?;
        let version = resolve_agent_version(ctx, self.version_id).await?;
        if version.agent_id.uuid() != source.internal_id {
            return Err(CommandError::bad_request(
                "Agent version belongs to another agent",
            ));
        }
        let mut fork = q::version_to_agent(&source, &version);
        fork.name = self.req.name;
        fork.display_name = self.req.display_name;
        fork.description = self.req.description.or(fork.description);
        let created = CreateAgent(CreateAgentRequest {
            id: None,
            name: fork.name.clone(),
            display_name: fork.display_name.clone(),
            description: fork.description.clone(),
            system_prompt: fork.system_prompt.clone(),
            default_model_id: fork.default_model_id,
            harness_id: Some(fork.harness_id),
            harness_name: None,
            tags: fork.tags.clone(),
            capabilities: fork.capabilities.clone(),
            initial_files: fork.initial_files.clone(),
            tools: fork.tools.clone(),
            mcp_servers: fork.mcp_servers.clone(),
            network_access: fork.network_access.clone(),
            max_iterations: fork.max_iterations,
            parallel_tool_calls: fork.parallel_tool_calls,
        })
        .execute(ctx)
        .await?;
        let root_agent_id = source
            .root_agent_id
            .unwrap_or_else(|| AgentId::from_uuid(source.internal_id));
        let row = ctx
            .db
            .update_agent(
                ctx.org_id(),
                AgentId::from_uuid(created.internal_id),
                UpdateAgent {
                    forked_from_agent_id: Some(AgentId::from_uuid(source.internal_id)),
                    forked_from_version_id: Some(version.public_id),
                    root_agent_id: Some(root_agent_id),
                    ..Default::default()
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))?;
        let caps = q::get_capabilities(&ctx.db, row.org_id, row.id.uuid())
            .await
            .map_err(classify_anyhow)?;
        let agent = q::row_to_agent(row, caps);
        create_version_from_agent(
            ctx,
            &agent,
            AgentVersionChangeKind::Fork,
            Some(format!("Forked from {}", version.version)),
            Some(version.public_id),
            true,
        )
        .await?;
        Ok(agent)
    }
}

inventory::submit! { CommandDescriptor::of::<ForkAgentVersion>() }

// ============================================================================
// PreviewAgent
// ============================================================================

/// Preview the final agent shape with capabilities applied.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PreviewAgent {
    pub system_prompt: Option<String>,
    #[serde(default)]
    #[schema(value_type = Vec<everruns_platform::CapabilityRefSchema>)]
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
    /// Advisory tier-1 findings about the previewed config (knowledge/evaluation/agent-checks.md).
    pub findings: Vec<super::checks::Finding>,
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

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::agents::AGENT_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AgentPreview, CommandError> {
        crate::domains::mcp_servers::scoped_mcp::validate_scoped_mcp_servers(&self.mcp_servers)
            .map_err(classify_anyhow)?;
        let authored_prompt = self.system_prompt.unwrap_or_default();
        let (prompt, mut tools) = ctx
            .capability_service
            .preview(ctx.org_id(), &authored_prompt, &self.capabilities)
            .await
            .map_err(classify_anyhow)?;
        tools.extend(
            crate::domains::mcp_servers::scoped_mcp::build_scoped_mcp_tool_definitions(
                &self.mcp_servers,
                None,
                None,
                ctx.capability_service.egress_service().as_ref(),
            )
            .await
            .map_err(classify_anyhow)?,
        );
        tools.extend(self.tools);
        // Apply org rule config (phase 4): override built-in severities/enabled
        // and run custom declarative rules. Defaults to no-op when unconfigured.
        let rule_config = super::check_rules::load_effective_config(&ctx.db, ctx.org_id()).await;
        let builtin = super::checks::run_builtin_checks(
            &authored_prompt,
            &prompt,
            &self.capabilities,
            &tools,
        );
        let mut findings = super::checks::apply_rule_overrides(builtin, &rule_config.overrides);
        findings.extend(super::checks::run_declarative_rules(
            &rule_config.declarative,
            &prompt,
        ));
        Ok(AgentPreview {
            system_prompt: prompt,
            tools,
            findings,
        })
    }
}

inventory::submit! { CommandDescriptor::of::<PreviewAgent>() }

// ============================================================================
// AnalyzeAgent
// ============================================================================

/// Run advisory checks (built-in rules + LLM analysis) against an agent shape.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AnalyzeAgent {
    pub system_prompt: Option<String>,
    #[serde(default)]
    #[schema(value_type = Vec<everruns_platform::CapabilityRefSchema>)]
    pub capabilities: Vec<AgentCapabilityConfig>,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub mcp_servers: ScopedMcpServers,
}

#[derive(Debug, serde::Serialize)]
pub struct AgentAnalysis {
    pub findings: Vec<super::checks::Finding>,
}

impl Command for AnalyzeAgent {
    type Output = AgentAnalysis;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "analyze_agent",
            category: "agents",
            description: "Run advisory checks (built-in rules plus LLM analysis) against an \
                          agent configuration.",
            method: "POST",
            path: "/v1/agents/analyze",
        }
    }

    // Makes paid utility-LLM calls; not a free read.
    fn read_only() -> bool {
        false
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::agents::AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AgentAnalysis, CommandError> {
        let service = ctx
            .utility_llm_service
            .clone()
            .filter(|s| s.is_configured())
            .ok_or_else(|| {
                CommandError::bad_request(
                    "Agent analysis requires the system utility LLM service, which is not \
                     configured on this deployment",
                )
            })?;
        let analysis_permit =
            super::analysis::acquire_analysis_permit(&ctx.caller).map_err(|e| {
                let retry_after_seconds = match e {
                    super::analysis::AnalysisAdmissionError::RateLimited {
                        retry_after_seconds,
                    }
                    | super::analysis::AnalysisAdmissionError::Busy {
                        retry_after_seconds,
                    } => retry_after_seconds,
                };
                CommandError::rate_limited(e.to_string())
                    .with_code("agent_analysis_rate_limited")
                    .with_retry_after(retry_after_seconds)
            })?;
        let authored_prompt = self.system_prompt.clone().unwrap_or_default();
        let preview = PreviewAgent {
            system_prompt: self.system_prompt,
            capabilities: self.capabilities,
            tools: self.tools,
            mcp_servers: self.mcp_servers,
        }
        .execute(ctx)
        .await?;
        // Oversized input is a client error (the prompt/tool descriptions are
        // too large) — reject it as a 4xx before issuing any paid LLM call.
        super::analysis::ensure_analysis_input_within_limit(
            &authored_prompt,
            &preview.system_prompt,
            &preview.tools,
        )
        .map_err(CommandError::bad_request)?;
        let llm_findings = super::analysis::run_llm_checks(
            service.clone(),
            &authored_prompt,
            &preview.system_prompt,
            &preview.tools,
        )
        .await
        .map_err(|e| {
            let safe = super::safe_agent_check_error(&e);
            CommandError::unprocessable(safe.fallback_message()).with_code(safe.code)
        })?;
        drop(analysis_permit);
        let mut findings = preview.findings;
        findings.extend(llm_findings);
        // Custom NL-rubric rules (phase 4) are judged by the utility LLM too.
        let rule_config = super::check_rules::load_effective_config(&ctx.db, ctx.org_id()).await;
        if !rule_config.nl_rubric.is_empty() {
            findings.extend(
                super::analysis::run_custom_nl_rules(
                    service,
                    &rule_config.nl_rubric,
                    &preview.system_prompt,
                )
                .await,
            );
        }
        Ok(AgentAnalysis { findings })
    }
}

inventory::submit! { CommandDescriptor::of::<AnalyzeAgent>() }

// ============================================================================
// CheckAgentName
// ============================================================================

/// Check whether an agent name is available.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckAgentName {
    /// Human-readable name. Safe to render in user-facing messages.
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
        if everruns_platform::validate_addressable_name(&self.name).is_err() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_imports::{
        AgentLoopError, Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, DefaultPermissionResolver,
        LlmResponse, LlmResponseStream, OrgRole, Result as CoreResult, UtilityLlmRequest,
        UtilityLlmService,
    };
    use crate::services::CapabilityService;
    use crate::storage::StorageBackend;
    use crate::storage::models::CreateHarnessRow;
    use async_trait::async_trait;
    use everruns_platform::FeatureFlags;
    use std::sync::Arc;
    use uuid::Uuid;

    #[test]
    fn first_agent_version_is_initial_minor_even_for_major_change() {
        let (major, minor, patch, version) =
            bump_published_version(None, &AgentVersionChangeKind::Major);

        assert_eq!((major, minor, patch), (0, 1, 0));
        assert_eq!(version, "0.1.0");
    }

    fn ctx_with_role_and_flags(
        db: Arc<StorageBackend>,
        role: OrgRole,
        feature_flags: FeatureFlags,
    ) -> Ctx {
        futures::executor::block_on(crate::org_init::initialize_org_harnesses(
            &db,
            DEFAULT_ORG_ID,
        ))
        .expect("initialize built-in harnesses for agent command tests");
        let capability_service = Arc::new(CapabilityService::new(db.clone(), None));
        Ctx::new(
            Caller {
                org_id: DEFAULT_ORG_ID,
                org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
                user_id: Some(Uuid::nil()),
                role,
                is_platform_user: false,
                is_internal: false,
            },
            db,
            capability_service,
            None,
            Arc::new(DefaultPermissionResolver),
        )
        .with_feature_flags(feature_flags)
    }

    fn ctx_with_role(db: Arc<StorageBackend>, role: OrgRole) -> Ctx {
        ctx_with_role_and_flags(
            db,
            role,
            FeatureFlags {
                agent_versions: true,
                ..FeatureFlags::default()
            },
        )
    }

    struct ExhaustedUtilityLlm;

    #[async_trait]
    impl UtilityLlmService for ExhaustedUtilityLlm {
        fn is_configured(&self) -> bool {
            true
        }

        async fn chat_completion(&self, _request: UtilityLlmRequest) -> CoreResult<LlmResponse> {
            Err(AgentLoopError::llm(
                "credit_balance_exhausted: credits depleted; api_key=secret",
            ))
        }

        async fn chat_completion_stream(
            &self,
            _request: UtilityLlmRequest,
        ) -> CoreResult<LlmResponseStream> {
            unreachable!("agent analysis does not stream")
        }
    }

    #[tokio::test]
    async fn analyze_maps_provider_quota_failure_to_safe_actionable_error() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db, OrgRole::Owner)
            .with_utility_llm_service(Arc::new(ExhaustedUtilityLlm));

        let error = AnalyzeAgent {
            system_prompt: Some("Be helpful.".to_string()),
            capabilities: Vec::new(),
            tools: Vec::new(),
            mcp_servers: Default::default(),
        }
        .execute(&ctx)
        .await
        .expect_err("exhausted provider must fail analysis");

        let (status, axum::Json(body)): (
            axum::http::StatusCode,
            axum::Json<crate::api::common::ErrorResponse>,
        ) = error.into();
        assert_eq!(status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body.code.as_deref(), Some("provider_quota_exhausted"));
        let detail = body.detail.expect("safe actionable detail");
        assert!(detail.contains("out of credits or quota"));
        assert!(!detail.contains("api_key"));
        assert!(!detail.contains("secret"));
    }

    fn high_risk_agent_request(name: String) -> CreateAgentRequest {
        CreateAgentRequest {
            id: None,
            name,
            display_name: None,
            description: None,
            system_prompt: "test".to_string(),
            default_model_id: None,
            harness_id: None,
            harness_name: None,
            tags: Vec::new(),
            capabilities: vec![AgentCapabilityConfig::new("bashkit_shell")],
            initial_files: Vec::new(),
            tools: Vec::new(),
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
        }
    }

    fn basic_agent_request(name: &str) -> CreateAgentRequest {
        CreateAgentRequest {
            id: None,
            name: name.to_string(),
            display_name: None,
            description: None,
            system_prompt: "initial prompt".to_string(),
            default_model_id: None,
            harness_id: None,
            harness_name: None,
            tags: Vec::new(),
            capabilities: Vec::new(),
            initial_files: Vec::new(),
            tools: Vec::new(),
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
        }
    }

    fn update_prompt_request(system_prompt: &str) -> UpdateAgentRequest {
        UpdateAgentRequest {
            name: None,
            display_name: None,
            description: None,
            system_prompt: Some(system_prompt.to_string()),
            default_model_id: None,
            harness_id: None,
            harness_name: None,
            tags: None,
            capabilities: None,
            initial_files: None,
            status: None,
            tools: None,
            mcp_servers: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
        }
    }

    async fn create_test_harness(db: &StorageBackend, name: &str) -> HarnessId {
        db.create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: name.to_string(),
                display_name: Some(name.to_string()),
                description: None,
                system_prompt: Some("test harness".to_string()),
                parent_harness_id: None,
                default_model_id: None,
                tags: Vec::new(),
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                embedder_metadata: serde_json::json!({}),
                is_built_in: false,
            },
        )
        .await
        .expect("create test harness")
        .id
    }

    #[tokio::test]
    async fn create_agent_defaults_to_generic_harness() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
        let generic_id = crate::org_init::generic_harness_id(&db, DEFAULT_ORG_ID)
            .await
            .expect("generic harness id");

        let created = CreateAgent(basic_agent_request("default-harness-agent"))
            .run(&ctx)
            .await
            .expect("agent is created");

        assert_eq!(created.harness_id, generic_id);
    }

    #[tokio::test]
    async fn create_and_update_agent_resolve_harness_name_and_id() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
        let first_harness_id = create_test_harness(&db, "agent-harness-one").await;
        let second_harness_id = create_test_harness(&db, "agent-harness-two").await;

        let mut req = basic_agent_request("named-harness-agent");
        req.harness_name = Some("agent-harness-one".to_string());
        let created = CreateAgent(req).run(&ctx).await.expect("agent is created");
        assert_eq!(created.harness_id, first_harness_id);

        let renamed = UpdateAgentCmd {
            id: created.public_id.to_string(),
            req: UpdateAgentRequest {
                name: None,
                display_name: Some("renamed".to_string()),
                description: None,
                system_prompt: None,
                default_model_id: None,
                harness_id: None,
                harness_name: None,
                tags: None,
                capabilities: None,
                initial_files: None,
                status: None,
                tools: None,
                mcp_servers: None,
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        }
        .run(&ctx)
        .await
        .expect("agent is updated without harness change");
        assert_eq!(renamed.harness_id, first_harness_id);

        let changed = UpdateAgentCmd {
            id: created.public_id.to_string(),
            req: UpdateAgentRequest {
                harness_id: Some(second_harness_id),
                ..update_prompt_request("changed harness")
            },
        }
        .run(&ctx)
        .await
        .expect("agent harness changes by id");
        assert_eq!(changed.harness_id, second_harness_id);
    }

    #[tokio::test]
    async fn create_agent_rejects_unknown_archived_or_ambiguous_harness() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db.clone(), OrgRole::Owner);

        let mut unknown = basic_agent_request("unknown-harness-agent");
        unknown.harness_id = Some(HarnessId::new());
        let err = CreateAgent(unknown)
            .run(&ctx)
            .await
            .expect_err("unknown harness is rejected");
        assert_eq!(err.status(), axum::http::StatusCode::NOT_FOUND);

        let archived_id = create_test_harness(&db, "archived-agent-harness").await;
        db.delete_harness(DEFAULT_ORG_ID, archived_id)
            .await
            .expect("archive harness");
        let mut archived = basic_agent_request("archived-harness-agent");
        archived.harness_id = Some(archived_id);
        let err = CreateAgent(archived)
            .run(&ctx)
            .await
            .expect_err("archived harness is rejected");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);

        let mut ambiguous = basic_agent_request("ambiguous-harness-agent");
        ambiguous.harness_id = Some(archived_id);
        ambiguous.harness_name = Some("generic".to_string());
        let err = CreateAgent(ambiguous)
            .run(&ctx)
            .await
            .expect_err("ambiguous harness selection is rejected");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    async fn create_high_risk_agent_version(ctx: &Ctx, name: &str) -> (Agent, AgentVersion) {
        let agent = CreateAgent(high_risk_agent_request(name.to_string()))
            .run(ctx)
            .await
            .expect("admin can create high-risk agent");
        let version = CreateAgentVersionCmd {
            agent_id: agent.public_id.to_string(),
            req: CreateAgentVersionRequest {
                summary: None,
                change_kind: None,
            },
        }
        .run(ctx)
        .await
        .expect("admin can create high-risk version");
        (agent, version)
    }

    #[tokio::test]
    async fn update_agent_creates_unpublished_auto_snapshot() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
        let agent = CreateAgent(basic_agent_request("auto-snapshot-agent"))
            .run(&ctx)
            .await
            .expect("agent is created");

        UpdateAgentCmd {
            id: agent.public_id.to_string(),
            req: update_prompt_request("updated prompt"),
        }
        .run(&ctx)
        .await
        .expect("agent is updated");

        let snapshots = ListAgentVersions {
            agent_id: agent.public_id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("versions are listed");
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].change_kind, AgentVersionChangeKind::Auto);
        assert!(!snapshots[0].is_published);
        assert_eq!(snapshots[0].version, "draft.1");
        assert_eq!(
            snapshots[0].authored_config["system_prompt"],
            "updated prompt"
        );

        let published = CreateAgentVersionCmd {
            agent_id: agent.public_id.to_string(),
            req: CreateAgentVersionRequest {
                summary: Some("publish current draft".to_string()),
                change_kind: None,
            },
        }
        .run(&ctx)
        .await
        .expect("published version is created");
        assert!(published.is_published);
        assert_eq!(published.version, "0.1.0");
        assert_eq!(published.version_number, 2);

        let latest_published = db
            .get_latest_agent_version(DEFAULT_ORG_ID, AgentId::from_uuid(agent.internal_id))
            .await
            .expect("latest published lookup succeeds")
            .expect("published version exists");
        assert_eq!(latest_published.id, published.public_id);
    }

    #[tokio::test]
    async fn update_agent_updates_parallel_tool_calls() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db, OrgRole::Owner);
        let agent = CreateAgent(basic_agent_request("parallel-tool-calls-agent"))
            .run(&ctx)
            .await
            .expect("agent is created");

        let mut req = update_prompt_request("enable parallel tool calls");
        req.parallel_tool_calls = Some(true);
        let updated = UpdateAgentCmd {
            id: agent.public_id.to_string(),
            req,
        }
        .run(&ctx)
        .await
        .expect("agent enables parallel tool calls");
        assert_eq!(updated.parallel_tool_calls, Some(true));

        let mut req = update_prompt_request("disable parallel tool calls");
        req.parallel_tool_calls = Some(false);
        let updated = UpdateAgentCmd {
            id: agent.public_id.to_string(),
            req,
        }
        .run(&ctx)
        .await
        .expect("agent disables parallel tool calls");
        assert_eq!(updated.parallel_tool_calls, Some(false));
    }

    #[tokio::test]
    async fn update_agent_skips_auto_snapshot_when_versions_disabled() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role_and_flags(db.clone(), OrgRole::Owner, FeatureFlags::default());
        let agent = CreateAgent(basic_agent_request("auto-snapshot-disabled-agent"))
            .run(&ctx)
            .await
            .expect("agent is created");

        UpdateAgentCmd {
            id: agent.public_id.to_string(),
            req: update_prompt_request("updated prompt"),
        }
        .run(&ctx)
        .await
        .expect("agent is updated");

        let snapshots = db
            .list_agent_versions(DEFAULT_ORG_ID, AgentId::from_uuid(agent.internal_id))
            .await
            .expect("stored versions are listed");
        assert!(snapshots.is_empty());
    }

    #[tokio::test]
    async fn update_agent_skips_auto_snapshot_for_unchanged_config() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db, OrgRole::Owner);
        let agent = CreateAgent(basic_agent_request("auto-snapshot-noop-agent"))
            .run(&ctx)
            .await
            .expect("agent is created");

        UpdateAgentCmd {
            id: agent.public_id.to_string(),
            req: UpdateAgentRequest {
                name: None,
                display_name: None,
                description: None,
                system_prompt: None,
                default_model_id: None,
                harness_id: None,
                harness_name: None,
                tags: None,
                capabilities: None,
                initial_files: None,
                status: None,
                tools: None,
                mcp_servers: None,
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        }
        .run(&ctx)
        .await
        .expect("no-op update succeeds");

        let snapshots = ListAgentVersions {
            agent_id: agent.public_id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("versions are listed");
        assert!(snapshots.is_empty());
    }

    #[tokio::test]
    async fn update_agent_prunes_old_auto_snapshots() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db, OrgRole::Owner);
        let agent = CreateAgent(basic_agent_request("auto-snapshot-pruned-agent"))
            .run(&ctx)
            .await
            .expect("agent is created");

        for i in 0..(MAX_AUTO_SNAPSHOTS_PER_AGENT + 2) {
            UpdateAgentCmd {
                id: agent.public_id.to_string(),
                req: update_prompt_request(&format!("updated prompt {i}")),
            }
            .run(&ctx)
            .await
            .expect("agent is updated");
        }

        let snapshots = ListAgentVersions {
            agent_id: agent.public_id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("versions are listed");
        assert_eq!(snapshots.len(), MAX_AUTO_SNAPSHOTS_PER_AGENT as usize);
        assert_eq!(snapshots[0].version, "draft.52");
        assert_eq!(snapshots.last().unwrap().version, "draft.3");
    }

    #[tokio::test]
    async fn upsert_agent_update_creates_unpublished_auto_snapshot() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db, OrgRole::Owner);
        let agent_id = AgentId::new();
        let mut req = basic_agent_request("upsert-auto-snapshot-agent");
        req.system_prompt = "initial upsert prompt".to_string();

        let created = UpsertAgent {
            id: agent_id.to_string(),
            req: req.clone(),
        }
        .run(&ctx)
        .await
        .expect("agent is created by upsert");
        assert!(created.was_created);

        req.system_prompt = "updated upsert prompt".to_string();
        let updated = UpsertAgent {
            id: agent_id.to_string(),
            req,
        }
        .run(&ctx)
        .await
        .expect("agent is updated by upsert");
        assert!(!updated.was_created);

        let snapshots = ListAgentVersions {
            agent_id: agent_id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("versions are listed");
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].change_kind, AgentVersionChangeKind::Auto);
        assert!(!snapshots[0].is_published);
        assert_eq!(
            snapshots[0].authored_config["system_prompt"],
            "updated upsert prompt"
        );
    }

    #[tokio::test]
    async fn upsert_agent_skips_auto_snapshot_for_unchanged_config() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db, OrgRole::Owner);
        let agent_id = AgentId::new();
        let req = basic_agent_request("upsert-auto-snapshot-noop-agent");

        UpsertAgent {
            id: agent_id.to_string(),
            req: req.clone(),
        }
        .run(&ctx)
        .await
        .expect("agent is created by upsert");
        UpsertAgent {
            id: agent_id.to_string(),
            req,
        }
        .run(&ctx)
        .await
        .expect("agent is upserted without changes");

        let snapshots = ListAgentVersions {
            agent_id: agent_id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("versions are listed");
        assert!(snapshots.is_empty());
    }

    #[tokio::test]
    async fn create_agent_version_rejects_auto_change_kind() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db, OrgRole::Owner);
        let agent = CreateAgent(basic_agent_request("manual-auto-kind-agent"))
            .run(&ctx)
            .await
            .expect("agent is created");

        let err = CreateAgentVersionCmd {
            agent_id: agent.public_id.to_string(),
            req: CreateAgentVersionRequest {
                summary: None,
                change_kind: Some(AgentVersionChangeKind::Auto),
            },
        }
        .run(&ctx)
        .await
        .expect_err("manual publish cannot use automatic change kind");

        assert!(
            matches!(
                err,
                CommandError {
                    kind: CommandErrorKind::BadRequest(_),
                    ..
                }
            ),
            "expected BadRequest, got {err:?}"
        );
    }

    #[tokio::test]
    async fn set_default_version_rejects_unpublished_snapshot() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db, OrgRole::Owner);
        let agent = CreateAgent(basic_agent_request("default-draft-agent"))
            .run(&ctx)
            .await
            .expect("agent is created");
        UpdateAgentCmd {
            id: agent.public_id.to_string(),
            req: update_prompt_request("draft-only prompt"),
        }
        .run(&ctx)
        .await
        .expect("agent is updated");
        let snapshots = ListAgentVersions {
            agent_id: agent.public_id.to_string(),
        }
        .run(&ctx)
        .await
        .expect("versions are listed");

        let err = SetDefaultAgentVersion {
            agent_id: agent.public_id.to_string(),
            req: SetDefaultAgentVersionRequest {
                version_id: snapshots[0].public_id,
            },
        }
        .run(&ctx)
        .await
        .expect_err("draft snapshot cannot become default");

        assert!(
            matches!(
                err,
                CommandError {
                    kind: CommandErrorKind::BadRequest(_),
                    ..
                }
            ),
            "expected BadRequest, got {err:?}"
        );
    }

    #[tokio::test]
    async fn set_default_version_blocks_member_for_high_risk_capabilities() {
        let db = Arc::new(StorageBackend::in_memory());
        let admin_ctx = ctx_with_role(db.clone(), OrgRole::Admin);
        let member_ctx = ctx_with_role(db, OrgRole::Member);
        let (agent, version) =
            create_high_risk_agent_version(&admin_ctx, "member-set-default-denied").await;

        let err = SetDefaultAgentVersion {
            agent_id: agent.public_id.to_string(),
            req: SetDefaultAgentVersionRequest {
                version_id: version.public_id,
            },
        }
        .run(&member_ctx)
        .await
        .expect_err("member must not activate high-risk version");

        assert!(
            matches!(
                err,
                CommandError {
                    kind: CommandErrorKind::Forbidden(_),
                    ..
                }
            ),
            "expected Forbidden, got {err:?}"
        );

        SetDefaultAgentVersion {
            agent_id: agent.public_id.to_string(),
            req: SetDefaultAgentVersionRequest {
                version_id: version.public_id,
            },
        }
        .run(&admin_ctx)
        .await
        .expect("admin can activate high-risk version");
    }

    #[tokio::test]
    async fn rollback_version_restores_versioned_harness() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
        let first_harness_id = create_test_harness(&db, "rollback-harness-one").await;
        let second_harness_id = create_test_harness(&db, "rollback-harness-two").await;

        let mut req = basic_agent_request("rollback-harness-agent");
        req.harness_id = Some(first_harness_id);
        let agent = CreateAgent(req).run(&ctx).await.expect("agent is created");
        let version = CreateAgentVersionCmd {
            agent_id: agent.public_id.to_string(),
            req: CreateAgentVersionRequest {
                summary: Some("save first harness".to_string()),
                change_kind: None,
            },
        }
        .run(&ctx)
        .await
        .expect("version is created");

        let changed = UpdateAgentCmd {
            id: agent.public_id.to_string(),
            req: UpdateAgentRequest {
                harness_id: Some(second_harness_id),
                ..update_prompt_request("switch to second harness")
            },
        }
        .run(&ctx)
        .await
        .expect("agent harness changes");
        assert_eq!(changed.harness_id, second_harness_id);

        let restored = RollbackAgentVersion {
            agent_id: agent.public_id.to_string(),
            version_id: version.public_id,
            req: RollbackAgentVersionRequest {
                save_version: false,
                summary: None,
            },
        }
        .run(&ctx)
        .await
        .expect("rollback succeeds");

        assert_eq!(restored.harness_id, first_harness_id);
    }

    #[tokio::test]
    async fn rollback_version_blocks_member_for_high_risk_capabilities() {
        let db = Arc::new(StorageBackend::in_memory());
        let owner_ctx = ctx_with_role(db.clone(), OrgRole::Owner);
        let member_ctx = ctx_with_role(db, OrgRole::Member);
        let (agent, version) =
            create_high_risk_agent_version(&owner_ctx, "member-rollback-denied").await;

        let err = RollbackAgentVersion {
            agent_id: agent.public_id.to_string(),
            version_id: version.public_id,
            req: RollbackAgentVersionRequest {
                save_version: false,
                summary: None,
            },
        }
        .run(&member_ctx)
        .await
        .expect_err("member must not roll back to high-risk version");

        assert!(
            matches!(
                err,
                CommandError {
                    kind: CommandErrorKind::Forbidden(_),
                    ..
                }
            ),
            "expected Forbidden, got {err:?}"
        );

        RollbackAgentVersion {
            agent_id: agent.public_id.to_string(),
            version_id: version.public_id,
            req: RollbackAgentVersionRequest {
                save_version: false,
                summary: None,
            },
        }
        .run(&owner_ctx)
        .await
        .expect("owner can roll back to high-risk version");
    }

    #[tokio::test]
    async fn agent_creation_rejected_at_limit_and_allowed_below() {
        let db = Arc::new(StorageBackend::in_memory());
        let mut ctx = ctx_with_role(db, OrgRole::Owner);
        ctx.resource_limits.max_agents_per_org = 2;

        CreateAgent(basic_agent_request("a1"))
            .execute(&ctx)
            .await
            .expect("first agent below limit");
        CreateAgent(basic_agent_request("a2"))
            .execute(&ctx)
            .await
            .expect("second agent at limit boundary");

        let err = CreateAgent(basic_agent_request("a3"))
            .execute(&ctx)
            .await
            .expect_err("third agent exceeds the cap");
        assert_eq!(err.status().as_u16(), 409);
        assert!(err.message().contains("Agent limit reached"));
    }

    #[tokio::test]
    async fn soft_deleted_agents_do_not_count_toward_limit() {
        let db = Arc::new(StorageBackend::in_memory());
        let mut ctx = ctx_with_role(db, OrgRole::Owner);
        ctx.resource_limits.max_agents_per_org = 1;

        let a1 = CreateAgent(basic_agent_request("a1"))
            .execute(&ctx)
            .await
            .expect("first agent below limit");

        let err = CreateAgent(basic_agent_request("a2"))
            .execute(&ctx)
            .await
            .expect_err("second agent exceeds the cap");
        assert_eq!(err.status().as_u16(), 409);

        // Archive then mark deleted; a deleted row must not count toward the cap.
        DeleteAgent {
            id: a1.public_id.to_string(),
        }
        .execute(&ctx)
        .await
        .expect("archive a1");
        DestroyAgent {
            id: a1.public_id.to_string(),
        }
        .execute(&ctx)
        .await
        .expect("mark a1 deleted");

        CreateAgent(basic_agent_request("a2"))
            .execute(&ctx)
            .await
            .expect("creation allowed once the deleted agent is excluded");
    }

    // ========================================================================
    // Built-in agent protection (EVE-865)
    // ========================================================================

    /// Create an agent and flip it to built-in, as org bootstrap does.
    async fn seed_built_in_agent(db: &StorageBackend, ctx: &Ctx, name: &str) -> Agent {
        let agent = CreateAgent(basic_agent_request(name))
            .execute(ctx)
            .await
            .expect("seed agent");
        db.mark_agent_built_in(DEFAULT_ORG_ID, AgentId::from_uuid(agent.internal_id))
            .await
            .expect("mark built-in");
        agent
    }

    /// Every guard must answer 400 with the copy-first hint, not 404 or 500.
    fn assert_built_in_rejection(err: &CommandError) {
        assert_eq!(err.status().as_u16(), 400, "message: {}", err.message());
        assert!(
            err.message().contains("built-in agent"),
            "expected a built-in rejection, got: {}",
            err.message()
        );
        assert!(
            err.message().contains("Copy it first"),
            "rejection must point at the escape hatch, got: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn built_in_agent_rejects_update() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
        let agent = seed_built_in_agent(&db, &ctx, "chat").await;

        let err = UpdateAgentCmd {
            id: agent.public_id.to_string(),
            req: UpdateAgentRequest {
                system_prompt: Some("hijacked".to_string()),
                ..Default::default()
            },
        }
        .execute(&ctx)
        .await
        .expect_err("built-in definition is immutable");
        assert_built_in_rejection(&err);

        // The prompt must actually be unchanged, not merely reported as such.
        let after = q::get_by_public_id(&db, DEFAULT_ORG_ID, &agent.public_id.to_string())
            .await
            .expect("reload")
            .expect("agent still present");
        assert_eq!(after.system_prompt, "initial prompt");
    }

    #[tokio::test]
    async fn built_in_agent_rejects_archive() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
        let agent = seed_built_in_agent(&db, &ctx, "chat").await;

        // Archive travels through UpdateAgentCmd as a status change.
        let err = UpdateAgentCmd {
            id: agent.public_id.to_string(),
            req: UpdateAgentRequest {
                status: Some(AgentStatus::Archived),
                ..Default::default()
            },
        }
        .execute(&ctx)
        .await
        .expect_err("built-in agent cannot be archived");
        assert_built_in_rejection(&err);
    }

    #[tokio::test]
    async fn built_in_agent_rejects_delete_and_destroy() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
        let agent = seed_built_in_agent(&db, &ctx, "chat").await;

        let err = DeleteAgent {
            id: agent.public_id.to_string(),
        }
        .execute(&ctx)
        .await
        .expect_err("built-in agent cannot be archived away");
        assert_built_in_rejection(&err);

        let err = DestroyAgent {
            id: agent.public_id.to_string(),
        }
        .execute(&ctx)
        .await
        .expect_err("built-in agent cannot be destroyed");
        assert_built_in_rejection(&err);

        assert!(
            q::get_by_public_id(&db, DEFAULT_ORG_ID, &agent.public_id.to_string())
                .await
                .expect("reload")
                .is_some(),
            "built-in agent must survive both delete verbs"
        );
    }

    #[tokio::test]
    async fn built_in_agent_rejects_upsert() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
        let agent = seed_built_in_agent(&db, &ctx, "chat").await;

        let err = UpsertAgent {
            id: agent.public_id.to_string(),
            req: basic_agent_request("chat"),
        }
        .execute(&ctx)
        .await
        .expect_err("upsert must not overwrite a built-in");
        assert_built_in_rejection(&err);
    }

    #[tokio::test]
    async fn built_in_agent_rejects_version_mutations() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
        let agent = seed_built_in_agent(&db, &ctx, "chat").await;
        let id = agent.public_id.to_string();

        let err = CreateAgentVersionCmd {
            agent_id: id.clone(),
            req: CreateAgentVersionRequest {
                change_kind: Some(AgentVersionChangeKind::Minor),
                summary: None,
            },
        }
        .execute(&ctx)
        .await
        .expect_err("versions are part of the protected definition");
        assert_built_in_rejection(&err);

        let err = SetDefaultAgentVersion {
            agent_id: id.clone(),
            req: SetDefaultAgentVersionRequest {
                version_id: AgentVersionId::new(),
            },
        }
        .execute(&ctx)
        .await
        .expect_err("default version is part of the protected definition");
        assert_built_in_rejection(&err);

        let err = RollbackAgentVersion {
            agent_id: id,
            version_id: AgentVersionId::new(),
            req: RollbackAgentVersionRequest {
                save_version: false,
                summary: None,
            },
        }
        .execute(&ctx)
        .await
        .expect_err("rollback rewrites the protected definition");
        assert_built_in_rejection(&err);
    }

    #[tokio::test]
    async fn built_in_agent_can_be_copied() {
        let db = Arc::new(StorageBackend::in_memory());
        let ctx = ctx_with_role(db.clone(), OrgRole::Owner);
        let agent = seed_built_in_agent(&db, &ctx, "chat").await;

        let copy = CopyAgent {
            id: agent.public_id.to_string(),
        }
        .execute(&ctx)
        .await
        .expect("copy is the escape hatch and must stay open");

        assert_ne!(copy.public_id, agent.public_id);

        // The copy must be editable, or the escape hatch leads nowhere.
        UpdateAgentCmd {
            id: copy.public_id.to_string(),
            req: UpdateAgentRequest {
                system_prompt: Some("edited".to_string()),
                ..Default::default()
            },
        }
        .execute(&ctx)
        .await
        .expect("the copy is an ordinary editable agent");
    }

    #[tokio::test]
    async fn built_in_agents_do_not_count_toward_limit() {
        let db = Arc::new(StorageBackend::in_memory());
        let mut ctx = ctx_with_role(db.clone(), OrgRole::Owner);
        ctx.resource_limits.max_agents_per_org = 1;

        seed_built_in_agent(&db, &ctx, "platform-chat").await;

        // The built-in must not consume the cap, so a user agent still fits.
        CreateAgent(basic_agent_request("mine"))
            .execute(&ctx)
            .await
            .expect("built-in agent must not consume the org quota");
    }
}

// ============================================================================
// DestroyAgent (hard delete)
// ============================================================================

/// Permanently delete an archived agent.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DestroyAgent {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
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

        q::ensure_not_built_in(&ctx.db, ctx.org_id(), &self.id, "delete").await?;

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

        crate::domains::apps::queries::ensure_no_app_references_to_agent(
            &ctx.db,
            ctx.org_id(),
            row.id.uuid(),
        )
        .await?;

        ctx.db
            .destroy_agent(ctx.org_id(), row.id)
            .await
            .map_err(classify_anyhow)?;

        Ok(serde_json::json!({"destroyed": true}))
    }
}

inventory::submit! { CommandDescriptor::of::<DestroyAgent>() }

// Agent commands — user-facing operations.
// Request types double as catalog entries and auto-register with inventory.

use super::command_validation::{
    normalize_capability_refs, reject_sandbox_override_for_fixed_harness,
    validate_sandbox_template_sources,
};
pub(crate) use super::managed::check_harness_assignment;
use super::managed::{
    check_high_risk_caps, resolve_create_harness_id, resolve_update_harness_id,
    validate_managed_name,
};
use super::preview::PreviewAgent;
use super::queries as q;
use super::sandbox_policy as sandbox_templates;
use super::types::{AgentRow, CreateAgentRequest, CreateAgentRow, UpdateAgent, UpdateAgentRequest};
use super::{AGENT_DANGEROUS, AGENT_MANAGE, AGENT_VIEW};
use crate::domains::common::*;
use crate::kernel_imports::{
    AgentCapabilityConfig, InitialFile, ScopedMcpServers, contracts::tool_types::ToolDefinition,
};
use crate::records::{Agent, AgentStatus};
use crate::{max_iterations, storage::UpdateField as StorageUpdate};
use everruns_contracts::typed_id::{AgentId, HarnessId};
use serde::Deserialize;
use utoipa::ToSchema;

// Input validation

use crate::domains::validation::{
    MAX_AGENT_CAPABILITIES, MAX_AGENT_DESCRIPTION_BYTES, MAX_AGENT_NAME_BYTES,
    MAX_AGENT_SYSTEM_PROMPT_BYTES, MAX_INITIAL_FILES, MAX_INITIAL_FILES_TOTAL_BYTES,
    check_platform_chat_content,
};

// Shared persistence helpers

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
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))
}

// CreateAgent

/// Create a new agent with a name, system prompt, and optional capabilities.
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct CreateAgent(pub CreateAgentRequest);

impl CommandSchema for CreateAgent {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateAgentRequest>()
    }
}

#[command(
    name = "create_agent",
    category = "agents",
    description = "Create a new agent with a name, system prompt, and optional capabilities. Agent name must contain only lowercase letters, digits, and hyphens (e.g. joke-telling-agent).",
    method = "POST",
    path = "/v1/agents",
    policy = AGENT_MANAGE,
    cli = CliRoute::new(&["agents"], "create").with_args(&[CliArg::new("harness_name").short('H').long("harness"), CliArg::new("tag").short('t'),]).with_examples(&[CliExample::new("Create an agent on the organization's default harness", "everruns agents create --name triage --system-prompt 'Triage incoming issues' --harness conversation --reason 'Set up issue triage for support'",)]),
)]
impl Command for CreateAgent {
    type Output = Agent;

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        let req = self.0;

        // Validate
        validate_name("Agent", &req.name)?;
        validate_managed_name(&req.name)?;
        validate_create_limits(&req)?;
        sandbox_templates::validate(req.sandbox_policy.as_ref())?;
        validate_sandbox_template_sources(ctx, req.sandbox_policy.as_ref()).await?;
        check_high_risk_caps(ctx, &req.capabilities).await?;

        // Enforce per-org agent cap (excludes soft-deleted) before insert.
        let max = ctx.resource_limits.max_agents_per_org;
        let count = ctx.db.count_agents_for_org(ctx.org_id()).await?;
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
        .await?;
        crate::domains::mcp_servers::scoped_mcp::validate_scoped_mcp_servers_for_org(
            &ctx.db,
            ctx.org_id(),
            &req.mcp_servers,
        )
        .await?;
        let default_model_id =
            q::validate_model_id(&ctx.db, ctx.org_id(), req.default_model_id).await?;
        let harness_source = if req.harness_id.is_some() || req.harness_name.is_some() {
            "explicit"
        } else {
            "organization_default"
        };
        let harness_id =
            resolve_create_harness_id(ctx, req.harness_id, req.harness_name.as_deref()).await?;
        reject_sandbox_override_for_fixed_harness(ctx, harness_id, req.sandbox_policy.is_some())
            .await?;

        validate_service_account(ctx, req.service_virtual_user_id).await?;
        // Persist
        let client_id = req.id;
        let (row, agent_uuid) = if let Some(client_id) = client_id {
            let input = CreateAgentRow {
                public_id: client_id.to_string(),
                name: req.name,
                display_name: req.display_name,
                description: req.description,
                intro_markdown: req.intro_markdown,
                short_description: req.short_description,
                starters: serde_json::to_value(&req.starters).unwrap_or(serde_json::json!([])),
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
                max_iterations: max_iterations::to_db(req.max_iterations)?,
                parallel_tool_calls: req.parallel_tool_calls,
                environments: sandbox_templates::to_json(req.sandbox_policy.as_ref()),
                // Built-in agents come from the platform definition via org
                // bootstrap. No API-facing creation path may mint one.
                is_built_in: false,
            };
            let row = ctx.db.create_agent(ctx.org_id(), input).await?;
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
                intro_markdown: req.intro_markdown,
                short_description: req.short_description,
                starters: serde_json::to_value(&req.starters).unwrap_or(serde_json::json!([])),
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
                max_iterations: max_iterations::to_db(req.max_iterations)?,
                parallel_tool_calls: req.parallel_tool_calls,
                environments: sandbox_templates::to_json(req.sandbox_policy.as_ref()),
                // Built-in agents come from the platform definition via org
                // bootstrap. No API-facing creation path may mint one.
                is_built_in: false,
            };
            let row = ctx
                .db
                .create_agent_with_id(ctx.org_id(), AgentId::from_uuid(internal_uuid), input)
                .await?
                .ok_or_else(|| {
                    CommandError::conflict("Agent UUID collision").with_code("agent_id_taken")
                })?;
            (row, internal_uuid)
        };

        let row = if let Some(id) = req.service_virtual_user_id {
            ctx.db
                .update_agent(
                    ctx.org_id(),
                    row.id,
                    UpdateAgent {
                        virtual_user_id: Some(Some(id)),
                        ..Default::default()
                    },
                )
                .await?
                .ok_or_else(|| CommandError::not_found("Agent"))?
        } else {
            row
        };
        let row = persist_harness_source(ctx, row, harness_source).await?;
        persist_capabilities(&ctx.db, agent_uuid, &caps).await?;
        Ok(q::row_to_agent(row, caps))
    }
}
// ListAgents

/// List agents. Supports search, include_archived, pagination.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
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

#[command(
    name = "list_agents",
    category = "agents",
    description = "List all active agents. Use search for name search, include_archived=true to include archived. Supports pagination (limit/offset).",
    method = "GET",
    path = "/v1/agents",
    policy = AGENT_VIEW,
    cli = CliRoute::new(&["agents"], "list").with_examples(&[CliExample::new("Find agents by name when you do not know the id", "everruns agents list --search triage --limit 20",)]),
)]
impl Command for ListAgents {
    type Output = Paginated<Agent>;

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
            .await?;
        let agents = q::load_agents_list(&ctx.db, rows).await?;
        Ok(Paginated {
            data: agents,
            total,
            offset: pg.offset,
            limit: pg.limit,
        })
    }
}
// ============================================================================
// GetAgent
// ============================================================================

/// Get a single agent by ID or name.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetAgent {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "get_agent",
    category = "agents",
    description = "Get a single agent by ID or name.",
    method = "GET",
    path = "/v1/agents/{id}",
    policy = AGENT_VIEW,
    positional = "id",
    cli = CliRoute::new(&["agents"], "get").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("Show one agent's full configuration", "everruns agents get agt_01h9",)]),
)]
impl Command for GetAgent {
    type Output = Agent;

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        q::resolve(&ctx.db, ctx.org_id(), &self.id)
            .await?
            .ok_or_else(|| CommandError::not_found("Agent"))
    }
}
// ============================================================================
// UpdateAgent
// ============================================================================

/// Update an agent. Only provided fields are changed.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateAgentCmd {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateAgentRequest,
}

#[command(
    name = "update_agent",
    category = "agents",
    description = "Update an agent. Only provided fields are changed.",
    method = "PATCH",
    path = "/v1/agents/{id}",
    policy = AGENT_MANAGE,
    positional = "id",
    cli = CliRoute::new(&["agents"], "update").with_args(&[CliArg::new("id").at(1), CliArg::new("harness_name").short('H').long("harness"), CliArg::new("tag").short('t'),]).with_examples(&[CliExample::new("Rename an agent, leaving the rest of it alone", "everruns agents update agt_01h9 --name triage-v2 --reason 'Rename after the triage split'",)]),
)]
impl Command for UpdateAgentCmd {
    type Output = Agent;

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
            validate_managed_name(name)?;
        }
        validate_update_limits(&req)?;
        sandbox_templates::validate_update(&req.sandbox_policy)?;
        if let StorageUpdate::Set(environments) = &req.sandbox_policy {
            validate_sandbox_template_sources(ctx, Some(environments)).await?;
        }
        if matches!(req.status, Some(AgentStatus::Deleted)) {
            return Err(CommandError::forbidden(
                "Setting status=deleted requires dangerous delete permission".to_string(),
            ));
        }
        let is_archiving = matches!(req.status, Some(AgentStatus::Archived));
        if let Some(ref caps) = req.capabilities {
            check_high_risk_caps(ctx, caps).await?;
        }

        // Resolve existing
        let existing = ctx
            .db
            .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
            .await?
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
            Some(caps) => Some(
                normalize_capability_refs(
                    ctx,
                    q::ensure_file_system_capability(caps, final_has_initial_files),
                )
                .await?,
            ),
            None if final_has_initial_files => Some(q::ensure_file_system_capability(
                q::get_capabilities(&ctx.db, ctx.org_id(), internal_id.uuid()).await?,
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
            .await?;
        }
        if let Some(ref servers) = req.mcp_servers {
            crate::domains::mcp_servers::scoped_mcp::validate_scoped_mcp_servers_for_org(
                &ctx.db,
                ctx.org_id(),
                servers,
            )
            .await?;
        }
        let default_model_id =
            q::validate_model_id(&ctx.db, ctx.org_id(), req.default_model_id).await?;
        let harness_id =
            resolve_update_harness_id(ctx, req.harness_id, req.harness_name.as_deref()).await?;
        let final_harness_id = harness_id.unwrap_or(existing.harness_id);
        let final_has_environments = match &req.sandbox_policy {
            StorageUpdate::Set(_) => true,
            StorageUpdate::Clear => false,
            StorageUpdate::Unchanged => existing.environments.is_some(),
        };
        reject_sandbox_override_for_fixed_harness(ctx, final_harness_id, final_has_environments)
            .await?;

        if let StorageUpdate::Set(id) = req.service_virtual_user_id {
            validate_service_account(ctx, Some(id)).await?;
        }
        if matches!(req.service_virtual_user_id, StorageUpdate::Clear) {
            crate::domains::virtual_users::VIRTUAL_USER_MANAGE
                .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
                .map_err(|_| {
                    CommandError::forbidden(
                        "Service account binding requires virtual-user management permission",
                    )
                })?;
        }
        let input = UpdateAgent {
            virtual_user_id: match req.service_virtual_user_id {
                StorageUpdate::Unchanged => None,
                StorageUpdate::Clear => Some(None),
                StorageUpdate::Set(id) => Some(Some(id)),
            },
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            intro_markdown: req.intro_markdown,
            short_description: req.short_description,
            starters: req
                .starters
                .map(|starters| serde_json::to_value(&starters).unwrap_or(serde_json::json!([]))),
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
                .transpose()?,
            network_access: req
                .network_access
                .map(|na| Some(serde_json::to_value(na).unwrap_or_default())),
            parallel_tool_calls: req.parallel_tool_calls.map(Some),
            environments: sandbox_templates::update_to_json(req.sandbox_policy),
            ..Default::default()
        };
        // Validate the entire update before irreversible external cleanup.
        if is_archiving {
            super::lifecycle::prepare_for_removal(ctx, internal_id.uuid()).await?;
        }
        let row = ctx
            .db
            .update_agent(ctx.org_id(), internal_id, input)
            .await?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        if is_archiving {
            super::credentials::revoke_agent_grants(ctx, &row).await?;
        }

        let caps = if let Some(caps) = capabilities_override {
            persist_capabilities(&ctx.db, internal_id.uuid(), &caps).await?;
            caps
        } else {
            q::get_capabilities(&ctx.db, ctx.org_id(), internal_id.uuid()).await?
        };

        let agent = q::row_to_agent(row, caps);
        super::branding_slack::sync_if_changed(
            ctx,
            super::branding_slack::display_name(&existing.name, existing.display_name.as_deref()),
            existing.description.as_deref(),
            &agent,
        );
        Ok(agent)
    }
}
// ============================================================================
// DeleteAgent
// ============================================================================

/// Archive an agent (soft delete).
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteAgent {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "delete_agent",
    category = "agents",
    description = "Archive an agent (soft delete). Can be restored.",
    method = "DELETE",
    path = "/v1/agents/{id}",
    policy = AGENT_MANAGE,
    positional = "id",
    cli = CliRoute::new(&["agents"], "delete").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("Archive an agent, keeping it restorable", "everruns agents delete agt_01h9 --reason 'Replaced by triage-v2'",)]),
)]
impl Command for DeleteAgent {
    type Output = serde_json::Value;

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let agent_id: AgentId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid agent ID: {e}")))?;

        q::ensure_not_built_in(&ctx.db, ctx.org_id(), &self.id, "delete").await?;

        let row = ctx
            .db
            .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
            .await?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        super::lifecycle::prepare_for_removal(ctx, row.id.uuid()).await?;
        ctx.db.delete_agent(ctx.org_id(), row.id).await?;
        super::credentials::revoke_agent_grants(ctx, &row).await?;

        Ok(serde_json::json!({"deleted": true}))
    }
}
// ============================================================================
// UpsertAgent
// ============================================================================

/// Upsert agent — create or update by ID.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpsertAgent {
    /// Internal replacement mode used by complete portable manifests.
    #[serde(skip)]
    pub(crate) replace_capabilities: bool,
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

#[command(
    name = "upsert_agent",
    category = "agents",
    description = "Upsert agent — create (201) or update (200) by ID.",
    method = "PUT",
    path = "/v1/agents/{id}",
    policy = AGENT_MANAGE,
    positional = "id",
    cli = CliRoute::new(&["agents"], "upsert").with_args(&[CliArg::new("id").at(1), CliArg::new("harness_name").short('H').long("harness"), CliArg::new("tag").short('t'),]).with_examples(&[CliExample::new("Create or replace an agent at a known id, for a scripted deploy", "everruns agents upsert agt_01h9 --name triage --system-prompt 'Triage incoming issues' --reason 'Deploy release 42'",)]),
)]
impl Command for UpsertAgent {
    type Output = UpsertResult;

    async fn execute(self, ctx: &Ctx) -> Result<UpsertResult, CommandError> {
        let req = self.req;
        let public_id = self.id;

        // Upsert addresses an agent by public id, so it reaches an existing
        // built-in the same way update does. An id that resolves to nothing
        // falls through to the create path, which cannot mint a built-in.
        q::ensure_not_built_in(&ctx.db, ctx.org_id(), &public_id, "modify").await?;

        // Validate (same checks as CreateAgent)
        validate_name("Agent", &req.name)?;
        validate_managed_name(&req.name)?;
        validate_create_limits(&req)?;
        sandbox_templates::validate(req.sandbox_policy.as_ref())?;
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
        .await?;
        crate::domains::mcp_servers::scoped_mcp::validate_scoped_mcp_servers_for_org(
            &ctx.db,
            ctx.org_id(),
            &req.mcp_servers,
        )
        .await?;
        let default_model_id =
            q::validate_model_id(&ctx.db, ctx.org_id(), req.default_model_id).await?;
        let harness_source = if req.harness_id.is_some() || req.harness_name.is_some() {
            "explicit"
        } else {
            "organization_default"
        };
        let harness_id =
            resolve_create_harness_id(ctx, req.harness_id, req.harness_name.as_deref()).await?;
        let existing = ctx
            .db
            .get_agent_by_public_id(ctx.org_id(), &public_id)
            .await?;
        let input = CreateAgentRow {
            public_id: public_id.clone(),
            name: req.name,
            display_name: req.display_name,
            description: req.description,
            intro_markdown: req.intro_markdown,
            short_description: req.short_description,
            starters: serde_json::to_value(&req.starters).unwrap_or(serde_json::json!([])),
            system_prompt: req.system_prompt,
            default_model_id,
            harness_id,
            tags: req.tags,
            initial_files: serde_json::to_value(&req.initial_files).unwrap_or_default(),
            tools: serde_json::to_value(&req.tools).unwrap_or_default(),
            mcp_servers: serde_json::to_value(&req.mcp_servers).unwrap_or_default(),
            max_iterations: max_iterations::to_db(req.max_iterations)?,
            parallel_tool_calls: req.parallel_tool_calls,
            environments: sandbox_templates::to_json(req.sandbox_policy.as_ref()),
            network_access: req
                .network_access
                .as_ref()
                .map(|na| serde_json::to_value(na).unwrap_or_default()),
            // See CreateAgent: only org bootstrap mints built-in agents.
            is_built_in: false,
        };
        let (row, was_created) = ctx.db.upsert_agent(ctx.org_id(), input).await?;
        let row = persist_harness_source(ctx, row, harness_source).await?;
        let agent_uuid = row.id.uuid();

        let final_caps = if self.replace_capabilities || !caps.is_empty() {
            persist_capabilities(&ctx.db, agent_uuid, &caps).await?;
            caps
        } else if was_created {
            vec![]
        } else {
            q::get_capabilities(&ctx.db, ctx.org_id(), agent_uuid).await?
        };

        let agent = q::row_to_agent(row, final_caps);
        if let Some(existing) = existing {
            super::branding_slack::sync_if_changed(
                ctx,
                super::branding_slack::display_name(
                    &existing.name,
                    existing.display_name.as_deref(),
                ),
                existing.description.as_deref(),
                &agent,
            );
        }
        Ok(UpsertResult { agent, was_created })
    }
}
// ============================================================================
// CopyAgent
// ============================================================================

/// Copy an agent. Generates a unique name ({name}-copy, -copy-2, etc.)
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CopyAgent {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "copy_agent",
    category = "agents",
    description = "Copy an agent. Generates a unique name.",
    method = "POST",
    path = "/v1/agents/{id}/copy",
    policy = AGENT_MANAGE,
    positional = "id",
    cli = CliRoute::new(&["agents"], "copy").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("Duplicate an agent to try a change without touching the original", "everruns agents copy agt_01h9 --reason 'Trial a stricter prompt'",)]),
)]
impl Command for CopyAgent {
    type Output = Agent;

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        let source = q::resolve(&ctx.db, ctx.org_id(), &self.id)
            .await?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        let copy_name =
            q::find_unique_name(&ctx.db, ctx.org_id(), &format!("{}-copy", source.name)).await?;

        let req = CreateAgentRequest {
            service_virtual_user_id: None,

            id: None,
            name: copy_name,
            display_name: source.display_name.map(|d| format!("{d} (copy)")),
            description: source.description,
            intro_markdown: source.intro_markdown,
            short_description: source.short_description,
            starters: source.starters,
            system_prompt: source.system_prompt,
            default_model_id: source.default_model_id,
            harness_id: Some(source.harness_id),
            harness_name: None,
            tags: source.tags,
            capabilities: source.capabilities,
            sandbox_policy: source.sandbox_policy,
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
// ============================================================================
// SuspendAgentExposures / ResumeAgentExposures
// ============================================================================

/// Take every endpoint on an agent off the internet in one action (EVE-1007).
///
/// This is the incident control, and is deliberately separate from archiving and
/// from per-endpoint publish: it leaves every endpoint's own `status` untouched,
/// so resuming restores exactly the set that was live before — which is what
/// makes it safe to reach for under pressure.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct SuspendAgentExposures {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
}

#[command(
    name = "suspend_agent_exposures",
    category = "agents",
    description = "Stop every channel on an agent from accepting traffic.",
    method = "POST",
    path = "/v1/agents/{agent_id}/exposures/suspend",
    policy = AGENT_MANAGE,
    positional = "agent_id",
    cli = CliRoute::new(&["agents", "exposures"], "suspend").with_args(&[CliArg::new("agent_id").at(1).long("agent")]).with_examples(&[CliExample::new("Stop an agent answering on its exposed surfaces without deleting it", "everruns agents exposures suspend agt_01h9 --reason 'Pause while the prompt is fixed'",)]),
)]
impl Command for SuspendAgentExposures {
    type Output = Agent;

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        set_exposures_suspended(ctx, &self.agent_id, true).await
    }
}
/// Clear the agent-level exposure suspend, restoring the previously live set.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ResumeAgentExposures {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
}

#[command(
    name = "resume_agent_exposures",
    category = "agents",
    description = "Let an agent's live channels accept traffic again.",
    method = "POST",
    path = "/v1/agents/{agent_id}/exposures/resume",
    policy = AGENT_MANAGE,
    positional = "agent_id",
    cli = CliRoute::new(&["agents", "exposures"], "resume").with_args(&[CliArg::new("agent_id").at(1).long("agent")]).with_examples(&[CliExample::new("Put a suspended agent back on its exposed surfaces", "everruns agents exposures resume agt_01h9 --reason 'Prompt fix verified'",)]),
)]
impl Command for ResumeAgentExposures {
    type Output = Agent;

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        set_exposures_suspended(ctx, &self.agent_id, false).await
    }
}

/// Resolve an agent for a command that mutates it in place.
///
/// Built-in agents are protected: a platform upgrade ships their definition,
/// and an org that edited its own copy would silently diverge.
async fn resolve_agent_for_mutation(ctx: &Ctx, id: &str) -> Result<Agent, CommandError> {
    let agent = q::resolve(&ctx.db, ctx.org_id(), id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    q::ensure_not_built_in(&ctx.db, ctx.org_id(), id, "modify").await?;
    Ok(agent)
}

async fn set_exposures_suspended(
    ctx: &Ctx,
    agent_id: &str,
    suspended: bool,
) -> Result<Agent, CommandError> {
    let agent = resolve_agent_for_mutation(ctx, agent_id).await?;
    let row = ctx
        .db
        .update_agent(
            ctx.org_id(),
            AgentId::from_uuid(agent.internal_id),
            UpdateAgent {
                exposures_suspended: Some(suspended),
                ..Default::default()
            },
        )
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    let caps = q::get_capabilities(&ctx.db, row.org_id, row.id.uuid()).await?;
    let agent = q::row_to_agent(row, caps);
    q::with_derived_exposure_one(&ctx.db, Some(agent))
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))
}

// ============================================================================
// AnalyzeAgent
// ============================================================================

/// Run advisory checks (built-in rules + LLM analysis) against an agent shape.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct AnalyzeAgent {
    #[serde(default)]
    pub harness_id: Option<HarnessId>,
    #[serde(default)]
    pub initial_files: Vec<InitialFile>,
    pub system_prompt: Option<String>,
    #[serde(default)]
    #[schema(value_type = Vec<crate::records::CapabilityRefSchema>)]
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

#[command(
    name = "analyze_agent",
    category = "agents",
    description = "Run advisory checks (built-in rules plus LLM analysis) against an \
                          agent configuration.",
    method = "POST",
    path = "/v1/agents/analyze",
    policy = crate::domains::agents::AGENT_MANAGE,
    // Makes paid utility-LLM calls; not a free read.
    read_only = false,
    cli = CliRoute::new(&["agents"], "analyze").with_examples(&[CliExample::new("Check a draft configuration for problems before creating the agent", "everruns agents analyze --system-prompt 'Triage incoming issues' --tools '[\"bash\"]'",)]),
)]
impl Command for AnalyzeAgent {
    type Output = AgentAnalysis;

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
            harness_id: self.harness_id,
            initial_files: self.initial_files,
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
// ============================================================================
// CheckAgentName
// ============================================================================

/// Check whether an agent name is available.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CheckAgentName {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    pub exclude_id: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct NameAvailability {
    pub available: bool,
}

#[command(
    name = "check_agent_name",
    category = "agents",
    description = "Check whether an agent name is available.",
    method = "GET",
    path = "/v1/agents/check-name",
    policy = AGENT_VIEW,
    cli = CliRoute::new(&["agents"], "check-name").with_examples(&[CliExample::new("See whether a name is free before creating an agent", "everruns agents check-name --name triage",)]),
)]
impl Command for CheckAgentName {
    type Output = NameAvailability;

    async fn execute(self, ctx: &Ctx) -> Result<NameAvailability, CommandError> {
        if crate::records::validate_addressable_name(&self.name).is_err() {
            return Ok(NameAvailability { available: false });
        }

        let exclude_id = self
            .exclude_id
            .map(|id| {
                id.parse::<AgentId>()
                    .map_err(|e| CommandError::bad_request(format!("Invalid exclude_id: {e}")))
            })
            .transpose()?;

        let existing = ctx.db.get_agent_by_name(ctx.org_id(), &self.name).await?;

        let available = match existing {
            Some(row) => exclude_id == Some(row.id),
            None => true,
        };

        Ok(NameAvailability { available })
    }
}
#[cfg(test)]
mod tests;

// ============================================================================
// DestroyAgent (hard delete)
// ============================================================================

/// Permanently delete an archived agent.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DestroyAgent {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "destroy_agent",
    category = "agents",
    description = "Permanently delete an archived agent.",
    method = "POST",
    path = "/v1/agents/{id}/delete",
    policy = AGENT_DANGEROUS,
    positional = "id",
    http = no_content,
    responses((status = 404, description = "Agent not found")),
    cli = CliRoute::new(&["agents"], "destroy").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("Permanently remove an already-archived agent", "everruns agents destroy agt_01h9 --reason 'Retired after the archive window'",)]),
)]
impl Command for DestroyAgent {
    type Output = serde_json::Value;

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let agent_id: AgentId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid agent ID: {e}")))?;

        q::ensure_not_built_in(&ctx.db, ctx.org_id(), &self.id, "delete").await?;

        let row = ctx
            .db
            .get_agent_by_public_id(ctx.org_id(), &agent_id.to_string())
            .await?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        if row.status != "archived" {
            return Err(CommandError::bad_request(
                "Agent must be archived before deletion",
            ));
        }

        super::lifecycle::prepare_for_removal(ctx, row.id.uuid()).await?;
        ctx.db.destroy_agent(ctx.org_id(), row.id).await?;

        Ok(serde_json::json!({"destroyed": true}))
    }
}
mod service_account;
use service_account::{validate_create_limits, validate_service_account, validate_update_limits};

// Agent-scripts commands: create, list, get, update, delete.
//
// A saved script is a shell script an agent owns. Delete archives it and frees
// its name. Mutations are recorded in change history through the registry
// (`domains::change_history::registry`).

use super::queries as q;
use super::types::{CreateAgentScriptRequest, UpdateAgentScriptRequest};
use super::validation::{
    MAX_ACTIVE_SCRIPTS_PER_AGENT, validate_body, validate_description, validate_input_schema,
    validate_name,
};
use crate::domains::agents::{AGENT_MANAGE, AGENT_VIEW};
use crate::domains::common::*;
use crate::records::AgentScript;
use crate::storage::{CreateAgentScriptRow, UpdateAgentScript};
use everruns_contracts::typed_id::ScriptId;
use serde::Deserialize;
use serde_json::json;
use utoipa::ToSchema;

// ============================================================================
// CreateAgentScript
// ============================================================================

/// Create a saved script on an agent.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateAgentScript {
    /// Owning agent's prefixed public identifier.
    pub agent_id: String,
    #[serde(flatten)]
    pub req: CreateAgentScriptRequest,
}

#[command(
    name = "create_agent_script",
    category = "agent_scripts",
    description = "Create a saved shell script an agent owns.",
    method = "POST",
    path = "/v1/agents/{agent_id}/scripts",
    policy = AGENT_MANAGE,
)]
impl Command for CreateAgentScript {
    type Output = AgentScript;

    async fn execute(self, ctx: &Ctx) -> Result<AgentScript, CommandError> {
        let agent_public = q::parse_agent_id(&self.agent_id)?;
        let agent = q::require_active_agent(&ctx.db, ctx.org_id(), &agent_public).await?;
        let req = self.req;

        validate_name(&req.name)?;
        validate_description(&req.description)?;
        validate_body(&req.body)?;
        if let Some(schema) = &req.input_schema {
            validate_input_schema(schema)?;
        }

        let active = ctx
            .db
            .list_agent_scripts(ctx.org_id(), agent.id, false)
            .await?;
        if active.iter().any(|script| script.name == req.name) {
            return Err(CommandError::conflict(format!(
                "Agent already has a script named '{}'",
                req.name
            )));
        }
        if active.len() as i64 >= MAX_ACTIVE_SCRIPTS_PER_AGENT {
            return Err(CommandError::bad_request(format!(
                "An agent may have at most {MAX_ACTIVE_SCRIPTS_PER_AGENT} scripts"
            )));
        }

        let row = ctx
            .db
            .create_agent_script(CreateAgentScriptRow {
                org_id: ctx.org_id(),
                id: ScriptId::new(),
                agent_id: agent.id,
                name: req.name,
                description: req.description,
                input_schema: req.input_schema,
                body: req.body,
            })
            .await?;
        Ok(q::row_to_script(row, agent_public))
    }
}

// ============================================================================
// ListAgentScripts
// ============================================================================

/// List an agent's saved scripts, including their bodies.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListAgentScripts {
    pub agent_id: String,
    #[serde(default, deserialize_with = "deserialize_bool_lenient")]
    pub include_archived: bool,
}

#[command(
    name = "list_agent_scripts",
    category = "agent_scripts",
    description = "List an agent's saved scripts with their bodies. include_archived=true also returns archived.",
    method = "GET",
    path = "/v1/agents/{agent_id}/scripts",
    policy = AGENT_VIEW,
    positional = "agent_id",
)]
impl Command for ListAgentScripts {
    type Output = Vec<AgentScript>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<AgentScript>, CommandError> {
        let agent_public = q::parse_agent_id(&self.agent_id)?;
        let agent = q::require_active_agent(&ctx.db, ctx.org_id(), &agent_public).await?;
        let rows = ctx
            .db
            .list_agent_scripts(ctx.org_id(), agent.id, self.include_archived)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| q::row_to_script(row, agent_public))
            .collect())
    }
}

// ============================================================================
// GetAgentScript
// ============================================================================

/// Get a single saved script by id.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetAgentScript {
    pub agent_id: String,
    pub script_id: String,
}

#[command(
    name = "get_agent_script",
    category = "agent_scripts",
    description = "Get a single saved script by id.",
    method = "GET",
    path = "/v1/agents/{agent_id}/scripts/{script_id}",
    policy = AGENT_VIEW,
)]
impl Command for GetAgentScript {
    type Output = AgentScript;

    async fn execute(self, ctx: &Ctx) -> Result<AgentScript, CommandError> {
        let (_, script) =
            q::resolve_script_for_agent(&ctx.db, ctx.org_id(), &self.agent_id, &self.script_id)
                .await?;
        Ok(q::row_to_script(script, q::parse_agent_id(&self.agent_id)?))
    }
}

// ============================================================================
// UpdateAgentScriptCmd
// ============================================================================

/// Update a saved script. Only provided fields change; the name is immutable.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateAgentScriptCmd {
    pub agent_id: String,
    pub script_id: String,
    #[serde(flatten)]
    pub req: UpdateAgentScriptRequest,
}

#[command(
    name = "update_agent_script",
    category = "agent_scripts",
    description = "Update a saved script's description, input schema or body. The name is immutable.",
    method = "PATCH",
    path = "/v1/agents/{agent_id}/scripts/{script_id}",
    policy = AGENT_MANAGE,
)]
impl Command for UpdateAgentScriptCmd {
    type Output = AgentScript;

    async fn execute(self, ctx: &Ctx) -> Result<AgentScript, CommandError> {
        let (_, existing) =
            q::resolve_script_for_agent(&ctx.db, ctx.org_id(), &self.agent_id, &self.script_id)
                .await?;
        let req = self.req;
        if let Some(description) = &req.description {
            validate_description(description)?;
        }
        if let Some(body) = &req.body {
            validate_body(body)?;
        }
        if let Some(schema) = &req.input_schema {
            validate_input_schema(schema)?;
        }
        let row = ctx
            .db
            .update_agent_script(
                ctx.org_id(),
                existing.id,
                UpdateAgentScript {
                    description: req.description,
                    input_schema: req.input_schema,
                    body: req.body,
                },
            )
            .await?
            // Archived scripts are read-only.
            .ok_or_else(|| CommandError::not_found("Agent script"))?;
        Ok(q::row_to_script(row, q::parse_agent_id(&self.agent_id)?))
    }
}

// ============================================================================
// DeleteAgentScript
// ============================================================================

/// Archive a saved script (soft delete); its name becomes free again.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteAgentScript {
    pub agent_id: String,
    pub script_id: String,
}

#[command(
    name = "delete_agent_script",
    category = "agent_scripts",
    description = "Archive a saved script and free its name.",
    method = "DELETE",
    path = "/v1/agents/{agent_id}/scripts/{script_id}",
    policy = AGENT_MANAGE,
)]
impl Command for DeleteAgentScript {
    type Output = serde_json::Value;

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let (_, script) =
            q::resolve_script_for_agent(&ctx.db, ctx.org_id(), &self.agent_id, &self.script_id)
                .await?;
        if ctx.db.delete_agent_script(ctx.org_id(), script.id).await? {
            Ok(json!({ "deleted": true }))
        } else {
            Err(CommandError::not_found("Agent script"))
        }
    }
}

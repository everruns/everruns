// Agent identity commands — user-facing operations.
//
// Each struct is the request type, catalog entry, and execution logic.
// inventory::submit! auto-registers for MCP catalog.

use super::queries as q;
use super::types::{
    CreateAgentIdentityRequest, CreateAgentIdentityRow, UpdateAgentIdentity,
    UpdateAgentIdentityRequest,
};
use crate::domains::common::*;
use crate::services::agent_identity::{
    AGENT_IDENTITY_DANGEROUS, AGENT_IDENTITY_MANAGE, AGENT_IDENTITY_VIEW,
};
use everruns_core::{AgentIdentity, AgentIdentityId, Policy};
use serde::Deserialize;

// ============================================================================
// CreateAgentIdentity
// ============================================================================

/// Create a new agent identity (virtual principal for unattended execution).
#[derive(Debug, Deserialize)]
pub struct CreateAgentIdentity(pub CreateAgentIdentityRequest);

impl Command for CreateAgentIdentity {
    type Output = AgentIdentity;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_agent_identity",
            category: "agent_identities",
            description: "Create a new agent identity (virtual principal for unattended execution).",
            method: "POST",
            path: "/v1/agent-identities",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_IDENTITY_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AgentIdentity, CommandError> {
        let req = self.0;

        // Validate locale/timezone
        if let Some(ref locale) = req.locale {
            q::validate_locale(locale).map_err(classify_anyhow)?;
        }
        if let Some(ref tz) = req.timezone {
            q::validate_timezone(tz).map_err(classify_anyhow)?;
        }

        // Persist
        let row = ctx
            .db
            .create_agent_identity(CreateAgentIdentityRow {
                org_id: ctx.org_id(),
                id: AgentIdentityId::new(),
                name: req.name,
                description: req.description,
                avatar_url: req.avatar_url,
                locale: req.locale,
                timezone: req.timezone,
            })
            .await
            .map_err(classify_anyhow)?;

        Ok(q::row_to_identity(row))
    }
}

inventory::submit! { CommandDescriptor::of::<CreateAgentIdentity>() }

// ============================================================================
// ListAgentIdentities
// ============================================================================

/// List agent identities. Supports search and include_archived.
#[derive(Debug, Deserialize)]
pub struct ListAgentIdentities {
    pub search: Option<String>,
    #[serde(default)]
    pub include_archived: bool,
}

impl Command for ListAgentIdentities {
    type Output = Vec<AgentIdentity>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_agent_identities",
            category: "agent_identities",
            description: "List all active agent identities. Use search for name search, include_archived=true to include archived.",
            method: "GET",
            path: "/v1/agent-identities",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_IDENTITY_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<AgentIdentity>, CommandError> {
        let rows = ctx
            .db
            .list_agent_identities(ctx.org_id(), self.search.as_deref(), self.include_archived)
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.into_iter().map(q::row_to_identity).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListAgentIdentities>() }

// ============================================================================
// GetAgentIdentity
// ============================================================================

/// Get a single agent identity by ID.
#[derive(Debug, Deserialize)]
pub struct GetAgentIdentity {
    pub id: String,
}

impl Command for GetAgentIdentity {
    type Output = AgentIdentity;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_agent_identity",
            category: "agent_identities",
            description: "Get a single agent identity by ID.",
            method: "GET",
            path: "/v1/agent-identities/{identity_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_IDENTITY_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AgentIdentity, CommandError> {
        let identity_id: AgentIdentityId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid identity ID: {e}")))?;

        q::get_by_id(&ctx.db, ctx.org_id(), identity_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent identity"))
    }
}

inventory::submit! { CommandDescriptor::of::<GetAgentIdentity>() }

// ============================================================================
// UpdateAgentIdentity
// ============================================================================

/// Update an agent identity. Only provided fields are changed.
#[derive(Debug, Deserialize)]
pub struct UpdateAgentIdentityCmd {
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateAgentIdentityRequest,
}

impl Command for UpdateAgentIdentityCmd {
    type Output = AgentIdentity;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_agent_identity",
            category: "agent_identities",
            description: "Update an agent identity. Only provided fields are changed.",
            method: "PATCH",
            path: "/v1/agent-identities/{identity_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_IDENTITY_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<AgentIdentity, CommandError> {
        let identity_id: AgentIdentityId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid identity ID: {e}")))?;

        let req = self.req;

        // Validate locale/timezone if being set
        if let everruns_durable::UpdateField::Set(ref locale) = req.locale {
            q::validate_locale(locale).map_err(classify_anyhow)?;
        }
        if let everruns_durable::UpdateField::Set(ref tz) = req.timezone {
            q::validate_timezone(tz).map_err(classify_anyhow)?;
        }

        // Persist
        let row = ctx
            .db
            .update_agent_identity(
                ctx.org_id(),
                identity_id,
                UpdateAgentIdentity {
                    name: req.name,
                    description: req.description,
                    avatar_url: req.avatar_url,
                    locale: req.locale,
                    timezone: req.timezone,
                    status: req.status.map(|s| s.to_string()),
                },
            )
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent identity"))?;

        Ok(q::row_to_identity(row))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateAgentIdentityCmd>() }

// ============================================================================
// DeleteAgentIdentity
// ============================================================================

/// Archive an agent identity (soft delete).
#[derive(Debug, Deserialize)]
pub struct DeleteAgentIdentity {
    pub id: String,
}

impl Command for DeleteAgentIdentity {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_agent_identity",
            category: "agent_identities",
            description: "Archive an agent identity (soft delete). Can be restored.",
            method: "DELETE",
            path: "/v1/agent-identities/{identity_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_IDENTITY_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let identity_id: AgentIdentityId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid identity ID: {e}")))?;

        let deleted = ctx
            .db
            .delete_agent_identity(ctx.org_id(), identity_id)
            .await
            .map_err(classify_anyhow)?;

        if deleted {
            Ok(serde_json::json!({"deleted": true}))
        } else {
            Err(CommandError::not_found("Agent identity"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteAgentIdentity>() }

// ============================================================================
// DestroyAgentIdentity (hard delete)
// ============================================================================

/// Permanently delete an agent identity.
#[derive(Debug, Deserialize)]
pub struct DestroyAgentIdentity {
    pub id: String,
}

impl Command for DestroyAgentIdentity {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "destroy_agent_identity",
            category: "agent_identities",
            description: "Permanently delete an agent identity.",
            method: "POST",
            path: "/v1/agent-identities/{identity_id}/delete",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&AGENT_IDENTITY_DANGEROUS)
    }

    async fn execute(self, ctx: &Ctx) -> Result<serde_json::Value, CommandError> {
        let identity_id: AgentIdentityId = self
            .id
            .parse()
            .map_err(|e| CommandError::bad_request(format!("Invalid identity ID: {e}")))?;

        let destroyed = ctx
            .db
            .destroy_agent_identity(ctx.org_id(), identity_id)
            .await
            .map_err(classify_anyhow)?;

        if destroyed {
            Ok(serde_json::json!({"destroyed": true}))
        } else {
            Err(CommandError::not_found("Agent identity"))
        }
    }
}

inventory::submit! { CommandDescriptor::of::<DestroyAgentIdentity>() }

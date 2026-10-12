//! Agent-scoped bound secrets: one write-only key for every session and
//! trigger of an agent.
//!
//! Decision: agent secrets are always bound (see
//! `knowledge/security/bound-secrets.md`). An unbound agent secret would have
//! to be readable by the model to be useful, which is what session secrets
//! already are; an agent-wide readable key would leak into every session's
//! transcript. A session secret of the same name shadows the agent's.
//!
//! THREAT[TM-BASH-030]: no command here returns a value.

use super::{MAX_SECRET_VALUE_BYTES, validate_binding};
use crate::domains::agents::{AGENT_MANAGE, AGENT_VIEW};
use crate::domains::common::*;
use crate::domains::session_storage::types::{AgentSecretInfo, SecretBinding};
use crate::kernel_imports::contracts::typed_id::AgentId;
use crate::storage::AgentSecretInfoRow;
use serde::Deserialize;
use utoipa::ToSchema;

/// The agent's internal id, from its public one. The two differ: sessions
/// and `agent_secrets` key on the internal id.
async fn owned_agent(ctx: &Ctx, agent_id: &str) -> Result<AgentId, CommandError> {
    let public_id: AgentId = agent_id
        .parse()
        .map_err(|_| CommandError::bad_request("Invalid agent ID"))?;
    let agent = ctx
        .db
        .get_agent_by_public_id(ctx.org_id(), &public_id.to_string())
        .await?
        .filter(|row| row.status != "deleted")
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    Ok(agent.id)
}

fn info(row: AgentSecretInfoRow) -> AgentSecretInfo {
    AgentSecretInfo {
        name: row.name,
        binding: row.binding.0,
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
/// List an agent's bound secrets, without values.
pub struct ListAgentSecrets {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
}

#[command(
    name = "list_agent_secrets",
    category = "agents",
    description = "List an agent's bound secrets: names, bound URLs and auth forms, never values.",
    method = "GET",
    path = "/v1/agents/{agent_id}/secrets",
    policy = AGENT_VIEW,
    cli = CliRoute::new(&["agents", "secrets"], "list").with_examples(&[CliExample::new("See which API keys an agent's shell can use", "everruns agents secrets list --agent-id agent_01h9",)]),
)]
impl Command for ListAgentSecrets {
    type Output = Vec<AgentSecretInfo>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<AgentSecretInfo>, CommandError> {
        let agent_id = owned_agent(ctx, &self.agent_id).await?;
        Ok(ctx
            .db
            .list_agent_secrets(ctx.org_id(), agent_id.uuid())
            .await?
            .into_iter()
            .map(info)
            .collect())
    }
}

#[derive(Deserialize, ToSchema, serde::Serialize)]
/// Store a bound secret for every session of an agent.
pub struct SetAgentSecret {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
    /// Env var name the agent's shell sees, e.g. `JEV_API_KEY`.
    pub name: String,
    /// Write-only value; encrypted and never returned.
    pub value: String,
    /// Where the value may be sent, and how.
    pub binding: SecretBinding,
}

impl std::fmt::Debug for SetAgentSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetAgentSecret")
            .field("agent_id", &self.agent_id)
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .field("binding", &self.binding)
            .finish()
    }
}

#[command(
    name = "set_agent_secret",
    category = "agents",
    description = "Encrypt and store a bound secret for every session of an agent. The shell sees $NAME as a placeholder; the value goes only on requests to the bound URL. Setting it again, binding included, replaces it.",
    method = "PUT",
    path = "/v1/agents/{agent_id}/secrets/{name}",
    policy = AGENT_MANAGE,
    cli = CliRoute::new(&["agents", "secrets"], "set").with_examples(&[CliExample::new("Give an agent's shell an API key it can use but never read", "everruns agents secrets set --agent-id agent_01h9 --name JEV_API_KEY --value \"$JEV_API_KEY\" --binding '{\"url\":\"https://api.typesafe.ai\",\"auth\":{\"type\":\"bearer\"}}' --reason 'Jev decisions for ticket triage'",)]),
)]
impl Command for SetAgentSecret {
    type Output = AgentSecretInfo;

    async fn execute(self, ctx: &Ctx) -> Result<AgentSecretInfo, CommandError> {
        let binding = validate_binding(&self.name, self.binding)?;
        if self.value.is_empty() || self.value.len() > MAX_SECRET_VALUE_BYTES {
            return Err(CommandError::bad_request(
                "Secret values must be between 1 byte and 64 KiB",
            ));
        }
        let agent_id = owned_agent(ctx, &self.agent_id).await?;
        let encryption = ctx.encryption.as_ref().ok_or_else(|| {
            CommandError::bad_request(
                "Encryption not configured. Set SECRETS_ENCRYPTION_KEY environment variable.",
            )
        })?;
        let encrypted = encryption
            .encrypt_string(&self.value)
            .map_err(CommandError::internal)?;
        let row = ctx
            .db
            .upsert_agent_secret(
                ctx.org_id(),
                agent_id.uuid(),
                &self.name,
                &binding,
                &encrypted,
            )
            .await?;
        Ok(info(row))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
/// Delete an agent's bound secret.
pub struct DeleteAgentSecret {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
    /// Secret name to delete.
    pub name: String,
}

#[command(
    name = "delete_agent_secret",
    category = "agents",
    description = "Delete an agent's bound secret by name.",
    method = "DELETE",
    path = "/v1/agents/{agent_id}/secrets/{name}",
    policy = AGENT_MANAGE,
    cli = CliRoute::new(&["agents", "secrets"], "delete").with_examples(&[CliExample::new("Revoke an agent's API key", "everruns agents secrets delete --agent-id agent_01h9 --name JEV_API_KEY --reason 'Key rotated'",)]),
)]
impl Command for DeleteAgentSecret {
    type Output = bool;

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        let agent_id = owned_agent(ctx, &self.agent_id).await?;
        if !ctx
            .db
            .delete_agent_secret(ctx.org_id(), agent_id.uuid(), &self.name)
            .await?
        {
            return Err(CommandError::not_found("Secret"));
        }
        Ok(true)
    }
}

/// The public id of a session's agent, for agent-scoped sets that name the
/// session (MCP `session_set_secret` with `scope: agent`).
pub async fn agent_of_session(ctx: &Ctx, session_id: &str) -> Result<String, CommandError> {
    let session_id = crate::domains::session_storage::queries::parse_owned_session_id(session_id)?;
    let session = ctx
        .db
        .get_session(ctx.org_id(), session_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Session"))?;
    let agent_id = session.agent_id.ok_or_else(|| {
        CommandError::bad_request("This session has no agent to store the secret on")
    })?;
    ctx.db
        .get_agent_public_id(ctx.org_id(), agent_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))
}

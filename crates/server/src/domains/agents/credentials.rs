use crate::domains::common::{Command, CommandError, CommandMeta, Ctx, classify_anyhow};
use crate::kernel_imports::{CapabilityId, everruns_provider::typed_id::AgentId};
use crate::storage::models::{AgentMcpSecretBindingRow, UpsertAgentMcpSecretBindingRow};
use everruns_mcp::McpCapabilityIdExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AGENT_MANAGE;

const MAX_NAME_LEN: usize = 255;

#[derive(Debug, Clone, Serialize, ToSchema)]
/// Metadata for a write-only credential bound to one agent and MCP tool parameter.
pub struct AgentCredentialBinding {
    /// Stable credential binding identifier.
    #[schema(example = "01933b5a-0000-7000-8000-000000000001")]
    pub id: Uuid,
    /// Agent that exclusively owns and may use this credential.
    #[schema(example = "agent_01933b5a000070008000000000000001")]
    pub agent_id: String,
    /// Name of the attached MCP server.
    #[schema(example = "visti")]
    pub mcp_server_name: String,
    /// Exact MCP endpoint authorized to receive the credential.
    #[schema(example = "https://visti.sh/mcp")]
    pub mcp_server_url: String,
    /// MCP tool whose outbound call receives the credential.
    #[schema(example = "visti_send")]
    pub tool_name: String,
    /// Top-level tool argument injected by the server.
    #[schema(example = "channel_key")]
    pub parameter_name: String,
    /// Human-readable credential label.
    #[schema(example = "Visti channel key")]
    pub label: String,
    /// Optional explanation of why the credential is needed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Delivers scheduled status notifications")]
    pub description: Option<String>,
    /// Whether an encrypted value has been provisioned.
    #[schema(example = true)]
    pub configured: bool,
    /// RFC 3339 creation time.
    #[schema(example = "2026-08-08T16:00:00Z")]
    pub created_at: String,
    /// RFC 3339 last-update time.
    #[schema(example = "2026-08-08T16:05:00Z")]
    pub updated_at: String,
    /// Relative UI route where the user can securely provision the value.
    #[schema(example = "/agents/agent_01933b5a000070008000000000000001?tab=credentials")]
    pub setup_url: String,
}

impl AgentCredentialBinding {
    pub fn from_row(row: AgentMcpSecretBindingRow, public_agent_id: &str) -> Self {
        Self {
            id: row.id,
            agent_id: public_agent_id.to_string(),
            mcp_server_name: row.mcp_server_name,
            mcp_server_url: row.mcp_server_url,
            tool_name: row.tool_name,
            parameter_name: row.parameter_name,
            label: row.label,
            description: row.description,
            configured: row.value_encrypted.is_some(),
            created_at: row.created_at.to_rfc3339(),
            updated_at: row.updated_at.to_rfc3339(),
            setup_url: format!("/agents/{public_agent_id}?tab=credentials"),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
/// Declare a credential requirement for an agent's attached MCP tool.
pub struct CreateAgentCredentialBinding {
    /// Agent ID, populated from the request path by the HTTP API.
    #[serde(default)]
    #[schema(example = "agent_01933b5a000070008000000000000001")]
    pub agent_id: String,
    /// Name of an MCP server attached to the agent.
    #[schema(example = "visti")]
    pub mcp_server_name: String,
    /// MCP tool whose outbound call requires the credential.
    #[schema(example = "visti_send")]
    pub tool_name: String,
    /// Top-level tool argument to inject outside model context.
    #[schema(example = "channel_key")]
    pub parameter_name: String,
    /// Human-readable label shown in the secure setup UI.
    #[schema(example = "Visti channel key")]
    pub label: String,
    /// Optional explanation of how the credential will be used.
    #[serde(default)]
    #[schema(example = "Delivers scheduled status notifications")]
    pub description: Option<String>,
}

impl Command for CreateAgentCredentialBinding {
    type Output = AgentCredentialBinding;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_agent_credential_binding",
            category: "agents",
            description: "Create a write-only Agent credential setup requirement for one attached MCP tool parameter. This command never accepts a secret value; the user completes setup in the Agent Credentials UI.",
            method: "POST",
            path: "/v1/agents/{agent_id}/credentials",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        validate_component("MCP server name", &self.mcp_server_name)?;
        validate_component("tool name", &self.tool_name)?;
        validate_component("parameter name", &self.parameter_name)?;
        validate_component("label", &self.label)?;
        if self.parameter_name.contains('.') {
            return Err(CommandError::bad_request(
                "Only top-level MCP tool parameters can be bound",
            ));
        }
        if self.description.as_ref().is_some_and(|v| v.len() > 2_000) {
            return Err(CommandError::bad_request(
                "Credential description must be 2000 characters or less",
            ));
        }

        let public_agent_id = self.agent_id.clone();
        let agent_id: AgentId = self
            .agent_id
            .parse()
            .map_err(|_| CommandError::bad_request("Invalid agent ID"))?;
        let agent = ctx
            .db
            .get_agent(ctx.org_id(), agent_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        let mcp_server_url =
            resolve_attached_server_url(ctx, &agent, &self.mcp_server_name).await?;

        let row = ctx
            .db
            .upsert_agent_mcp_secret_binding(UpsertAgentMcpSecretBindingRow {
                org_id: ctx.org_id(),
                agent_id: agent.id,
                mcp_server_name: self.mcp_server_name,
                mcp_server_url,
                tool_name: self.tool_name,
                parameter_name: self.parameter_name,
                label: self.label,
                description: self.description,
            })
            .await
            .map_err(classify_anyhow)?;
        Ok(AgentCredentialBinding::from_row(row, &public_agent_id))
    }
}

inventory::submit! { crate::domains::common::CommandDescriptor::of::<CreateAgentCredentialBinding>() }

fn validate_component(label: &str, value: &str) -> Result<(), CommandError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_NAME_LEN {
        return Err(CommandError::bad_request(format!(
            "{label} must be between 1 and {MAX_NAME_LEN} non-whitespace characters"
        )));
    }
    Ok(())
}

async fn resolve_attached_server_url(
    ctx: &Ctx,
    agent: &crate::storage::models::AgentRow,
    requested_name: &str,
) -> Result<String, CommandError> {
    let scoped: everruns_core::ScopedMcpServers =
        serde_json::from_value(agent.mcp_servers.clone()).unwrap_or_default();
    if let Some(server) = scoped.get(requested_name) {
        return Ok(server.url.clone());
    }

    let capabilities = ctx
        .db
        .get_agent_capabilities(agent.id.uuid())
        .await
        .map_err(classify_anyhow)?;
    for capability in capabilities {
        let capability_id = CapabilityId::new(capability.capability_id);
        let Some(server_id) = capability_id.mcp_server_id() else {
            continue;
        };
        let Some(server) = ctx
            .db
            .get_mcp_server(ctx.org_id(), server_id)
            .await
            .map_err(classify_anyhow)?
        else {
            continue;
        };
        if server.status == "active" && server.name == requested_name {
            return Ok(server.url);
        }
    }

    Err(CommandError::bad_request(format!(
        "MCP server '{requested_name}' is not attached to this Agent"
    )))
}

pub async fn resolve_runtime_secret_bindings(
    db: &crate::storage::StorageBackend,
    encryption: Option<&crate::storage::EncryptionService>,
    org_id: i64,
    agent_id: Option<AgentId>,
    server_name: &str,
    server_url: &str,
) -> anyhow::Result<HashMap<String, Vec<everruns_mcp::McpSecretBinding>>> {
    let Some(agent_id) = agent_id else {
        return Ok(HashMap::new());
    };
    let public_id = db
        .get_agent_public_id(org_id, agent_id)
        .await?
        .unwrap_or_else(|| agent_id.to_string());
    let setup_url = format!("/agents/{public_id}?tab=credentials");
    let rows = db.list_agent_mcp_secret_bindings(org_id, agent_id).await?;
    let mut bindings: HashMap<String, Vec<everruns_mcp::McpSecretBinding>> = HashMap::new();
    for row in rows
        .into_iter()
        .filter(|row| row.mcp_server_name == server_name)
    {
        let value = if row.mcp_server_url != server_url {
            None
        } else {
            match (row.value_encrypted.as_deref(), encryption) {
                (Some(ciphertext), Some(encryption)) => {
                    Some(encryption.decrypt_to_string(ciphertext)?)
                }
                _ => None,
            }
        };
        bindings
            .entry(row.tool_name)
            .or_default()
            .push(everruns_mcp::McpSecretBinding {
                parameter_name: row.parameter_name,
                value,
                setup_url: setup_url.clone(),
                label: row.label,
            });
    }
    Ok(bindings)
}

pub async fn list_secret_binding_metadata(
    db: &crate::storage::StorageBackend,
    org_id: i64,
    agent_id: Option<AgentId>,
) -> anyhow::Result<Vec<everruns_core::McpSecretBindingMetadata>> {
    let Some(agent_id) = agent_id else {
        return Ok(Vec::new());
    };
    let public_id = db
        .get_agent_public_id(org_id, agent_id)
        .await?
        .unwrap_or_else(|| agent_id.to_string());
    let setup_url = format!("/agents/{public_id}?tab=credentials");
    Ok(db
        .list_agent_mcp_secret_bindings(org_id, agent_id)
        .await?
        .into_iter()
        .map(|row| everruns_core::McpSecretBindingMetadata {
            server_name: row.mcp_server_name,
            tool_name: row.tool_name,
            parameter_name: row.parameter_name,
            configured: row.value_encrypted.is_some(),
            setup_url: setup_url.clone(),
        })
        .collect())
}

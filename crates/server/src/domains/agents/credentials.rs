use crate::domains::common::{CliExample, CliRoute, Command, CommandError, Ctx, command};
use crate::kernel_imports::{CapabilityId, contracts::typed_id::AgentId};
use crate::storage::{
    AgentMcpSecretBindingRow, UpsertAgentHttpCredentialRow, UpsertAgentMcpSecretBindingRow,
};
use everruns_contracts::runtime::{SecretAuth, SecretBinding};
use everruns_core::mcp::McpCapabilityIdExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AGENT_MANAGE;

/// Revoke every service-owned grant held by an agent's identity.
///
/// Archival and deletion are the same lifecycle transition as far as grants are
/// concerned, and each has its own API route, so both call this rather than
/// repeating the revocation and risking one of them drifting.
pub(crate) async fn revoke_agent_grants(
    ctx: &Ctx,
    row: &crate::storage::AgentRow,
) -> Result<(), CommandError> {
    if let Some(identity_id) = row.virtual_user_id {
        ctx.db
            .delete_all_virtual_user_connections(identity_id)
            .await?;
    }
    Ok(())
}

const MAX_NAME_LEN: usize = 255;

/// How a write-only agent credential reaches the outside world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentCredentialKind {
    /// Injected into one top-level argument of an attached MCP tool.
    McpParameter,
    /// Injected by the shell's HTTP layer into requests to one URL; the shell
    /// sees `$ENV_NAME` as a placeholder (a bound secret).
    Http,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
/// Metadata for a write-only credential bound to one agent: either an MCP
/// tool parameter or an HTTP API the agent's shell calls.
pub struct AgentCredentialBinding {
    /// Stable credential binding identifier.
    #[schema(example = "01933b5a-0000-7000-8000-000000000001")]
    pub id: Uuid,
    /// Agent that exclusively owns and may use this credential.
    #[schema(example = "agent_01933b5a000070008000000000000001")]
    pub agent_id: String,
    /// How the credential is injected.
    pub kind: AgentCredentialKind,
    /// Name of the attached MCP server (`mcp_parameter`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "visti")]
    pub mcp_server_name: Option<String>,
    /// Exact MCP endpoint authorized to receive the credential (`mcp_parameter`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "https://visti.sh/mcp")]
    pub mcp_server_url: Option<String>,
    /// MCP tool whose outbound call receives the credential (`mcp_parameter`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "visti_send")]
    pub tool_name: Option<String>,
    /// Top-level tool argument injected by the server (`mcp_parameter`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "channel_key")]
    pub parameter_name: Option<String>,
    /// Env var the agent's shell uses as a placeholder (`http`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "JEV_API_KEY")]
    pub env_name: Option<String>,
    /// URL prefix and auth form the value may be sent with (`http`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<SecretBinding>,
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
            kind: if row.kind == "http" {
                AgentCredentialKind::Http
            } else {
                AgentCredentialKind::McpParameter
            },
            mcp_server_name: row.mcp_server_name,
            mcp_server_url: row.mcp_server_url,
            tool_name: row.tool_name,
            parameter_name: row.parameter_name,
            env_name: row.env_name,
            binding: row.binding.map(|binding| binding.0),
            label: row.label,
            description: row.description,
            configured: row.value_encrypted.is_some(),
            created_at: row.created_at.to_rfc3339(),
            updated_at: row.updated_at.to_rfc3339(),
            setup_url: format!("/agents/{public_agent_id}?tab=credentials"),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
/// Declare a credential requirement for an agent: an attached MCP tool's
/// parameter (`mcp_server_name`, `tool_name`, `parameter_name`), or an HTTP
/// API the agent's shell calls (`env_name`, `url`, `auth`).
pub struct CreateAgentCredentialBinding {
    /// Agent ID, populated from the request path by the HTTP API.
    #[serde(default)]
    #[schema(example = "agent_01933b5a000070008000000000000001")]
    pub agent_id: String,
    /// Name of an MCP server attached to the agent (MCP tool parameter).
    #[serde(default)]
    #[schema(example = "visti")]
    pub mcp_server_name: Option<String>,
    /// MCP tool whose outbound call requires the credential (MCP tool parameter).
    #[serde(default)]
    #[schema(example = "visti_send")]
    pub tool_name: Option<String>,
    /// Top-level tool argument to inject outside model context (MCP tool parameter).
    #[serde(default)]
    #[schema(example = "channel_key")]
    pub parameter_name: Option<String>,
    /// Env var name the agent's shell uses as a placeholder (HTTP API).
    #[serde(default)]
    #[schema(example = "JEV_API_KEY")]
    pub env_name: Option<String>,
    /// HTTPS URL prefix the value may be sent to (HTTP API).
    #[serde(default)]
    #[schema(example = "https://api.typesafe.ai")]
    pub url: Option<String>,
    /// `bearer` (default) sends `Authorization: Bearer <value>`; `header`
    /// sends the value in `header_name` (HTTP API).
    #[serde(default)]
    #[schema(example = "bearer")]
    pub auth: Option<String>,
    /// Header that carries the value when `auth` is `header` (HTTP API).
    #[serde(default)]
    #[schema(example = "X-Api-Key")]
    pub header_name: Option<String>,
    /// Human-readable label shown in the secure setup UI.
    #[schema(example = "Visti channel key")]
    pub label: String,
    /// Optional explanation of how the credential will be used.
    #[serde(default)]
    #[schema(example = "Delivers scheduled status notifications")]
    pub description: Option<String>,
}

#[command(
    name = "create_agent_credential_binding",
    category = "agents",
    description = "Create a write-only Agent credential setup requirement: one attached MCP tool parameter, or an HTTP API the agent's shell calls with $ENV_NAME (the value is injected only into requests to the bound URL). This command never accepts a secret value; the user completes setup in the Agent Credentials UI.",
    method = "POST",
    path = "/v1/agents/{agent_id}/credentials",
    policy = AGENT_MANAGE,
    cli = CliRoute::new(&["agents", "credentials"], "create").with_examples(&[CliExample::new("Declare that an MCP tool needs a secret the model must never see", "everruns agents credentials create --agent-id agent_01h9 --mcp-server-name visti --tool-name visti_send --parameter-name channel_key --label 'Visti channel key' --reason 'Notify the on-call channel'",), CliExample::new("Declare an API key the agent's shell sends to one host without seeing it", "everruns agents credentials create --agent-id agent_01h9 --env-name JEV_API_KEY --url https://api.typesafe.ai --label 'Jev API key' --reason 'Ask Jev for triage decisions'",)]),
)]
impl Command for CreateAgentCredentialBinding {
    type Output = AgentCredentialBinding;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        validate_component("label", &self.label)?;
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
            .await?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        if let Some(env_name) = self.env_name {
            if self.mcp_server_name.is_some()
                || self.tool_name.is_some()
                || self.parameter_name.is_some()
            {
                return Err(CommandError::bad_request(
                    "A credential is either an HTTP API (env_name, url) or an MCP tool parameter, not both",
                ));
            }
            let binding = http_binding(
                &env_name,
                self.url.as_deref(),
                self.auth.as_deref(),
                self.header_name,
            )?;
            let row = ctx
                .db
                .upsert_agent_http_credential(UpsertAgentHttpCredentialRow {
                    org_id: ctx.org_id(),
                    agent_id: agent.id,
                    env_name,
                    binding,
                    label: self.label,
                    description: self.description,
                })
                .await?;
            return Ok(AgentCredentialBinding::from_row(row, &public_agent_id));
        }

        if self.url.is_some() || self.auth.is_some() || self.header_name.is_some() {
            return Err(CommandError::bad_request(
                "url, auth and header_name need env_name (an HTTP API credential)",
            ));
        }
        let (Some(mcp_server_name), Some(tool_name), Some(parameter_name)) =
            (self.mcp_server_name, self.tool_name, self.parameter_name)
        else {
            return Err(CommandError::bad_request(
                "Give env_name and url for an HTTP API credential, or mcp_server_name, tool_name and parameter_name for an MCP tool parameter",
            ));
        };
        validate_component("MCP server name", &mcp_server_name)?;
        validate_component("tool name", &tool_name)?;
        validate_component("parameter name", &parameter_name)?;
        if parameter_name.contains('.') {
            return Err(CommandError::bad_request(
                "Only top-level MCP tool parameters can be bound",
            ));
        }

        let mcp_server_url = resolve_attached_server_url(ctx, &agent, &mcp_server_name).await?;

        let row = ctx
            .db
            .upsert_agent_mcp_secret_binding(UpsertAgentMcpSecretBindingRow {
                org_id: ctx.org_id(),
                agent_id: agent.id,
                mcp_server_name,
                mcp_server_url,
                tool_name,
                parameter_name,
                label: self.label,
                description: self.description,
            })
            .await?;
        Ok(AgentCredentialBinding::from_row(row, &public_agent_id))
    }
}

/// Build and validate an HTTP credential's binding: an env var name, an
/// HTTPS URL prefix without credentials, query or fragment, and a header form.
pub(crate) fn http_binding(
    env_name: &str,
    url: Option<&str>,
    auth: Option<&str>,
    header_name: Option<String>,
) -> Result<SecretBinding, CommandError> {
    let url = url.ok_or_else(|| {
        CommandError::bad_request("An HTTP API credential needs the url it may be sent to")
    })?;
    let auth = match (auth.unwrap_or("bearer"), header_name) {
        ("bearer", None) => SecretAuth::Bearer,
        ("header", Some(name)) => SecretAuth::Header { name },
        ("header", None) => {
            return Err(CommandError::bad_request(
                "auth 'header' needs header_name",
            ));
        }
        ("bearer", Some(_)) => {
            return Err(CommandError::bad_request(
                "header_name only applies to auth 'header'",
            ));
        }
        (other, _) => {
            return Err(CommandError::bad_request(format!(
                "Unknown auth '{other}': use 'bearer' or 'header'"
            )));
        }
    };
    crate::domains::session_storage::validate_binding(
        env_name,
        SecretBinding {
            url: url.to_string(),
            auth,
        },
    )
}

/// The internal id of the agent a session runs, for credentials stored on the
/// agent from inside a session (MCP elicitation).
pub(crate) async fn agent_of_session(ctx: &Ctx, session_id: &str) -> Result<AgentId, CommandError> {
    let session_id = crate::domains::session_storage::queries::parse_owned_session_id(session_id)?;
    let session = ctx
        .db
        .get_session(ctx.org_id(), session_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Session"))?;
    session.agent_id.ok_or_else(|| {
        CommandError::bad_request("This session has no agent to store the credential on")
    })
}

/// Declare an HTTP credential on the agent and store its value in one step.
/// Only for flows where the person typing the value is the one approving the
/// binding (MCP URL elicitation): the binding comes from a signed intent.
pub(crate) async fn store_agent_http_credential(
    ctx: &Ctx,
    agent_id: AgentId,
    env_name: &str,
    binding: SecretBinding,
    value: &str,
) -> Result<(), CommandError> {
    let binding = crate::domains::session_storage::validate_binding(env_name, binding)?;
    if value.is_empty() || value.len() > 64 * 1024 {
        return Err(CommandError::bad_request(
            "Credential value must be between 1 byte and 64 KiB",
        ));
    }
    let encryption = ctx.encryption.as_deref().ok_or_else(|| {
        CommandError::bad_request(
            "Encryption not configured. Set SECRETS_ENCRYPTION_KEY environment variable.",
        )
    })?;
    let encrypted = encryption
        .encrypt_string(value)
        .map_err(CommandError::internal)?;
    let row = ctx
        .db
        .upsert_agent_http_credential(UpsertAgentHttpCredentialRow {
            org_id: ctx.org_id(),
            agent_id,
            env_name: env_name.to_string(),
            binding,
            label: env_name.to_string(),
            description: None,
        })
        .await?;
    ctx.db
        .set_agent_mcp_secret_binding_value(ctx.org_id(), agent_id, row.id, encrypted)
        .await?
        .ok_or_else(|| CommandError::not_found("Credential binding"))?;
    Ok(())
}

/// Whether the agent holds a configured HTTP credential with this name and
/// exactly this binding.
pub(crate) async fn agent_http_credential_configured(
    ctx: &Ctx,
    agent_id: AgentId,
    env_name: &str,
    binding: &SecretBinding,
) -> Result<bool, CommandError> {
    Ok(ctx
        .db
        .list_agent_mcp_secret_bindings(ctx.org_id(), agent_id)
        .await?
        .into_iter()
        .any(|row| {
            row.value_encrypted.is_some()
                && row.env_name.as_deref() == Some(env_name)
                && row.binding.as_ref().map(|b| &b.0) == Some(binding)
        }))
}

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
    agent: &crate::storage::AgentRow,
    requested_name: &str,
) -> Result<String, CommandError> {
    let scoped: everruns_core::ScopedMcpServers =
        serde_json::from_value(agent.mcp_servers.clone()).unwrap_or_default();
    if let Some(server) = scoped.get(requested_name) {
        return Ok(server.url.clone());
    }

    let capabilities = ctx.db.get_agent_capabilities(agent.id.uuid()).await?;
    for capability in capabilities {
        let capability_id = CapabilityId::new(capability.capability_id);
        let Some(server_id) = capability_id.mcp_server_id() else {
            continue;
        };
        let Some(server) = ctx.db.get_mcp_server(ctx.org_id(), server_id).await? else {
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
) -> anyhow::Result<HashMap<String, Vec<everruns_core::mcp::McpSecretBinding>>> {
    let Some(agent_id) = agent_id else {
        return Ok(HashMap::new());
    };
    let public_id = db
        .get_agent_public_id(org_id, agent_id)
        .await?
        .unwrap_or_else(|| agent_id.to_string());
    let setup_url = format!("/agents/{public_id}?tab=credentials");
    let rows = db.list_agent_mcp_secret_bindings(org_id, agent_id).await?;
    let mut bindings: HashMap<String, Vec<everruns_core::mcp::McpSecretBinding>> = HashMap::new();
    for row in rows
        .into_iter()
        .filter(|row| row.mcp_server_name.as_deref() == Some(server_name))
    {
        let (Some(row_url), Some(tool_name), Some(parameter_name)) =
            (row.mcp_server_url, row.tool_name, row.parameter_name)
        else {
            continue;
        };
        let value = if row_url != server_url {
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
            .entry(tool_name)
            .or_default()
            .push(everruns_core::mcp::McpSecretBinding {
                parameter_name,
                value,
                setup_url: setup_url.clone(),
                label: row.label,
            });
    }
    Ok(bindings)
}

/// `known_public_id` skips the public id lookup when the caller already
/// loaded the agent.
pub async fn list_secret_binding_metadata(
    db: &crate::storage::StorageBackend,
    org_id: i64,
    agent_id: Option<AgentId>,
    known_public_id: Option<String>,
) -> anyhow::Result<Vec<everruns_core::McpSecretBindingMetadata>> {
    let Some(agent_id) = agent_id else {
        return Ok(Vec::new());
    };
    let public_id = match known_public_id {
        Some(public_id) => public_id,
        None => db
            .get_agent_public_id(org_id, agent_id)
            .await?
            .unwrap_or_else(|| agent_id.to_string()),
    };
    let setup_url = format!("/agents/{public_id}?tab=credentials");
    Ok(db
        .list_agent_mcp_secret_bindings(org_id, agent_id)
        .await?
        .into_iter()
        .filter_map(|row| {
            Some(everruns_core::McpSecretBindingMetadata {
                server_name: row.mcp_server_name?,
                tool_name: row.tool_name?,
                parameter_name: row.parameter_name?,
                configured: row.value_encrypted.is_some(),
                setup_url: setup_url.clone(),
            })
        })
        .collect())
}

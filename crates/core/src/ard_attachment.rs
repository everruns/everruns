//! Runtime ARD (Agentic Resource Discovery) attachments.
//!
//! The `resource_discovery` capability (see `crates/ard`) lets a running
//! agent discover external capabilities through an ARD registry and attach them
//! mid-session. Attachments are persisted as session KV entries (key prefix
//! [`ARD_ATTACHMENT_KV_PREFIX`]) written from the tool side, then folded into
//! the session config layer when the turn context is assembled — so the
//! existing scoped-`mcpServers` and A2A-delegation machinery picks them up with
//! no change to the agent loop.
//!
//! This module is the single source of truth for the KV schema and the merge.
//! It is shared by the integration crate (the writer) and by server/runtime
//! turn-context assembly (the reader). Keeping it in `everruns-core` avoids a
//! dependency from the server onto the leaf integration crate.

use serde::{Deserialize, Serialize};

use crate::capability_types::AgentCapabilityConfig;
use crate::mcp_server::ScopedMcpServer;
use crate::session::ExecutionSession;
use crate::session_services::SessionStorageStore;
use crate::typed_id::SessionId;

/// Session KV key prefix under which `resource_discovery` writes attachments.
///
/// `session_storage` reserves this prefix from the user-facing `kv_store` tool
/// (see `is_internal_session_kv_key`) so session/tool actors cannot forge ARD
/// attachments.
pub const ARD_ATTACHMENT_KV_PREFIX: &str = "ard_attach:";

/// Session KV key prefix under which `resource_discovery` caches catalog
/// entries seen via `discover_resources`, so `attach_resource` can resolve a
/// URN deterministically. Reserved from the user-facing `kv_store` tool.
pub const ARD_DISCOVERY_KV_PREFIX: &str = "ard_disco:";

/// `kind` used when registering an attachment in the session resource registry
/// (for `list_attached_resources` visibility and infra audit).
pub const ARD_ATTACHMENT_RESOURCE_KIND: &str = "ard_attachment";

/// Build the KV key for an attachment, keyed by a stable per-URN slug so repeat
/// attaches of the same URN are idempotent (last write wins, no duplicates).
pub fn attachment_kv_key(slug: &str) -> String {
    format!("{ARD_ATTACHMENT_KV_PREFIX}{slug}")
}

/// The materialized form of an attached resource. Mirrors the two attachment
/// kinds the ARD client supports: a scoped MCP server, or an external A2A agent
/// folded into `a2a_agent_delegation` config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ArdAttachmentTarget {
    /// Attach as a session-scoped `mcpServers` record. `name` is the logical
    /// server name used for `mcp_{name}__{tool}` prefixing.
    McpServer {
        name: String,
        server: ScopedMcpServer,
    },
    /// Attach as a session-scoped external A2A agent. `agent` is an
    /// `a2a_agent_delegation` agent entry (the object shape under its config
    /// `agents[]`), consumed by the existing `spawn_agent` flow.
    ExternalAgent { agent: serde_json::Value },
}

/// A single ARD attachment persisted in session storage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArdAttachment {
    /// URN of the resolved catalog entry (`urn:ai:<publisher>:...`).
    pub urn: String,
    /// Human-readable label from the catalog entry.
    pub display_name: String,
    /// IANA media type of the entry (e.g. `application/mcp-server+json`).
    pub media_type: String,
    /// `registry_id` the entry was resolved from (allowlisted config id).
    pub registry_id: String,
    /// Materialized attachment target.
    pub target: ArdAttachmentTarget,
}

impl ArdAttachment {
    /// Stable slug derived from the URN, used as the KV key suffix and resource
    /// id so the same URN maps to exactly one attachment.
    pub fn slug(&self) -> String {
        urn_slug(&self.urn)
    }
}

/// Derive a filesystem/key-safe slug from a URN. Non-alphanumeric characters
/// collapse to `_` so it is safe as a KV key suffix and resource id.
pub fn urn_slug(urn: &str) -> String {
    urn.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Merge a single attachment into a session's config layer. Idempotent: an MCP
/// server with the same logical name, or an A2A agent with the same `id`, is
/// not duplicated (the attachment wins on conflict, matching last-layer-wins).
pub fn merge_attachment_into_session(session: &mut ExecutionSession, attachment: &ArdAttachment) {
    match &attachment.target {
        ArdAttachmentTarget::McpServer { name, server } => {
            session.mcp_servers.insert(name.clone(), server.clone());
        }
        ArdAttachmentTarget::ExternalAgent { agent } => {
            merge_external_agent(session, agent);
        }
    }
}

/// Fold an external A2A agent object into the session's `a2a_agent_delegation`
/// capability config (creating the capability entry if absent). Adding the
/// capability at the session layer makes `spawn_agent` available scoped to this
/// session even when the base agent was not provisioned with A2A delegation.
fn merge_external_agent(session: &mut ExecutionSession, agent: &serde_json::Value) {
    let Some(agent_id) = agent.get("id").and_then(|v| v.as_str()) else {
        return;
    };

    let cap = session
        .capabilities
        .iter_mut()
        .find(|c| c.capability_id() == crate::capabilities::A2A_AGENT_DELEGATION_CAPABILITY_ID);

    let cap = match cap {
        Some(cap) => cap,
        None => {
            session
                .capabilities
                .push(AgentCapabilityConfig::with_config(
                    crate::capabilities::A2A_AGENT_DELEGATION_CAPABILITY_ID,
                    serde_json::json!({ "agents": [] }),
                ));
            session
                .capabilities
                .last_mut()
                .expect("just pushed a2a capability config")
        }
    };

    if !cap.config_value().is_object() {
        cap.set_config(serde_json::json!({ "agents": [] }));
    }
    let config = cap.config_mut();
    if !config["agents"].is_array() {
        config["agents"] = serde_json::Value::Array(Vec::new());
    }
    let agents = config["agents"]
        .as_array_mut()
        .expect("agents array ensured above");

    let exists = agents
        .iter()
        .any(|a| a.get("id").and_then(|v| v.as_str()) == Some(agent_id));
    if !exists {
        agents.push(agent.clone());
    }
}

/// Load all ARD attachments persisted for a session, oldest-key-first.
/// Best-effort: malformed entries are skipped with a warning rather than
/// failing turn-context assembly.
pub async fn load_session_attachments(
    storage: &dyn SessionStorageStore,
    session_id: SessionId,
) -> Vec<ArdAttachment> {
    let keys = match storage.list_keys(session_id).await {
        Ok(keys) => keys,
        Err(e) => {
            tracing::warn!("failed to list session keys for ARD attachments: {e}");
            return Vec::new();
        }
    };
    let mut attachments = Vec::new();
    for info in keys {
        if !info.key.starts_with(ARD_ATTACHMENT_KV_PREFIX) {
            continue;
        }
        match storage.get_value(session_id, &info.key).await {
            Ok(Some(raw)) => match serde_json::from_str::<ArdAttachment>(&raw) {
                Ok(attachment) => attachments.push(attachment),
                Err(e) => tracing::warn!(key = %info.key, "skipping malformed ARD attachment: {e}"),
            },
            Ok(None) => {}
            Err(e) => tracing::warn!(key = %info.key, "failed to read ARD attachment: {e}"),
        }
    }
    attachments
}

/// Read ARD attachments from session storage and fold them into the session's
/// config layer. Called during turn-context assembly (server `GetTurnContext`
/// and the in-process runtime) after the session record is loaded and before
/// scoped MCP servers / capabilities are resolved into the live tool set.
pub async fn apply_session_attachments(
    storage: &dyn SessionStorageStore,
    session: &mut ExecutionSession,
) {
    let attachments = load_session_attachments(storage, session.id).await;
    for attachment in &attachments {
        merge_attachment_into_session(session, attachment);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_server::McpServerTransportType;

    fn test_session() -> ExecutionSession {
        ExecutionSession::with_own_workspace(SessionId::new(), crate::typed_id::HarnessId::new())
    }

    fn mcp_attachment(urn: &str, name: &str) -> ArdAttachment {
        ArdAttachment {
            urn: urn.to_string(),
            display_name: "Docs MCP".to_string(),
            media_type: "application/mcp-server+json".to_string(),
            registry_id: "public".to_string(),
            target: ArdAttachmentTarget::McpServer {
                name: name.to_string(),
                server: ScopedMcpServer {
                    transport_type: McpServerTransportType::Http,
                    url: "https://docs.example.com/mcp".to_string(),
                    ..Default::default()
                },
            },
        }
    }

    fn agent_attachment(urn: &str, id: &str) -> ArdAttachment {
        ArdAttachment {
            urn: urn.to_string(),
            display_name: "Concierge".to_string(),
            media_type: "application/a2a-agent-card+json".to_string(),
            registry_id: "public".to_string(),
            target: ArdAttachmentTarget::ExternalAgent {
                agent: serde_json::json!({
                    "id": id,
                    "name": "Concierge",
                    "base_url": "https://agent.example.com",
                }),
            },
        }
    }

    #[test]
    fn slug_is_key_safe() {
        assert_eq!(
            urn_slug("urn:ai:acme.com:agent:assistant"),
            "urn_ai_acme_com_agent_assistant"
        );
    }

    #[test]
    fn merges_mcp_server_into_session() {
        let mut session = test_session();
        merge_attachment_into_session(&mut session, &mcp_attachment("urn:ai:x:y:z", "docs"));
        assert!(session.mcp_servers.contains_key("docs"));
        assert_eq!(
            session.mcp_servers["docs"].url,
            "https://docs.example.com/mcp"
        );
    }

    #[test]
    fn mcp_merge_is_idempotent_by_name() {
        let mut session = test_session();
        merge_attachment_into_session(&mut session, &mcp_attachment("urn:ai:x:y:z", "docs"));
        merge_attachment_into_session(&mut session, &mcp_attachment("urn:ai:x:y:z", "docs"));
        assert_eq!(session.mcp_servers.len(), 1);
    }

    #[test]
    fn merges_external_agent_into_new_capability() {
        let mut session = test_session();
        merge_attachment_into_session(&mut session, &agent_attachment("urn:ai:x:y:c", "concierge"));
        let cap = session
            .capabilities
            .iter()
            .find(|c| c.capability_id() == "a2a_agent_delegation")
            .expect("a2a capability added");
        let agents = cap.config_value()["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["id"], "concierge");
    }

    #[test]
    fn external_agent_merge_is_idempotent_by_id() {
        let mut session = test_session();
        merge_attachment_into_session(&mut session, &agent_attachment("urn:ai:x:y:c", "concierge"));
        merge_attachment_into_session(&mut session, &agent_attachment("urn:ai:x:y:c", "concierge"));
        let cap = session
            .capabilities
            .iter()
            .find(|c| c.capability_id() == "a2a_agent_delegation")
            .unwrap();
        assert_eq!(cap.config_value()["agents"].as_array().unwrap().len(), 1);
        // And the capability itself is not duplicated.
        assert_eq!(
            session
                .capabilities
                .iter()
                .filter(|c| c.capability_id() == "a2a_agent_delegation")
                .count(),
            1
        );
    }

    #[test]
    fn external_agent_appends_to_existing_capability() {
        let mut session = test_session();
        session
            .capabilities
            .push(AgentCapabilityConfig::with_config(
                "a2a_agent_delegation",
                serde_json::json!({ "agents": [ {"id": "preexisting", "name": "Pre"} ] }),
            ));
        merge_attachment_into_session(&mut session, &agent_attachment("urn:ai:x:y:c", "concierge"));
        let cap = session
            .capabilities
            .iter()
            .find(|c| c.capability_id() == "a2a_agent_delegation")
            .unwrap();
        let ids: Vec<&str> = cap.config_value()["agents"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|a| a["id"].as_str())
            .collect();
        assert_eq!(ids, vec!["preexisting", "concierge"]);
    }
}

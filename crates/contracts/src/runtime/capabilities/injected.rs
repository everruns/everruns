//! Injected capabilities: hosted capabilities that a controller registers with
//! a data plane and serves itself, over MCP.
//!
//! Decisions:
//! - An injected capability is a declarative capability plus one MCP server
//!   that the controller runs. It reuses the declarative shape (name,
//!   description, system prompt, skills, files) instead of a new concept, and
//!   [`InjectedCapabilityDefinition::to_declarative`] hands hosts a definition
//!   they already know how to collect.
//! - The server always authenticates with a session token: a short-lived JWT
//!   the data plane signs per call, scoped to one session, one group and one
//!   server URL ([`SessionTokenClaims`]). The audience is the exact server URL,
//!   so a token minted for one server never verifies at another, including a
//!   look-alike capability someone else registered.
//! - Only a controller registers injected capabilities. User-authored MCP
//!   servers (declarative, plugin, scoped attachments) may not use
//!   [`McpServerAuthMode::SessionToken`]; see
//!   [`reject_session_token_servers`].

use super::declarative::DeclarativeCapabilityDefinition;
use crate::runtime::capability_mcp_server::{CapabilityMcpServer, CapabilityMcpServers};
use crate::runtime::mcp_server::{
    McpServerActsAs, McpServerAuthMode, ScopedMcpServer, validate_mcp_tool_timeout_ms,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const INJECTED_CAPABILITY_PREFIX: &str = "injected:";
// Same budget as declarative names: refs persist in VARCHAR(50) columns and
// `injected:` is 9 bytes.
const MAX_NAME_BYTES: usize = 40;
const MAX_LABELS: usize = 16;
const MAX_LABEL_BYTES: usize = 128;

/// JWT `typ` header of a session token. Verification requires it, so no other
/// token the data plane signs can be replayed as one.
/// The control plane's files server for a session's Memory
/// (`io.everruns/files`): a data plane mounts the session's Memory from it.
pub const MEMORY_FILES: &str = "memory_files";
/// The control plane's server for the platform commands of a session's
/// native Platform tools and shell builtin, run as the token's `act`.
pub const PLATFORM_COMMANDS: &str = "platform_commands";
/// [`PLATFORM_COMMANDS`]' one tool: `{operation, arguments}`.
pub const PLATFORM_COMMAND_TOOL: &str = "platform_command";
/// The control plane's server for the turn's person's own MCP servers
/// (`user_mcp`), as the token's `act`. Not named `user_mcp`: a controller
/// server's name never matches a hosted capability.
pub const PERSON_MCP: &str = "person_mcp";
/// [`PERSON_MCP`]'s tool for the `user_mcp` manage tools' store: `{call}`.
pub const PERSON_MCP_STORE_TOOL: &str = "person_mcp_store";
/// [`PERSON_MCP`]'s tool for the servers the turn adds:
/// `{session_servers}`.
pub const PERSON_MCP_SERVERS_TOOL: &str = "person_mcp_servers";
/// The control plane's servers a data plane calls for a session's native
/// capabilities rather than lists as capabilities: their URLs come with the
/// session's create, never from an agent's package.
/// The control plane's server for the credentials a session's native
/// integrations use (provider connections, MCP sign-ins, GitHub, service
/// keys), as the token's `act`: the one lookup that crosses planes.
pub const CREDENTIALS: &str = "credentials";
/// [`CREDENTIALS`]' one tool: `{call, params}`, `call` one of
/// [`CREDENTIAL_CALLS`]. Answers `{"ok": value}` or
/// `{"error": {"kind", "message"}}` as text.
pub const CREDENTIAL_TOOL: &str = "credential";
/// A provider connection token, as the turn's person or the agent.
pub const CONNECTION_TOKEN: &str = "connection_token";
/// The token of an MCP server that acts as the turn's person (`user`, or
/// `user_or_service` falling back to the agent's grant), or of a
/// connection-backed preset; refreshed in the control plane.
pub const MCP_CONNECTION_TOKEN: &str = "mcp_connection_token";
/// A GitHub token minted for one use.
pub const GITHUB_TOKEN: &str = "github_token";
/// The responding agent's service API key.
pub const SERVICE_API_KEY: &str = "service_api_key";
/// The credential lookups [`CREDENTIALS`] serves.
pub const CREDENTIAL_CALLS: [&str; 4] = [
    CONNECTION_TOKEN,
    MCP_CONNECTION_TOKEN,
    GITHUB_TOKEN,
    SERVICE_API_KEY,
];
pub const CONTROLLER_SERVERS: [&str; 4] =
    [MEMORY_FILES, PLATFORM_COMMANDS, PERSON_MCP, CREDENTIALS];

pub const SESSION_TOKEN_TYP: &str = "everruns-session+jwt";
/// How long a minted session token stays valid. Short, like the host-call
/// receipts it replaces: a token is minted per call, never stored.
pub const SESSION_TOKEN_TTL_SECS: u64 = 300;
/// Well-known path where a data plane publishes the keys that verify its
/// session tokens.
pub const SESSION_TOKEN_JWKS_PATH: &str = "/.well-known/jwks.json";

pub fn injected_capability_id(name: &str) -> String {
    format!("{INJECTED_CAPABILITY_PREFIX}{name}")
}

pub fn is_injected_capability(capability_id: &str) -> bool {
    capability_id.starts_with(INJECTED_CAPABILITY_PREFIX)
}

pub fn parse_injected_capability_id(capability_id: &str) -> Option<&str> {
    capability_id.strip_prefix(INJECTED_CAPABILITY_PREFIX)
}

/// When an injected capability joins a session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectedAttach {
    /// Only sessions whose agent lists `injected:{name}` get it.
    #[default]
    OnRequest,
    /// Every session whose labels match gets it, listed or not.
    Always,
}

/// How the capability's tools are named for the model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectedToolNames {
    /// `mcp_{name}_{tool}`, like any MCP server.
    #[default]
    Prefixed,
    /// The server's own tool names, unprefixed. Lets a hosted capability move
    /// to injection without renaming the tools agents already call.
    Native,
}

/// The server that serves an injected capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjectedServer {
    /// Streamable HTTP MCP endpoint. It is also the session token audience.
    pub url: String,
    /// How long one `tools/call` may take, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_timeout_ms: Option<u64>,
}

/// A hosted capability a controller registers with a data plane
/// (`PUT /v1/capabilities/{name}` on agentd).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectedCapabilityDefinition {
    /// Catalog metadata, prompt, skills and files, in the declarative shape.
    /// `mcp_servers` must stay empty: [`Self::server`] is the server.
    #[serde(flatten)]
    pub capability: DeclarativeCapabilityDefinition,
    pub server: InjectedServer,
    #[serde(default)]
    pub attach: InjectedAttach,
    /// Session labels that must all match for the capability to apply. Empty
    /// matches every session.
    #[serde(default, rename = "match", skip_serializing_if = "BTreeMap::is_empty")]
    pub match_labels: BTreeMap<String, String>,
    #[serde(default)]
    pub tools: InjectedToolNames,
}

impl InjectedCapabilityDefinition {
    pub fn name(&self) -> &str {
        &self.capability.name
    }

    pub fn id(&self) -> String {
        injected_capability_id(&self.capability.name)
    }

    /// Whether every `match` label is present with the same value.
    pub fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        self.match_labels
            .iter()
            .all(|(key, value)| labels.get(key) == Some(value))
    }

    /// Whether a session with these labels, whose agent lists
    /// `capability_refs`, gets this capability.
    pub fn applies<'a>(
        &self,
        labels: &BTreeMap<String, String>,
        mut capability_refs: impl Iterator<Item = &'a str>,
    ) -> bool {
        if !self.matches(labels) {
            return false;
        }
        match self.attach {
            InjectedAttach::Always => true,
            InjectedAttach::OnRequest => {
                let id = self.id();
                capability_refs.any(|r| r == id)
            }
        }
    }

    /// The MCP server contribution: one session-token server named after the
    /// capability.
    pub fn mcp_servers(&self) -> CapabilityMcpServers {
        let server = ScopedMcpServer {
            url: self.server.url.clone(),
            auth_mode: McpServerAuthMode::SessionToken,
            tool_timeout_ms: self.server.tool_timeout_ms,
            description: Some(self.capability.description.clone()),
            ..ScopedMcpServer::default()
        };
        BTreeMap::from([(
            self.capability.name.clone(),
            CapabilityMcpServer::new(server, McpServerActsAs::None),
        )])
    }

    /// The definition hosts collect: the declarative shape with the server
    /// attached.
    pub fn to_declarative(&self) -> DeclarativeCapabilityDefinition {
        let mut definition = self.capability.clone();
        definition.mcp_servers = Some(self.mcp_servers());
        definition
    }

    pub fn validate(&self) -> Result<(), String> {
        let name = &self.capability.name;
        if name.is_empty() || name.len() > MAX_NAME_BYTES {
            return Err(format!("name must be 1..={MAX_NAME_BYTES} bytes"));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err("name may contain only a-z, 0-9, '_' and '-'".to_string());
        }
        if self
            .capability
            .mcp_servers
            .as_ref()
            .is_some_and(|s| !s.is_empty())
        {
            return Err("mcp_servers must be empty; server is the capability's server".to_string());
        }
        // Validate the rest of the declarative shape with the name budget it
        // has there; the name was checked above.
        let mut declarative = self.capability.clone();
        declarative.name = "injected".to_string();
        declarative.mcp_servers = None;
        super::declarative::validate_declarative_capability_definition(&declarative)?;
        validate_server_url(&self.server.url)?;
        validate_mcp_tool_timeout_ms(self.server.tool_timeout_ms)?;
        if self.match_labels.len() > MAX_LABELS {
            return Err(format!(
                "match cannot contain more than {MAX_LABELS} labels"
            ));
        }
        for (key, value) in &self.match_labels {
            if key.is_empty() || key.len() > MAX_LABEL_BYTES || value.len() > MAX_LABEL_BYTES {
                return Err(format!(
                    "match labels must have a 1..={MAX_LABEL_BYTES} byte key and a value of at most {MAX_LABEL_BYTES} bytes"
                ));
            }
        }
        Ok(())
    }
}

fn validate_server_url(url: &str) -> Result<(), String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| "server.url must be an http(s) URL".to_string())?;
    if rest.is_empty() || rest.starts_with('/') || url.contains(char::is_whitespace) {
        return Err("server.url must name a host".to_string());
    }
    if url.contains('#') {
        return Err("server.url must not carry a fragment".to_string());
    }
    Ok(())
}

/// Refuse session-token servers in user-authored MCP server sets. Session
/// tokens are minted only for servers a controller registered.
pub fn reject_session_token_servers<'a>(
    servers: impl IntoIterator<Item = (&'a String, &'a ScopedMcpServer)>,
) -> Result<(), String> {
    for (name, server) in servers {
        if server.auth_mode == McpServerAuthMode::SessionToken {
            return Err(format!(
                "MCP server '{name}' cannot use session_token auth; only injected capabilities may"
            ));
        }
    }
    Ok(())
}

/// Claims of a session token.
///
/// The data plane signs one per call to an injected capability's server. The
/// server verifies signature, `typ`, `aud` (its own URL) and expiry, then acts
/// for `sub` in `grp`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTokenClaims {
    /// Data plane that signed it (its base URL or install id).
    pub iss: String,
    /// The injected capability's server URL.
    pub aud: String,
    /// Session id.
    pub sub: String,
    /// Injected capability id (`injected:{name}`).
    pub cap: String,
    /// Value of the install's group label for the session (the organization
    /// in an Everruns deployment). Absent on installs without a group label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grp: Option<String>,
    /// The person whose message opened the session's turn, as the data
    /// plane recorded it (RFC 8693 `act`). Absent when no person did (a
    /// channel message, a trigger). A server that acts as a person rechecks
    /// that they are a member of `grp`, and refuses without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub act: Option<SessionActor>,
    pub iat: u64,
    pub exp: u64,
    /// Unique token id, for audit.
    pub jti: String,
}

/// The `act` claim of a session token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionActor {
    /// The person's public user id.
    pub sub: String,
    /// The input message of the turn the person opened, from the same
    /// session record as `sub`: a server that acts for the turn (its
    /// invocation, its `user_mcp` layer) names it from here, never from a
    /// tool's arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn knowledge() -> InjectedCapabilityDefinition {
        serde_json::from_value(json!({
            "name": "knowledge",
            "description": "Search organization Knowledge Bases.",
            "system_prompt": "Consult search_knowledge first.",
            "server": { "url": "https://cp.example/v1/injected/knowledge/mcp" },
            "attach": "always",
            "match": { "org": "org_1" },
            "tools": "native"
        }))
        .unwrap()
    }

    #[test]
    fn parses_validates_and_round_trips() {
        let definition = knowledge();
        definition.validate().unwrap();
        assert_eq!(definition.id(), "injected:knowledge");
        assert_eq!(definition.attach, InjectedAttach::Always);
        assert_eq!(definition.tools, InjectedToolNames::Native);
        let wire = serde_json::to_value(&definition).unwrap();
        assert_eq!(wire["match"], json!({"org": "org_1"}));
        assert_eq!(
            wire["server"]["url"],
            "https://cp.example/v1/injected/knowledge/mcp"
        );
        let back: InjectedCapabilityDefinition = serde_json::from_value(wire).unwrap();
        assert_eq!(back.match_labels, definition.match_labels);
    }

    #[test]
    fn defaults_are_on_request_and_prefixed() {
        let definition: InjectedCapabilityDefinition = serde_json::from_value(json!({
            "name": "poppy",
            "description": "Poppy",
            "server": { "url": "http://localhost:9000/mcp" }
        }))
        .unwrap();
        definition.validate().unwrap();
        assert_eq!(definition.attach, InjectedAttach::OnRequest);
        assert_eq!(definition.tools, InjectedToolNames::Prefixed);
        assert!(definition.matches(&BTreeMap::new()));
    }

    #[test]
    fn applies_by_labels_and_attach() {
        let mut definition = knowledge();
        let org1 = BTreeMap::from([("org".to_string(), "org_1".to_string())]);
        let org2 = BTreeMap::from([("org".to_string(), "org_2".to_string())]);
        assert!(definition.applies(&org1, std::iter::empty()));
        assert!(!definition.applies(&org2, std::iter::empty()));
        assert!(!definition.applies(&BTreeMap::new(), std::iter::empty()));

        definition.attach = InjectedAttach::OnRequest;
        assert!(!definition.applies(&org1, ["web_fetch"].into_iter()));
        assert!(definition.applies(&org1, ["injected:knowledge"].into_iter()));
        assert!(!definition.applies(&org2, ["injected:knowledge"].into_iter()));
    }

    #[test]
    fn declarative_form_carries_one_session_token_server() {
        let declarative = knowledge().to_declarative();
        let servers = declarative.mcp_servers.unwrap();
        let server = &servers["knowledge"];
        assert_eq!(server.auth_mode, McpServerAuthMode::SessionToken);
        assert_eq!(server.acts_as, McpServerActsAs::None);
        assert_eq!(server.url, "https://cp.example/v1/injected/knowledge/mcp");
        assert_eq!(
            server.description.as_deref(),
            Some("Search organization Knowledge Bases.")
        );
    }

    #[test]
    fn rejects_bad_definitions() {
        let mut definition = knowledge();
        definition.capability.name = "Knowledge".to_string();
        assert!(definition.validate().is_err());

        let mut definition = knowledge();
        definition.server.url = "ftp://cp.example/mcp".to_string();
        assert!(definition.validate().is_err());

        let mut definition = knowledge();
        definition.server.url = "https:///mcp".to_string();
        assert!(definition.validate().is_err());

        let mut definition = knowledge();
        definition.capability.mcp_servers = Some(definition.mcp_servers());
        assert!(definition.validate().unwrap_err().contains("mcp_servers"));

        let mut definition = knowledge();
        definition.capability.description = String::new();
        assert!(definition.validate().is_err());
    }

    #[test]
    fn user_authored_servers_cannot_use_session_tokens() {
        let token_server = ScopedMcpServer {
            url: "https://evil.example/mcp".to_string(),
            auth_mode: McpServerAuthMode::SessionToken,
            ..ScopedMcpServer::default()
        };
        let plain = ScopedMcpServer {
            url: "https://ok.example/mcp".to_string(),
            ..ScopedMcpServer::default()
        };
        let name = "x".to_string();
        assert!(reject_session_token_servers([(&name, &plain)]).is_ok());
        assert!(
            reject_session_token_servers([(&name, &token_server)])
                .unwrap_err()
                .contains("session_token")
        );
    }

    #[test]
    fn claims_wire_shape() {
        let claims = SessionTokenClaims {
            iss: "https://dp.example".into(),
            aud: "https://cp.example/v1/injected/knowledge/mcp".into(),
            sub: "ses_1".into(),
            cap: "injected:knowledge".into(),
            grp: None,
            act: None,
            iat: 1,
            exp: 2,
            jti: "j".into(),
        };
        let wire = serde_json::to_value(&claims).unwrap();
        assert!(wire.get("grp").is_none());
        assert!(wire.get("act").is_none());
        assert_eq!(
            serde_json::from_value::<SessionTokenClaims>(wire).unwrap(),
            claims
        );
        // RFC 8693: the actor is an object.
        let acted = SessionTokenClaims {
            act: Some(SessionActor {
                sub: "usr_1".into(),
                turn: None,
            }),
            ..claims
        };
        let wire = serde_json::to_value(&acted).unwrap();
        assert_eq!(wire["act"], json!({ "sub": "usr_1" }));
    }
}

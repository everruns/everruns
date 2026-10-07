//! A person's own MCP servers, as a host-neutral store and login prompter.
//!
//! Decision: these two traits are the whole seam the `user_mcp` capability's
//! *manage* tools need, so the same tools run in every host. The hosted control
//! plane stores entries as rows owned by a virtual user and prompts with the
//! in-chat Connect card; a terminal host keeps them in its settings file and
//! prompts with a loopback callback and the browser. The shape mirrors yolop's
//! `McpConfigStore` (list, upsert, remove, set enabled) on purpose.
//!
//! Decision: a store acts on exactly one person, fixed when the host builds it
//! (for a hosted turn, the turn's verified initiating person). No method takes
//! a person, so a tool holding the store has nothing it could vary to reach
//! someone else's list.
//!
//! Decision: [`McpLoginPrompter`] never handles a credential. It starts a sign-in
//! the person completes in their own browser and reports whether that is done.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::ScopedMcpServer;

pub use everruns_contracts::runtime::mcp_server::{
    USER_MCP_CAPABILITY_ID, USER_MCP_CONNECT_SETTING,
};

/// One server in a person's list: whether it is enabled, and how to reach it.
///
/// A server added from an organization catalog sets `server.preset`
/// (`catalog:<name>`) and nothing else; a custom one sets `server.url`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserMcpServerEntry {
    /// Disabled servers are kept but never offered to agents.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// The server definition.
    #[serde(flatten)]
    pub server: ScopedMcpServer,
}

fn default_enabled() -> bool {
    true
}

impl UserMcpServerEntry {
    /// An enabled entry for `server`.
    pub fn enabled(server: ScopedMcpServer) -> Self {
        Self {
            enabled: true,
            server,
        }
    }
}

/// Whether the person has signed in to one of their servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserMcpLoginStatus {
    /// Signed in, or a key is stored.
    Connected,
    /// Needs a sign-in before its tools can be used.
    NotConnected,
    /// The server needs no sign-in.
    NotNeeded,
}

/// A server in a person's list, as the manage tools report it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserMcpServerSummary {
    /// Name; also the tool prefix agents see.
    pub name: String,
    /// Whether the server joins the person's turns.
    pub enabled: bool,
    /// Catalog preset name, for servers added from a catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<String>,
    /// Endpoint URL, for display. Never carries credentials.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// Sign-in state.
    pub login: UserMcpLoginStatus,
    /// Why the server is not used in this conversation even though it is
    /// enabled, e.g. a server of the agent with the same name wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

/// Why a store or prompter refused a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum UserMcpStoreError {
    /// No server with that name in the person's list.
    #[error("No MCP server named '{0}' in your list")]
    NotFound(String),
    /// The request was refused as given (bad name, URL, duplicate, limit).
    #[error("{0}")]
    Invalid(String),
    /// There is no person to act for, or this conversation cannot change
    /// their list (an unattended run, a conversation with several people).
    #[error("{0}")]
    Unavailable(String),
    /// The host failed; the message is not meant for the model.
    #[error("{0}")]
    Internal(String),
}

/// Result of a store or prompter call.
pub type UserMcpStoreResult<T> = std::result::Result<T, UserMcpStoreError>;

/// A person's own MCP server list.
#[async_trait]
pub trait UserMcpStore: Send + Sync {
    /// Every server in the list, enabled or not.
    async fn list(&self) -> UserMcpStoreResult<Vec<UserMcpServerSummary>>;

    /// Add `name`, or replace the entry already under that name.
    async fn upsert(
        &self,
        name: &str,
        entry: UserMcpServerEntry,
    ) -> UserMcpStoreResult<UserMcpServerSummary>;

    /// Remove `name`. `false` when it was not in the list.
    async fn remove(&self, name: &str) -> UserMcpStoreResult<bool>;

    /// Enable or disable `name`.
    async fn set_enabled(
        &self,
        name: &str,
        enabled: bool,
    ) -> UserMcpStoreResult<UserMcpServerSummary>;
}

/// Where a started sign-in stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum McpLogin {
    /// The server needs no sign-in.
    NotNeeded,
    /// The person is already signed in.
    AlreadyConnected,
    /// The person finished signing in during this call.
    Completed,
    /// The person was asked to sign in and has not finished yet. A hosted
    /// prompter reports this: the Connect card resumes the conversation once
    /// they have.
    Pending {
        /// Connection provider the sign-in is for.
        provider: String,
        /// Where the person can finish it if no card can be shown.
        setup_url: String,
        /// The sign-in is the agent's own (a `service` attachment): someone
        /// with MCP management permission authorizes it, never the person
        /// chatting as themselves.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        for_agent: bool,
        /// The agent's attachment says `connectInChat: never`: the tool hands
        /// the model `setup_url` instead of showing a card. Absent means
        /// `ask`, so an older peer keeps showing the card.
        #[serde(default, skip_serializing_if = "crate::McpConnectInChat::is_ask")]
        connect_in_chat: crate::McpConnectInChat,
    },
}

/// Starts a sign-in to one of the person's MCP servers.
#[async_trait]
pub trait McpLoginPrompter: Send + Sync {
    /// Start signing in to `name` and report where that stands.
    async fn start_login(&self, name: &str) -> UserMcpStoreResult<McpLogin>;
}

/// One store or prompter call, for hosts that forward them across a process
/// boundary (the hosted worker sends these to the control plane).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum UserMcpStoreCall {
    /// [`UserMcpStore::list`].
    List,
    /// [`UserMcpStore::upsert`].
    Upsert {
        /// Server name.
        name: String,
        /// The entry to store. Boxed: it dwarfs the other calls.
        entry: Box<UserMcpServerEntry>,
    },
    /// [`UserMcpStore::remove`].
    Remove {
        /// Server name.
        name: String,
    },
    /// [`UserMcpStore::set_enabled`].
    SetEnabled {
        /// Server name.
        name: String,
        /// The new state.
        enabled: bool,
    },
    /// [`McpLoginPrompter::start_login`].
    StartLogin {
        /// Server name.
        name: String,
    },
}

/// The answer to a [`UserMcpStoreCall`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum UserMcpStoreReply {
    /// Answer to [`UserMcpStoreCall::List`].
    Servers {
        /// Every server in the list.
        servers: Vec<UserMcpServerSummary>,
    },
    /// Answer to [`UserMcpStoreCall::Upsert`] and [`UserMcpStoreCall::SetEnabled`].
    Server {
        /// The server as stored.
        server: UserMcpServerSummary,
    },
    /// Answer to [`UserMcpStoreCall::Remove`].
    Removed {
        /// Whether the server was in the list.
        removed: bool,
    },
    /// Answer to [`UserMcpStoreCall::StartLogin`].
    Login {
        /// Where the sign-in stands.
        login: McpLogin,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_default_to_enabled_and_flatten_the_server() {
        let entry: UserMcpServerEntry =
            serde_json::from_value(serde_json::json!({"url": "https://mcp.example.com/mcp"}))
                .unwrap();
        assert!(entry.enabled);
        assert_eq!(entry.server.url, "https://mcp.example.com/mcp");
        let wire = serde_json::to_value(&entry).unwrap();
        assert_eq!(wire["enabled"], true);
        assert_eq!(wire["url"], "https://mcp.example.com/mcp");
    }

    #[test]
    fn calls_and_errors_round_trip() {
        let call = UserMcpStoreCall::SetEnabled {
            name: "linear".into(),
            enabled: false,
        };
        let wire = serde_json::to_string(&call).unwrap();
        assert_eq!(
            serde_json::from_str::<UserMcpStoreCall>(&wire).unwrap(),
            call
        );
        let error = UserMcpStoreError::Unavailable("no person".into());
        let wire = serde_json::to_string(&error).unwrap();
        assert_eq!(
            serde_json::from_str::<UserMcpStoreError>(&wire).unwrap(),
            error
        );
        let login = McpLogin::Pending {
            provider: "mcp_oauth_1".into(),
            setup_url: "/settings".into(),
            for_agent: false,
            connect_in_chat: crate::McpConnectInChat::Ask,
        };
        let wire = serde_json::to_value(&login).unwrap();
        assert_eq!(wire["status"], "pending");
        // A person's own sign-in keeps the shape it had before agent sign-ins.
        assert!(wire.get("for_agent").is_none());
        assert!(wire.get("connect_in_chat").is_none());
        let agent = McpLogin::Pending {
            provider: "mcp_oauth_1".into(),
            setup_url: "/agents/agent_1?tab=mcp".into(),
            for_agent: true,
            connect_in_chat: crate::McpConnectInChat::Never,
        };
        let wire = serde_json::to_string(&agent).unwrap();
        assert!(wire.contains(r#""connect_in_chat":"never""#), "{wire}");
        assert_eq!(serde_json::from_str::<McpLogin>(&wire).unwrap(), agent);
    }
}

//! Host calls: the one way a data plane asks its control plane anything at
//! call time (data plane v2 parity, Phase A).
//!
//! Decisions:
//! - A cell is an agent runtime that runs without a control plane. Its
//!   traffic with one is of four kinds: runtime (inside the data plane),
//!   desired state (pushed before it is needed), reports (the feed), and
//!   host calls. Only a host call asks at call time, and only when the
//!   answer needs the control plane's authority now: acting as a person,
//!   minting a short-lived credential, acting as a channel bot, or an
//!   org-wide service the data plane does not host.
//! - Every host call is declared here, with who it acts as, its effect,
//!   its timeout and what happens while the control plane is down. The
//!   registry is the whole list of what a data plane needs from a control
//!   plane: `host_calls_registry.txt` prints it, and a test fails until a
//!   change to the registry changes that file too, so a new call shows in
//!   review.
//! - Each call says what it is planned to become (`Fate`). Four of today's
//!   calls are values known before the call (web search settings and keys,
//!   egress grants, the approval policy): the org's are pushed with its
//!   config (`dataplane::ORG_SETTING`), and the call stays only as the
//!   fallback until they arrive and for the session owner's own approval
//!   settings. The session's SQL databases were the one call of session
//!   state; they moved into the runtime (parity step 9) and are no call
//!   now. A connection token or a service key stays a call: which one
//!   applies depends on the turn's person and responder, and the control
//!   plane refreshes OAuth grants.
//! - The path is fixed: executor, the session's directory cell, then
//!   `POST /dp/v1/tenants/{org}/host-calls/{name}` with the link token. The
//!   cell stamps what it verified (the acting person) and limits each org's
//!   calls in flight; the control plane runs the call's one handler and
//!   seals the answer with the org's data key under [`host_call_secret`],
//!   so the cells between hold ciphertext only and an answer cannot stand
//!   in for another call's.
//! - Reads are retried once on a transport failure; writes never are.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Who the control plane acts as when it answers a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActsAs {
    /// The org's internal caller, with the org's feature flags.
    Org,
    /// The person who sent the turn's input message, as the session's cell
    /// recorded them, re-checked against current membership and role. The
    /// control plane records the turn's invocation from it, as v1 does when
    /// the message comes through its API, so its credential checks run
    /// unchanged; no person (a channel message, a trigger) means the turn
    /// acts as no one, and only the agent's own connections apply.
    Person,
    /// The bot of the channel endpoint the session came through.
    ChannelBot,
}

/// What a call does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Read,
    Write,
}

/// What a call does while the control plane cannot be reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhenDown {
    /// The tool call fails. A credential or a policy never falls back to a
    /// default.
    Fail,
}

/// What a call is planned to become.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fate {
    /// Stays a host call: it needs the control plane's authority now.
    HostCall,
    /// A value known before the call: moves to pushed state.
    Pushed,
    /// Session state: moves into the data plane's runtime.
    Runtime,
}

/// One declared host call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostCall {
    /// The call's name on the wire (`host-calls/{name}`).
    pub name: &'static str,
    /// Bumped when params or answer change incompatibly.
    pub version: u32,
    /// The v1 worker command that answers it, which the data plane's shared
    /// resolvers issue (`CommandConnectionResolver` and friends).
    pub command: &'static str,
    pub acts_as: ActsAs,
    pub effect: Effect,
    pub timeout_secs: u64,
    pub when_down: WhenDown,
    pub fate: Fate,
    /// What a tool uses it for.
    pub purpose: &'static str,
}

impl HostCall {
    /// Whether a transport failure may be retried.
    pub fn retries(&self) -> bool {
        self.effect == Effect::Read
    }

    pub fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.timeout_secs)
    }
}

/// The host call that runs a platform tool's call (discover, query,
/// execute, run) on the control plane's command catalog, as the person who
/// sent the turn's input message. Its params name the session and the input
/// message; the cell sets `acting_user` from the session's own record.
pub const PLATFORM_COMMAND: &str = "platform_command";
/// The host call that performs a Slack action (post, react, update,
/// upload, look up a user) as the bot of the session's Slack endpoint. Its
/// params name the session and the action (`slack_action::SlackAction`);
/// the answer is `{"outcome": ...}` or `{"error": ...}`.
pub const SLACK_ACTION: &str = "slack_action";
/// The host call that runs one file operation in a session's Memory
/// (`/memory/agent`, `/memory/user`, `/memory/shared`, `/team`, `/room`),
/// which stays in the control plane (`op`: `read`, `write`,
/// `write_if_matches`, `create_if_absent`, `delete`, `list`, `stat`,
/// `mkdir`, with its `path`); the answer is `{"ok": ...}` or
/// `{"error": message}`. Declared by a session the control plane gave
/// Memory mounts.
pub const MEMORY_FILE: &str = "memory_file";
/// The host call behind the `user_mcp` tools that manage the turn's
/// person's own MCP servers (catalog, list, add, remove, enable, connect).
/// Its params name the session, `input_message_id: "open"` ([`OPEN_TURN`])
/// and the call (`mcp::UserMcpStoreCall`); the answer is `{"ok": reply}`
/// or `{"error": ...}` (`mcp::UserMcpStoreError`). The control plane
/// resolves the person from the turn, never from the call.
pub const USER_MCP_STORE: &str = "user_mcp_store";
/// The host call that gives a turn its person's own MCP servers (the
/// `user_mcp` capability with `use` on), resolved into what an executor
/// runs: a catalog server's address, a key as a literal header, a server
/// that signs in as the person with its OAuth provider, whose token is
/// then [`MCP_CONNECTION_TOKEN`] per use. Its params name the session and
/// `input_message_id: "open"`; the answer is `{"ok": servers}`, empty when
/// the turn has no single person. With `session_servers: true` the answer
/// also holds the session's own MCP records, which the control plane reads
/// from the session's cell; then `input_message_id` may be left out.
pub const USER_MCP_SERVERS: &str = "user_mcp_servers";
/// Register (or update) one of a session's resources: a fleet sandbox, an
/// activated skill. A session's resources stay in the control plane, which
/// lists them in the console (the Resources panel).
pub const SESSION_RESOURCE_REGISTER: &str = "session_resource_register";
/// Change the status of one of a session's resources.
pub const SESSION_RESOURCE_STATUS: &str = "session_resource_status";
/// A session's resources, optionally by kind and status.
pub const SESSION_RESOURCES_LIST: &str = "session_resources_list";
/// Remove one of a session's resources.
pub const SESSION_RESOURCE_DEREGISTER: &str = "session_resource_deregister";
/// What the skills capabilities ask: an activated skill is a session
/// resource, so a second activation in the session answers from it.
pub const SKILL_ACTIVATIONS: &[&str] = &[SESSION_RESOURCES_LIST, SESSION_RESOURCE_REGISTER];
/// Take or renew the lease of something a tool started at a provider (a
/// sandbox, a browser). Leases stay in the control plane, whose sweeper
/// stops what a session left running when its lease ends.
pub const LEASED_RESOURCE_UPSERT: &str = "leased_resource_upsert";
/// Release a lease, when the tool stopped what it started.
pub const LEASED_RESOURCE_RELEASE: &str = "leased_resource_release";
/// A session's leases.
pub const LEASED_RESOURCES_LIST: &str = "leased_resources_list";
/// The model a spawned subagent's helper key resolves to in the org's
/// catalog (`helper_models`). Its params name the parent session and the
/// spawn's helper model; the answer is `{"provider": key, "model": name}`
/// for the child's definition, or `null` to keep the parent agent's model.
pub const HELPER_MODEL: &str = "helper_model";
/// Dreaming's maintenance state for a session the data plane runs (turn
/// counters, the dream job, daily budgets), and the org's due dreams.
pub const DREAMING_STATE: &str = "dreaming_state";
/// Keep a task push config's signing secret in the control plane, which
/// delivers the webhooks: the cell keeps only that the config has one.
pub const TASK_PUSH_SECRET: &str = "task_push_secret";
/// Record a subagent the data plane started as a session of the control
/// plane (its parent's owner, agent and org), so the calls that read a
/// session's record serve it, and answer its Memory mounts.
pub const SUBAGENT_RECORD: &str = "subagent_record";

const fn call(
    name: &'static str,
    command: &'static str,
    acts_as: ActsAs,
    effect: Effect,
    fate: Fate,
    purpose: &'static str,
) -> HostCall {
    HostCall {
        name,
        version: 1,
        command,
        acts_as,
        effect,
        timeout_secs: 10,
        when_down: WhenDown::Fail,
        fate,
        purpose,
    }
}

/// A provider connection token, as the turn's person or the agent.
pub const CONNECTION_TOKEN: &str = "connection_token";
/// The token of an MCP server that acts as the turn's person (`user`, or
/// `user_or_service` falling back to the agent's grant), or of a
/// connection-backed preset; refreshed in the control plane. Declared by
/// the agent's own MCP servers rather than a capability: the cell asks only
/// for a server its definition names this way. Its params name the session
/// and `input_message_id: "open"`, which the session's cell replaces with
/// the input message of the turn it has open ([`OPEN_TURN`]).
pub const MCP_CONNECTION_TOKEN: &str = "mcp_connection_token";
/// An `input_message_id` that asks the session's cell for the turn it has
/// open: an MCP server's credential is asked for by session, and the cell,
/// not the executor, knows whose turn is running.
pub const OPEN_TURN: &str = "open";
/// A GitHub token minted for one use.
pub const GITHUB_TOKEN: &str = "github_token";
/// The org's egress grants.
pub const EGRESS_GRANTS: &str = "egress_grants";
/// The responding agent's service API key.
pub const SERVICE_API_KEY: &str = "service_api_key";
/// The session owner's tool approval settings.
pub const TOOL_APPROVAL_POLICY: &str = "tool_approval_policy";
/// A web search account key.
pub const WEB_SEARCH_KEY: &str = "web_search_key";
/// The org's web search settings.
pub const WEB_SEARCH_SETTINGS: &str = "web_search_settings";
/// The org's knowledge: search and browse.
pub const KNOWLEDGE: &str = "knowledge";
/// A Poppy call.
pub const POPPY_CALL: &str = "poppy_call";

/// The host calls any turn may make, whatever its agent's capabilities:
/// the approval policy every tool call is checked against, the egress
/// grants every outbound request is, and the session's own MCP records
/// (chat-only servers, ARD MCP targets), which join any agent's turns as
/// on v1: asked for resolved ([`USER_MCP_SERVERS`]), and a token per use
/// for one that signs in as the person ([`MCP_CONNECTION_TOKEN`]). Every
/// other call is declared by the capabilities that make it
/// (`Capability::host_calls`).
pub const TURN_CALLS: &[&str] = &[
    TOOL_APPROVAL_POLICY,
    EGRESS_GRANTS,
    USER_MCP_SERVERS,
    MCP_CONNECTION_TOKEN,
];

/// Every host call, by name.
pub const HOST_CALLS: &[HostCall] = &[
    call(
        CONNECTION_TOKEN,
        "worker_get_connection_token",
        ActsAs::Person,
        Effect::Read,
        Fate::HostCall,
        "a provider connection token of the turn's person, or of the agent; refreshed in the control plane",
    ),
    call(
        MCP_CONNECTION_TOKEN,
        "worker_get_mcp_connection_token",
        ActsAs::Person,
        Effect::Read,
        Fate::HostCall,
        "the token of an MCP server acting as the turn's person, or a connection-backed preset's",
    ),
    call(
        GITHUB_TOKEN,
        "worker_get_github_token",
        ActsAs::Person,
        Effect::Read,
        Fate::HostCall,
        "a GitHub token minted for one use from the GitHub App key",
    ),
    call(
        EGRESS_GRANTS,
        "worker_get_org_egress_allowlist",
        ActsAs::Org,
        Effect::Read,
        Fate::Pushed,
        "the org's egress grants (open egress, allowlist extension), until they are pushed",
    ),
    call(
        SERVICE_API_KEY,
        "worker_get_service_api_key_connection",
        ActsAs::Person,
        Effect::Read,
        Fate::HostCall,
        "the responding agent's service API key, for the turn",
    ),
    call(
        TOOL_APPROVAL_POLICY,
        "worker_get_tool_approval_policy",
        ActsAs::Org,
        Effect::Read,
        Fate::Pushed,
        "the session owner's own approval settings; the org's policy is pushed",
    ),
    call(
        WEB_SEARCH_KEY,
        "worker_get_web_search_account_key",
        ActsAs::Org,
        Effect::Read,
        Fate::Pushed,
        "a web search account key, until the org's keys are pushed",
    ),
    call(
        WEB_SEARCH_SETTINGS,
        "worker_get_web_search_settings",
        ActsAs::Org,
        Effect::Read,
        Fate::Pushed,
        "the org's web search settings, until they are pushed",
    ),
    call(
        KNOWLEDGE,
        "worker_knowledge",
        ActsAs::Org,
        Effect::Read,
        Fate::HostCall,
        "the org's knowledge the session's agent attaches: search and browse",
    ),
    call(
        POPPY_CALL,
        "worker_poppy_call",
        ActsAs::Org,
        Effect::Write,
        Fate::HostCall,
        "a Poppy call: the owner's accounts and conversations stay in the control plane",
    ),
    call(
        PLATFORM_COMMAND,
        "platform_command_surface",
        ActsAs::Person,
        Effect::Write,
        Fate::HostCall,
        "a platform tool's call on the command catalog, as the person who sent the turn",
    ),
    call(
        SLACK_ACTION,
        "slack_action",
        ActsAs::ChannelBot,
        Effect::Write,
        Fate::HostCall,
        "a Slack action as the bot of the session's Slack endpoint",
    ),
    call(
        MEMORY_FILE,
        "memory_file",
        ActsAs::Org,
        Effect::Write,
        Fate::HostCall,
        "one file operation in the session's Memory, shared across sessions in the control plane",
    ),
    call(
        USER_MCP_STORE,
        "user_mcp_store",
        ActsAs::Person,
        Effect::Write,
        Fate::HostCall,
        "one call on the turn's person's own MCP server list, which stays in the control plane",
    ),
    call(
        USER_MCP_SERVERS,
        "user_mcp_servers",
        ActsAs::Person,
        Effect::Read,
        Fate::HostCall,
        "the turn's person's own MCP servers, resolved for the turn",
    ),
    call(
        SESSION_RESOURCE_REGISTER,
        "worker_register_session_resource",
        ActsAs::Org,
        Effect::Write,
        Fate::HostCall,
        "register or update one of the session's resources, kept in the control plane",
    ),
    call(
        SESSION_RESOURCE_STATUS,
        "worker_update_session_resource_status",
        ActsAs::Org,
        Effect::Write,
        Fate::HostCall,
        "change the status of one of the session's resources",
    ),
    call(
        SESSION_RESOURCES_LIST,
        "worker_list_session_resources",
        ActsAs::Org,
        Effect::Read,
        Fate::HostCall,
        "the session's resources",
    ),
    call(
        SESSION_RESOURCE_DEREGISTER,
        "worker_deregister_session_resource",
        ActsAs::Org,
        Effect::Write,
        Fate::HostCall,
        "remove one of the session's resources",
    ),
    call(
        LEASED_RESOURCE_UPSERT,
        "worker_upsert_leased_resource",
        ActsAs::Org,
        Effect::Write,
        Fate::HostCall,
        "take or renew the lease of something a tool started at a provider",
    ),
    call(
        LEASED_RESOURCE_RELEASE,
        "worker_release_leased_resource",
        ActsAs::Org,
        Effect::Write,
        Fate::HostCall,
        "release a lease the session holds",
    ),
    call(
        LEASED_RESOURCES_LIST,
        "worker_list_session_leased_resources",
        ActsAs::Org,
        Effect::Read,
        Fate::HostCall,
        "the session's leases",
    ),
    call(
        HELPER_MODEL,
        "worker_resolve_helper_model",
        ActsAs::Org,
        Effect::Read,
        Fate::HostCall,
        "the model a subagent's helper key resolves to: presets and the org's catalog stay in the control plane",
    ),
    call(
        DREAMING_STATE,
        "worker_dreaming_state",
        ActsAs::Org,
        Effect::Write,
        Fate::HostCall,
        "dreaming's maintenance state",
    ),
    call(
        TASK_PUSH_SECRET,
        "task_push_secret",
        ActsAs::Org,
        Effect::Write,
        Fate::HostCall,
        "keep a task push config's signing secret",
    ),
    call(
        SUBAGENT_RECORD,
        "subagent_record",
        ActsAs::Org,
        Effect::Write,
        Fate::HostCall,
        "record a subagent session the data plane started",
    ),
];

/// The host call named `name`.
pub fn by_name(name: &str) -> Option<&'static HostCall> {
    HOST_CALLS.iter().find(|c| c.name == name)
}

/// The host call a v1 worker command maps to.
pub fn by_command(command: &str) -> Option<&'static HostCall> {
    HOST_CALLS.iter().find(|c| c.command == command)
}

/// `POST /dp/v1/tenants/{org}/host-calls/{name}`: one call, answered once.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HostCallRequest {
    /// A fresh id per call: the answer is sealed under
    /// [`host_call_secret`] of it, so it cannot pass for another's.
    pub call: String,
    pub params: Value,
}

/// What a host call answered. Its JSON travels as a sealed secret named
/// [`host_call_secret`] (revision 0) under the org's data key, because
/// most answers are credentials and the cells between pass them on
/// unopened.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HostCallAnswer {
    Value(Value),
    /// The call refused: its kind (the internal protocol's `CommandError`
    /// kind) and a message safe to show.
    Error {
        kind: i32,
        message: String,
    },
}

/// The secret id a host call's answer is sealed under.
pub fn host_call_secret(call: &str) -> String {
    format!("host-call:{call}")
}

/// The registry as text, one call per line: what
/// `host_calls_registry.txt` holds.
pub fn registry_text() -> String {
    let mut out = String::from("name version acts_as effect timeout_s when_down fate command\n");
    for c in HOST_CALLS {
        out.push_str(&format!(
            "{} v{} {:?} {:?} {} {:?} {:?} {}\n",
            c.name, c.version, c.acts_as, c.effect, c.timeout_secs, c.when_down, c.fate, c.command
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A change to the registry must change `host_calls_registry.txt` too,
    /// so every new or changed call shows in review.
    #[test]
    fn the_registry_matches_its_printed_copy() {
        assert_eq!(
            registry_text(),
            include_str!("host_calls_registry.txt"),
            "the host call registry changed: update crates/contracts/src/host_calls_registry.txt"
        );
    }

    #[test]
    fn names_and_commands_are_unique_and_found() {
        for c in HOST_CALLS {
            assert_eq!(by_name(c.name), Some(c));
            assert_eq!(by_command(c.command), Some(c));
        }
        assert!(by_name("worker_get_github_token").is_none());
        assert!(by_command("worker_get_connection_token_for_connection").is_none());
    }

    #[test]
    fn only_reads_retry() {
        assert!(by_name("github_token").unwrap().retries());
        assert!(!by_name(SLACK_ACTION).unwrap().retries());
    }
}

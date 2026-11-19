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
//!   calls are org values known before the call (policy, settings, keys,
//!   egress grants) and move to pushed state; the session's SQL databases
//!   move into the runtime. Until then they travel here, under the same
//!   rules. A connection token or a service key stays a call: which one
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
/// The host call that runs one operation on a session's SQL databases
/// (`op`: `create`, `list`, `get`, `delete`, `execute`, `query`, `schema`
/// with its `name`, `sql` or `table`); the answer is `{"ok": ...}` or
/// `{"error": ...}` (`session_sqldb::SessionSqlDbError`).
pub const SESSION_SQL_DB: &str = "session_sql_db";

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

/// Every host call, by name.
pub const HOST_CALLS: &[HostCall] = &[
    call(
        "connection_token",
        "worker_get_connection_token",
        ActsAs::Person,
        Effect::Read,
        Fate::HostCall,
        "a provider connection token of the turn's person, or of the agent; refreshed in the control plane",
    ),
    call(
        "github_token",
        "worker_get_github_token",
        ActsAs::Person,
        Effect::Read,
        Fate::HostCall,
        "a GitHub token minted for one use from the GitHub App key",
    ),
    call(
        "egress_grants",
        "worker_get_org_egress_allowlist",
        ActsAs::Org,
        Effect::Read,
        Fate::Pushed,
        "the org's egress grants: open egress, allowlist extension",
    ),
    call(
        "service_api_key",
        "worker_get_service_api_key_connection",
        ActsAs::Person,
        Effect::Read,
        Fate::HostCall,
        "the responding agent's service API key, for the turn",
    ),
    call(
        "tool_approval_policy",
        "worker_get_tool_approval_policy",
        ActsAs::Org,
        Effect::Read,
        Fate::Pushed,
        "the org's tool approval policy",
    ),
    call(
        "web_search_key",
        "worker_get_web_search_account_key",
        ActsAs::Org,
        Effect::Read,
        Fate::Pushed,
        "a web search account key",
    ),
    call(
        "web_search_settings",
        "worker_get_web_search_settings",
        ActsAs::Org,
        Effect::Read,
        Fate::Pushed,
        "the org's web search settings",
    ),
    call(
        "knowledge",
        "worker_knowledge",
        ActsAs::Org,
        Effect::Read,
        Fate::HostCall,
        "the org's knowledge the session's agent attaches: search and browse",
    ),
    call(
        "poppy_call",
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
        SESSION_SQL_DB,
        "session_sql_db",
        ActsAs::Org,
        Effect::Write,
        Fate::Runtime,
        "one operation on the session's SQL databases",
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

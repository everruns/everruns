//! Session commands and session routes: the two ways into a session on a
//! data plane from outside it (data plane v2 parity, Phase A).
//!
//! Decisions:
//! - The control plane reaches a v2 session only through a declared session
//!   command or read (`POST` or `GET /dp/v1/orgs/{org}/sessions/{id}/{name}`),
//!   each with what it does, its version and who uses it. The shard serves
//!   only these, and the control plane names them by constant, never by a
//!   string of its own.
//! - Every session route of the public API (`/v1/sessions/{id}/{segment}`)
//!   is declared too, with the plane that serves it ([`Plane`]): the
//!   session's shard, the control plane's record, or a known gap (the shard
//!   answers "not available yet"). The edge routes by this table, and a test
//!   on each side fails on a route nobody declared (the server's API), a
//!   shard route the shard does not serve, or a gap the shard has closed, so a
//!   new route shows up in review instead of silently going to the wrong
//!   plane.
//! - Both lists print to `session_commands_registry.txt`; a test fails
//!   until a change to them changes that file too.

/// What a session command does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `POST /dp/v1/orgs/{org}/sessions/{id}/{name}`: changes the session.
    Command,
    /// `GET /dp/v1/orgs/{org}/sessions/{id}/{name}`: reads it.
    Read,
}

/// One declared session command or read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCommand {
    /// The name on the wire.
    pub name: &'static str,
    pub kind: Kind,
    /// Bumped when the body or the answer change incompatibly.
    pub version: u32,
    /// What the control plane uses it for.
    pub purpose: &'static str,
}

const fn command(name: &'static str, purpose: &'static str) -> SessionCommand {
    SessionCommand {
        name,
        kind: Kind::Command,
        version: 1,
        purpose,
    }
}

const fn read(name: &'static str, purpose: &'static str) -> SessionCommand {
    SessionCommand {
        name,
        kind: Kind::Read,
        version: 1,
        purpose,
    }
}

/// Admit a message the control plane produced or passed on (an agent
/// trigger's run, a channel's message, a message with uploads, a reaction
/// note), once per message id.
pub const MESSAGES: SessionCommand = command(
    "messages",
    "admit a message the control plane produced or passed on",
);
/// Cancel the running turn.
pub const CANCEL_TURN: SessionCommand = command("cancel-turn", "cancel the running turn");
/// A channel's prior conversation, before the first turn.
pub const SEED: SessionCommand = command("seed", "a channel's prior conversation");
/// The result of a parked tool call the control plane resolved (a question
/// set's answers, an MCP sign-in).
pub const TOOL_RESULTS: SessionCommand = command(
    "tool-results",
    "the result of a parked tool call the control plane resolved",
);
/// An event the control plane records on the session (a reaction, feedback).
pub const RECORD: SessionCommand = command("record", "an event the control plane records");
/// Consume a channel approval at the canonical position the control plane checked.
pub const CLAIM_APPROVAL: SessionCommand = command(
    "claim-approval",
    "claim a channel approval once at a checked log position",
);
/// A person's change to the session's storage (values, sealed secrets).
pub const STORAGE: SessionCommand =
    command("storage", "a person's change to the session's storage");
/// The session's files: a message's uploads, copied before it is sent, and
/// every file with its content for a git commit (`snapshot`).
pub const FS: SessionCommand = command(
    "fs",
    "the session's files: a message's uploads, a git commit's snapshot",
);
/// A moved v1 session's SQL database, imported whole (one SQLite file).
pub const SQLDB: SessionCommand =
    command("sqldb", "a moved session's SQL database, imported whole");
/// Open a moved v1 session to messages once its import is done.
pub const ACTIVATE: SessionCommand = command(
    "activate",
    "open a moved session to messages once it is imported",
);

/// Event rows in a sequence range, by type or message.
pub const EVENTS: SessionCommand = read("events", "event rows in a range, by type or message");
/// The committed log in public form, a page after a sequence (an export).
pub const LOG: SessionCommand = read("log", "the committed log in public form (an export)");
/// The session's trace rows.
pub const TRACE: SessionCommand = read("trace", "the session's trace rows");
/// The session's storage values.
pub const STORAGE_READ: SessionCommand = read("storage", "the session's storage values");
/// The session's sealed secrets' names and revisions.
pub const SECRETS: SessionCommand = read("secrets", "the session's sealed secrets, without values");
/// The pinned sandbox's state on the shard.
pub const SANDBOX: SessionCommand = read("sandbox", "the pinned sandbox's state");
/// What the session's model calls, turns and image tools spent, unpriced
/// (`runtime::session_spend::SpendInputs`).
pub const SPEND: SessionCommand = read(
    "spend",
    "the session's model calls, turn outcomes and image results",
);

/// Every session command and read the control plane may use.
pub const SESSION_COMMANDS: &[SessionCommand] = &[
    MESSAGES,
    CANCEL_TURN,
    SEED,
    TOOL_RESULTS,
    RECORD,
    CLAIM_APPROVAL,
    STORAGE,
    FS,
    SQLDB,
    ACTIVATE,
    EVENTS,
    LOG,
    TRACE,
    STORAGE_READ,
    SECRETS,
    SANDBOX,
    SPEND,
];

/// The declared command (`Kind::Command`) or read named `name`.
pub fn find(kind: Kind, name: &str) -> Option<&'static SessionCommand> {
    SESSION_COMMANDS
        .iter()
        .find(|c| c.kind == kind && c.name == name)
}

/// Which plane serves a session route of the public API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    /// The session's shard, through the edge.
    Shard,
    /// The control plane, for both planes: the session's record (owner,
    /// budgets, cost) or a feature that reads the session on its shard
    /// through the session commands above.
    Record,
    /// Not served for a v2 session yet: the shard answers 501 with the
    /// feature's name. Each one is a parity gap to close.
    Gap,
}

/// Every `/v1/sessions/{id}/{segment}` route of the public API, by its
/// first segment, with the plane that serves it for a v2 session.
pub const SESSION_ROUTES: &[(&str, Plane)] = &[
    // The conversation: the shard runs it.
    ("messages", Plane::Shard),
    ("events", Plane::Shard),
    ("sse", Plane::Shard),
    ("cancel", Plane::Shard),
    ("tool-approvals", Plane::Shard),
    ("tool-results", Plane::Shard),
    ("tasks", Plane::Shard),
    ("schedules", Plane::Shard),
    ("work", Plane::Shard),
    ("resources", Plane::Shard),
    // The session's SQL databases (parity step 9).
    ("databases", Plane::Shard),
    // The record the control plane keeps for both planes.
    ("archive", Plane::Record),
    ("pin", Plane::Record),
    ("cost", Plane::Record),
    ("budgets", Plane::Record),
    ("budget-check", Plane::Record),
    ("resolved-model", Plane::Record),
    ("participants", Plane::Record),
    ("commands", Plane::Record),
    ("run-requests", Plane::Record),
    ("agent-view", Plane::Record),
    // Served by the control plane with one code path, reading the
    // session's events from its shard and sending outcomes back as session
    // commands (step 11.6).
    ("trace", Plane::Record),
    ("question-answers", Plane::Record),
    ("reactions", Plane::Record),
    ("feedback", Plane::Record),
    ("context-report", Plane::Record),
    ("export", Plane::Record),
    // The session's storage: read and changed on the shard, with the
    // control plane's policy and secret sealing.
    ("storage", Plane::Record),
    ("approval-mode", Plane::Record),
    // Answered by the control plane on the session's shard: an MCP server's
    // sign-in link, a fork, the whole run, continuing past a usage policy.
    ("mcp-elicitation-consent", Plane::Record),
    ("fork", Plane::Record),
    ("run", Plane::Record),
    ("resume", Plane::Record),
    // The pinned sandbox: its description (also under its older name),
    // and pausing, resuming or deleting it.
    ("environment", Plane::Record),
    ("sandbox", Plane::Record),
    // What a coding session changed in the repositories in its sandbox
    // (the Changes tab): read from the pinned sandbox, as its lifecycle is.
    ("repositories", Plane::Record),
    // Commits of the session's files: the history the control plane keeps
    // for both planes, a commit's files read from the session's shard.
    ("git", Plane::Record),
    // The MCP servers added to one chat: records in the session's storage.
    ("mcp-servers", Plane::Record),
    // A voice call: the control plane runs it, follows the session's
    // events from its shard and records the call's events there.
    ("voice", Plane::Record),
];

/// Paths under a [`Plane::Shard`] route the shard does not serve yet (it
/// answers 501), relative to the session, as the public API spells them.
pub const AGENTD_ROUTE_GAPS: &[&str] = &[];

/// The calls on one of a session's messages (`messages/{id}/{call}`) the
/// control plane serves; every other message route is the shard's.
pub const MESSAGE_RECORD_ROUTES: &[&str] = &["reactions", "feedback"];

/// The plane serving the session route whose first segment is `segment`;
/// an undeclared one is the shard's, which answers what it does not serve
/// with "not available yet".
pub fn plane(segment: &str) -> Plane {
    SESSION_ROUTES
        .iter()
        .find(|(name, _)| *name == segment)
        .map_or(Plane::Shard, |(_, plane)| *plane)
}

/// Both lists as text: what `session_commands_registry.txt` holds.
pub fn registry_text() -> String {
    let mut out = String::from("# session commands: kind name version\n");
    for c in SESSION_COMMANDS {
        out.push_str(&format!("{:?} {} v{}\n", c.kind, c.name, c.version));
    }
    out.push_str("# session routes: segment plane\n");
    for (segment, plane) in SESSION_ROUTES {
        out.push_str(&format!("{segment} {plane:?}\n"));
    }
    for call in MESSAGE_RECORD_ROUTES {
        out.push_str(&format!("messages/{{id}}/{call} Record\n"));
    }
    for path in AGENTD_ROUTE_GAPS {
        out.push_str(&format!("{path} Gap\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_approval_claim_is_an_internal_command() {
        assert!(find(Kind::Command, "claim-approval").is_some());
        assert!(
            SESSION_ROUTES
                .iter()
                .all(|(route, _)| *route != "claim-approval"),
            "only the trusted control plane may claim channel approvals"
        );
    }

    /// A change to either list must change `session_commands_registry.txt`
    /// too, so every new command or route shows in review.
    #[test]
    fn the_registry_matches_its_printed_copy() {
        assert_eq!(
            registry_text(),
            include_str!("session_commands_registry.txt"),
            "the session command registry changed: update crates/contracts/src/session_commands_registry.txt"
        );
    }

    #[test]
    fn names_are_unique_per_kind_and_found() {
        for c in SESSION_COMMANDS {
            assert_eq!(find(c.kind, c.name), Some(c));
        }
        let mut segments: Vec<&str> = SESSION_ROUTES.iter().map(|(s, _)| *s).collect();
        segments.sort_unstable();
        segments.dedup();
        assert_eq!(
            segments.len(),
            SESSION_ROUTES.len(),
            "a route declared twice"
        );
        assert!(find(Kind::Command, "events").is_none());
        assert_eq!(plane("archive"), Plane::Record);
        assert_eq!(plane("messages"), Plane::Shard);
        assert_eq!(plane("git"), Plane::Record);
        assert_eq!(plane("voice"), Plane::Record);
    }
}

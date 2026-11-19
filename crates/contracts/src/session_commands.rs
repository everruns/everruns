//! Session commands and session routes: the two ways into a session on a
//! data plane from outside it (data plane v2 parity, Phase A).
//!
//! Decisions:
//! - Host calls (`host_calls`) are the one way a data plane asks its
//!   control plane anything. This is the same discipline the other way: the
//!   control plane reaches a v2 session only through a declared session
//!   command or read (`POST` or `GET /dp/v1/orgs/{org}/sessions/{id}/{name}`),
//!   each with what it does, its version and who uses it. The cell serves
//!   only these, and the control plane names them by constant, never by a
//!   string of its own.
//! - Every session route of the public API (`/v1/sessions/{id}/{segment}`)
//!   is declared too, with the plane that serves it ([`Plane`]): the
//!   session's cell, the control plane's record, or a known gap (the cell
//!   answers "not available yet"). The edge routes by this table, and a test
//!   on each side fails on a route nobody declared (the server's API), a
//!   cell route the cell does not serve, or a gap the cell has closed, so a
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
/// A person's change to the session's storage (values, sealed secrets).
pub const STORAGE: SessionCommand =
    command("storage", "a person's change to the session's storage");
/// The session's files: a message's uploads, copied before it is sent.
pub const FS: SessionCommand = command("fs", "the session's files: a message's uploads");

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
/// The pinned sandbox's state on the cell.
pub const SANDBOX: SessionCommand = read("sandbox", "the pinned sandbox's state");

/// Every session command and read the control plane may use.
pub const SESSION_COMMANDS: &[SessionCommand] = &[
    MESSAGES,
    CANCEL_TURN,
    SEED,
    TOOL_RESULTS,
    RECORD,
    STORAGE,
    FS,
    EVENTS,
    LOG,
    TRACE,
    STORAGE_READ,
    SECRETS,
    SANDBOX,
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
    /// The session's cell, through the edge.
    Cell,
    /// The control plane, for both planes: the session's record (owner,
    /// budgets, cost) or a feature that reads the session on its cell
    /// through the session commands above.
    Record,
    /// Not served for a v2 session yet: the cell answers 501 with the
    /// feature's name. Each one is a parity gap to close.
    Gap,
}

/// Every `/v1/sessions/{id}/{segment}` route of the public API, by its
/// first segment, with the plane that serves it for a v2 session.
pub const SESSION_ROUTES: &[(&str, Plane)] = &[
    // The conversation: the cell runs it.
    ("messages", Plane::Cell),
    ("events", Plane::Cell),
    ("sse", Plane::Cell),
    ("cancel", Plane::Cell),
    ("tool-approvals", Plane::Cell),
    ("tool-results", Plane::Cell),
    ("tasks", Plane::Cell),
    ("schedules", Plane::Cell),
    ("work", Plane::Cell),
    ("resources", Plane::Cell),
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
    // Served by the control plane with one code path, reading the
    // session's events from its cell and sending outcomes back as session
    // commands (step 11.6).
    ("trace", Plane::Record),
    ("question-answers", Plane::Record),
    ("reactions", Plane::Record),
    ("feedback", Plane::Record),
    ("context-report", Plane::Record),
    ("export", Plane::Record),
    // The session's storage: read and changed on the cell, with the
    // control plane's policy and secret sealing.
    ("storage", Plane::Record),
    ("approval-mode", Plane::Record),
    // Answered by the control plane on the session's cell: an MCP server's
    // sign-in link, a fork, the whole run, continuing past a usage policy.
    ("mcp-elicitation-consent", Plane::Record),
    ("fork", Plane::Record),
    ("run", Plane::Record),
    ("resume", Plane::Record),
    // The pinned sandbox: its description (also under its older name),
    // and pausing, resuming or deleting it.
    ("environment", Plane::Record),
    ("sandbox", Plane::Record),
    // Known gaps (data plane v2 parity, Phase B).
    ("git", Plane::Gap),
    ("mcp-servers", Plane::Gap),
    ("repositories", Plane::Gap),
    ("voice", Plane::Gap),
];

/// Paths under a [`Plane::Cell`] route the cell does not serve yet (it
/// answers 501), relative to the session, as the public API spells them.
pub const CELL_ROUTE_GAPS: &[&str] = &[
    "tasks/{task_id}/push-configs",
    "tasks/{task_id}/push-configs/{config_id}",
];

/// The calls on one of a session's messages (`messages/{id}/{call}`) the
/// control plane serves; every other message route is the cell's.
pub const MESSAGE_RECORD_ROUTES: &[&str] = &["reactions", "feedback"];

/// The plane serving the session route whose first segment is `segment`;
/// an undeclared one is the cell's, which answers what it does not serve
/// with "not available yet".
pub fn plane(segment: &str) -> Plane {
    SESSION_ROUTES
        .iter()
        .find(|(name, _)| *name == segment)
        .map_or(Plane::Cell, |(_, plane)| *plane)
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
    for path in CELL_ROUTE_GAPS {
        out.push_str(&format!("{path} Gap\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(plane("messages"), Plane::Cell);
        assert_eq!(plane("git"), Plane::Gap);
    }
}

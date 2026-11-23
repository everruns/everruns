//! What applying a command answers its proposer (`state.rs`).

use serde::{Deserialize, Serialize};

use super::{Event, Placed, ShardId};

/// The outcome of applying one command, returned to its proposer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Response {
    Ok,
    /// One response per command of a `Batch`.
    Batch {
        responses: Vec<Response>,
    },
    /// The finalized event of an `Append`, with its assigned sequence.
    Appended {
        event: Box<Event>,
    },
    /// The command had already been applied (same message, same session).
    Duplicate,
    NotFound,
    /// The claim is not the session's current one: a stale executor.
    Fenced,
    /// A conditional append found the log ending elsewhere.
    Conflict {
        expected: Option<i32>,
        actual: Option<i32>,
    },
    /// The session is frozen for a move; retry shortly.
    Frozen,
    /// The session moved to another shard.
    Moved {
        shard: ShardId,
    },
    /// The shard is short of memory and takes no new work now; retry after
    /// this long (see `pressure.rs`). Never applied: the leader answers it
    /// before proposing.
    Busy {
        retry_after_ms: u64,
    },
    /// The command does not apply in the session's current state.
    Rejected {
        reason: String,
    },
    /// A directory entry, as `Locate` read it or a `Place` found it.
    Placement {
        current: Option<Placed>,
    },
    /// An authority command's reply, as JSON.
    Value {
        value: serde_json::Value,
    },
}

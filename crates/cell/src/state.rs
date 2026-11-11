//! The cell's replicated state machine.
//!
//! Every replica applies the same committed [`Command`]s in the same order and
//! must reach the same [`CellState`]. So nothing here reads a clock, draws a
//! random number or does I/O: time, ids and finalized event envelopes travel
//! inside the command, chosen once by whoever proposed it.
//!
//! A session's canonical Everruns event log lives here, in memory, beside its
//! inbox and the claim naming the executor allowed to run its current turn.
//! The Raft log in front of it is the write-ahead log; snapshots of this value
//! let the WAL be truncated.

use std::collections::HashMap;

use everruns_contracts::typed_id::{EventId, SessionId};
use everruns_core::events::{Event, EventData, EventRequest};
use openraft::{BasicNode, LogId, StoredMembership};
use serde::{Deserialize, Serialize};

use crate::types::NodeId;

/// Identifies one cell (one Raft group) in the fleet.
pub type CellId = u32;

/// A replicated change to the cell.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "a command lives only between decode and apply; boxing the session command would add an allocation per apply for no saving"
)]
pub enum Command {
    /// Several commands in one Raft entry, applied in order. The leader's
    /// batcher builds these so one quorum write covers many sessions.
    Batch { commands: Vec<Command> },
    /// One epoch's writes in order, applied until the first that fails;
    /// the rest are skipped (`Rejected`). See `hosting::Pipe`.
    Chain { commands: Vec<Command> },
    /// One of Engine's session commands (create, admit, cancel, approve,
    /// resolve, configure, ...), run through its shared reducer at apply.
    /// `definition` rides along with a create: the agent manifest the
    /// session's build key names. `seed` determines the ids of the events
    /// the command appends.
    Host {
        session: SessionId,
        command: everruns::hosting::spi::session_commands::SessionCommand,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        definition: Option<serde_json::Value>,
        now_ms: i64,
        seed: EventId,
    },
    /// Bill `session`'s spend to `tenant`. Set once, right after create,
    /// before the session takes input; a different tenant is refused.
    Assign { session: SessionId, tenant: String },
    /// Take a spend allowance grant for `tenant` (see `allowance`).
    Grant {
        tenant: String,
        grant_id: String,
        grant: crate::allowance::Grant,
    },
    /// The control plane answered allowance request `request` of `tenant`
    /// (see `node_control.rs`): its grant, if any, and the grants that
    /// request settled as closed.
    Allowance {
        tenant: String,
        request: u64,
        #[serde(default)]
        grant: Option<(String, crate::allowance::Grant)>,
        #[serde(default)]
        refused: Option<String>,
        #[serde(default)]
        closed: Vec<String>,
    },
    /// A control plane config update for `org` (directory cell).
    Config {
        org: String,
        update: crate::dp::ConfigUpdate,
        now_ms: i64,
    },
    /// A control plane admin command for `org` (directory cell).
    Admin {
        org: String,
        command: crate::control::AdminCommand,
        now_ms: i64,
    },
    /// The control plane stored the feed through `(index, ordinal)`.
    FeedAcked { through: (u64, u32) },
    /// Reserve `session`'s next provider call; the reply is the Engine's
    /// budget reading.
    Reserve { session: SessionId, now_ms: i64 },
    /// Engine's claim: a new epoch for `holder` on `node`, if the session
    /// owes work and no unexpired epoch holds it.
    Claim {
        session: SessionId,
        holder: String,
        node: NodeId,
        ttl_ms: i64,
        now_ms: i64,
    },
    /// Extend an epoch and record the input it consumed. `receipts` ride
    /// on it (Engine's `commit_with_lease`): committed if absent, only if
    /// the renewal is.
    Renew {
        session: SessionId,
        holder: String,
        fence: u64,
        ttl_ms: i64,
        consumed: i64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        receipts: Vec<(String, serde_json::Value)>,
        now_ms: i64,
    },
    /// A resumed epoch's turn made progress: the poison guard counts from
    /// this epoch again.
    Progress {
        session: SessionId,
        holder: String,
        fence: u64,
        now_ms: i64,
    },
    /// Give an epoch back (`hand_off`: mid-turn, still runnable), with the
    /// receipts riding on it, as on `Renew`.
    Release {
        session: SessionId,
        holder: String,
        fence: u64,
        consumed: i64,
        #[serde(default)]
        hand_off: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        receipts: Vec<(String, serde_json::Value)>,
        now_ms: i64,
    },
    /// Append one canonical event under epoch `fence`. `expected` is the
    /// engine's conditional append (`None`: unconditional).
    Append {
        session: SessionId,
        fence: u64,
        event_id: EventId,
        request: EventRequest,
        expected: Option<Option<i32>>,
        now_ms: i64,
    },
    /// A Framework write under epoch `fence`.
    Write {
        session: SessionId,
        fence: u64,
        change: crate::authority::WriteOp,
        now_ms: i64,
    },
    /// A workspace commit (see `files`): under epoch `fence`, or, without
    /// one, from the gateway or control plane. `op_id` makes a retry return
    /// the first reply.
    Files {
        session: SessionId,
        #[serde(default)]
        fence: Option<u64>,
        op_id: String,
        changes: Vec<crate::files::Change>,
        now_ms: i64,
    },
    /// Directory: `session` lives in `cell` at placement `generation`. Only
    /// the directory cell applies it.
    ///
    /// It commits only over the entry at generation `previous` (0: none),
    /// a compare-and-swap that is a move's ownership transfer: a late or
    /// duplicated placement of an older move can never repoint the session
    /// (`Moves` model in `tests/models/data-plane-v2/`). A failed swap
    /// answers [`Response::Placement`] with the entry as it is. A new
    /// session's first placement (`previous` 0) is refused by a draining
    /// cell.
    Place {
        session: SessionId,
        cell: CellId,
        generation: u64,
        previous: u64,
    },
    /// Directory: read `session`'s entry through the log, so the answer
    /// ([`Response::Placement`]) is current, not a replica's lagging view.
    Locate { session: SessionId },
    /// Record `cell`'s current members in the registry (directory cell).
    /// A list, not a map: integer map keys do not survive serde's
    /// internally tagged enums.
    RegisterCell {
        cell: CellId,
        members: Vec<(NodeId, String)>,
    },
    /// Directory: start (or stop) draining `cell`. New sessions are not
    /// placed in a draining cell, and the rebalancer moves its sessions out.
    DrainCell { cell: CellId, drain: bool },
    /// Directory: drop a drained, empty `cell` from the registry for good.
    /// Its nodes can then be shut down; it can never register again.
    RetireCell { cell: CellId },
    /// Stop starting turns and accepting input, ahead of a move to `to`.
    /// The cell's leader finishes the move once the session is idle, so a
    /// move survives the death of whoever asked for it.
    Freeze {
        session: SessionId,
        #[serde(default)]
        to: Option<CellId>,
    },
    /// A move is about to copy the frozen `session`: end its epoch, so no
    /// write lands in the leaving copy after the copy is taken. Refused
    /// (`Frozen`) while the epoch is leased at `now_ms`; it hands off at
    /// its next step boundary, or expires.
    ///
    /// Decision: the move checks expiry through the log, not on the
    /// driver's clock. An epoch the driver's clock calls expired can look
    /// current to a proposer whose clock is behind (the `Epochs` model's
    /// `Unexpire`), and its late write would land after the copy and be
    /// lost on forward (found by the protocol simulation).
    Revoke { session: SessionId, now_ms: i64 },
    /// Undo a freeze. With `generation`, a move that was abandoned after
    /// it may have installed a copy elsewhere: the directory already took
    /// the session back at that generation (above the abandoned move's), so
    /// that copy can never be activated. Without, a freeze that never
    /// became a move: `to` must be the target the copy is frozen for, so a
    /// late thaw of an earlier freeze can't thaw a copy a later move
    /// already committed elsewhere (found by the protocol simulation).
    Thaw {
        session: SessionId,
        #[serde(default)]
        generation: Option<u64>,
        #[serde(default)]
        to: Option<CellId>,
    },
    /// Install a session moved from another cell, prepared: it runs nothing
    /// until `Activate`. Only a generation newer than any copy this cell
    /// holds or has seen leave is taken.
    Import {
        session: SessionId,
        image: Box<SessionState>,
    },
    /// The directory committed this cell's prepared copy at `generation`:
    /// it may run now.
    Activate { session: SessionId, generation: u64 },
    /// The session now lives in `cell` at `generation`; drop the leaving
    /// copy it was taken from (`generation - 1`) and redirect callers.
    Forward {
        session: SessionId,
        cell: CellId,
        generation: u64,
    },
    /// Drop a prepared copy the directory never committed: it placed the
    /// session in `cell` at `generation` or later.
    Drop {
        session: SessionId,
        cell: CellId,
        generation: u64,
    },
    /// Directory: shard `shard`'s entries now live in cell `to`.
    AssignShard { shard: u16, to: CellId },
    /// Stop taking directory writes for `shard`: it is being handed to
    /// `to`, and a `Place` for it is answered with `Moved { cell: to }`.
    SealShard { shard: u16, to: CellId },
    /// Take directory entries handed over from another cell. Of an entry
    /// this cell already has, the higher generation stays.
    Adopt {
        shard: u16,
        entries: Vec<(SessionId, Placed)>,
    },
    /// Forget a sealed shard's entries once its new owner serves them.
    DropShard { shard: u16 },
    /// The cell's leader stored the session's events `seg` in the object
    /// store. Every replica adds it to the session's segment list.
    Stored {
        session: SessionId,
        seg: crate::store::SegRef,
    },
    /// Piece `seq` of `total` of a command too big for one Raft entry (its
    /// JSON, cut on character boundaries). Replicas collect the pieces; the
    /// last one applies the whole command and answers for it. The proposer
    /// writes them in order, one after another.
    Part {
        id: uuid::Uuid,
        seq: u32,
        total: u32,
        at_ms: i64,
        data: String,
    },
}

/// A command whose pieces are still arriving (see `Command::Part`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Assembly {
    pub at_ms: i64,
    pub total: u32,
    pub next: u32,
    pub data: String,
}

/// Pieces of a command not finished within this long are dropped (their
/// proposer gave up and retried under a new id).
const PART_TTL_MS: i64 = 10 * 60 * 1000;

/// A context compaction checkpoint (Everruns' `CompactionCheckpoint`):
/// replacement model input for the history up to `source_sequence`. The
/// engine reads it back on the next turn, so it is replicated with the
/// session, not kept by the executor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Checkpoint {
    pub id: uuid::Uuid,
    pub source_sequence: i64,
    pub provider_type: String,
    pub model: String,
    pub format_version: u32,
    pub payload: everruns_core::CompactionCheckpointPayload,
}

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
    /// The session moved to another cell.
    Moved {
        cell: CellId,
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

/// Lifecycle of a session within this cell.
///
/// Decision: an imported copy and a departing copy are different states.
/// When they were one ("frozen"), a late duplicate activation could wake a
/// copy that had already started leaving (TLC found it in the `Moves`
/// model). Only a prepared copy can be activated.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    #[default]
    Live,
    /// Leaving for another cell: no new turns or input.
    Frozen,
    /// Imported, waiting for the directory to commit its generation.
    Prepared,
}

/// A directory entry: the session's cell and placement generation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Placed {
    pub cell: CellId,
    pub generation: u64,
}

/// A session that left this cell: where it went, and the highest
/// generation that left (its tombstone: an import at or below it is late).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Left {
    pub cell: CellId,
    pub generation: u64,
}

/// One session: its canonical log, inbox and current turn.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionState {
    /// The agent's name, from its definition (for metrics and listings).
    pub agent: String,
    /// The agent this session runs: an Everruns agent package manifest,
    /// exactly as the client sent it. Every turn rebuilds the agent from it,
    /// so the runtime knows no agents of its own.
    ///
    /// Sessions of the same agent share one copy in memory (see
    /// [`definitions`]), and a snapshot stores each definition once.
    #[serde(default, with = "definitions")]
    pub definition: std::sync::Arc<serde_json::Value>,
    /// In a snapshot: the key of `definition` in [`CellState::definitions`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_ref: Option<String>,
    pub created_ms: i64,
    /// Events `1..=offloaded` are only in the object store; `events` holds
    /// the rest, in order. A replica unloads an idle session by flushing its
    /// events to segments and raising this; it never changes the history.
    #[serde(default)]
    pub offloaded: i32,
    /// The session's segments in the object store, contiguous from
    /// sequence 1. Replicated: only the leader writes segments, and records
    /// each with a `Stored` command, so the cell keeps one copy.
    #[serde(default)]
    pub segments: Vec<crate::store::SegRef>,
    pub events: Vec<Event>,
    /// What Engine's hosting contract asks the cell to own for the session:
    /// binding, admission, epoch, receipts, values (see `authority`).
    #[serde(default)]
    pub authority: crate::authority::Authority,
    pub placement: Placement,
    /// The tenant this session's spend is billed to (`None`: unmetered).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    /// Placement generation of this copy. It only grows: every move takes
    /// the next one, an abandoned move the one after.
    #[serde(default)]
    pub generation: u64,
    /// The cell this session is being moved to, while frozen for a move.
    #[serde(default)]
    pub moving_to: Option<CellId>,
    /// Sequence of the latest `turn.started` event (0: none yet). A turn
    /// resumes from the log after it, so an executor reads every event from
    /// here on and only the messages before (see `Shared::context`).
    #[serde(default)]
    pub turn_start: i32,
    /// The latest compaction checkpoint per provider, model and format.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkpoints: Vec<Checkpoint>,
    /// The session's workspace (see `files`).
    #[serde(default, skip_serializing_if = "crate::files::Files::is_empty")]
    pub files: crate::files::Files,
}

impl SessionState {
    pub fn last_sequence(&self) -> Option<i32> {
        self.events
            .last()
            .and_then(|event| event.sequence)
            .or((self.offloaded > 0).then_some(self.offloaded))
    }

    /// The last sequence this replica's segments cover.
    pub fn flushed(&self) -> i32 {
        self.segments.last().map(|s| s.to).unwrap_or(0)
    }

    /// Note that `seg` is stored, keeping `segments` contiguous from
    /// sequence 1. A segment that starts where a recorded one does and
    /// reaches the end replaces that one and those after it (compaction);
    /// one that does not continue the list is ignored.
    pub fn record(&mut self, seg: crate::store::SegRef) {
        let flushed = self.flushed();
        let first = self.segments.iter().position(|s| s.from == seg.from);
        let last = self.segments.iter().position(|s| s.to == seg.to);
        match (first, last) {
            // A compaction: it replaces exactly the run it covers.
            (Some(first), Some(last)) if first < last => {
                self.segments.splice(first..=last, [seg]);
            }
            _ if seg.from == flushed + 1 => self.segments.push(seg),
            _ => {}
        }
    }

    /// The run of segments a compaction rewrites into one: the trailing run
    /// whose older end is at most twice the size of what follows it, or
    /// none while that run is the newest segment alone.
    ///
    /// Decision: tiered, never from sequence 1 (that made every compaction
    /// cost the whole history), and never a forced pair. Forcing at least
    /// two segments merged the newest one, once large, with each later
    /// flush: an O(n) rewrite per flush (2400 times the events written in a
    /// simulation with flushes of 11 to 77 events). Waiting instead until
    /// the small segments after it add up to half its size keeps sizes
    /// geometric: an event is rewritten about log(n) times, and a session
    /// holds about COMPACT_AT segments. The run covers stored segments only;
    /// the unstored tail is a flush's.
    pub fn compaction_range(&self) -> Option<(i32, i32)> {
        let segments = &self.segments;
        let len = |s: &crate::store::SegRef| i64::from(s.to - s.from + 1);
        let last = segments.len().checked_sub(1)?;
        let mut at = last;
        let mut size = len(&segments[at]);
        // The first segment (the session's first flush, with
        // `session.started`) is never merged: reads near the session's start
        // then parse a few events, not half its history.
        let floor = usize::from(segments.first().is_some_and(|s| s.from == 1));
        while at > floor && len(&segments[at - 1]) <= 2 * size {
            at -= 1;
            size += len(&segments[at]);
        }
        (at < last && at >= floor).then(|| (segments[at].from, segments[last].to))
    }

    /// Where the current (or last) turn's events begin: its input or its
    /// `turn.started`, whichever is earlier. A turn resumes from the events
    /// after it; before it, only messages matter.
    pub fn turn_from(&self) -> i32 {
        let mut from = self.last_sequence().unwrap_or(0) + 1;
        if self.turn_start > 0 {
            from = from.min(self.turn_start);
        }
        // Admitted input no turn has consumed yet is read whole too.
        if let Some(first) = self
            .authority
            .pending_inputs
            .iter()
            .find(|seq| **seq > self.authority.consumed)
        {
            from = from.min(*first as i32);
        }
        from
    }

    /// Where an executor's copy of the log begins: the current turn's
    /// events, and the messages after the newest compaction checkpoint
    /// (the engine reads no further back while it can restore one). Older
    /// messages are read on demand.
    pub fn context_from(&self) -> i32 {
        let floor = self.checkpoints.iter().map(|c| c.source_sequence).max();
        match floor {
            Some(floor) => self.turn_from().min(floor.saturating_add(1) as i32),
            None => 1,
        }
    }

    /// Resident events with a sequence above `after`.
    pub fn resident_after(&self, after: i32) -> &[Event] {
        let skip = (after - self.offloaded).max(0) as usize;
        &self.events[skip.min(self.events.len())..]
    }

    /// An epoch holds the session (a turn runs, or its holder is presumed
    /// to until superseded).
    pub fn turn_running(&self) -> bool {
        self.authority.holder.is_some()
    }

    /// Admitted input no turn has consumed yet.
    pub fn queued(&self) -> usize {
        let consumed = self.authority.consumed;
        self.authority
            .pending_inputs
            .iter()
            .filter(|seq| **seq > consumed)
            .count()
    }

    /// Turns with a Framework outcome.
    pub fn turns_completed(&self) -> u64 {
        self.authority.outcomes
    }

    /// No turn runs and none waits: safe to unload.
    pub fn idle(&self) -> bool {
        self.authority.holder.is_none()
            && !self.authority.runnable
            && self.authority.open.is_none()
            && self.placement == Placement::Live
    }
}

/// Everything a cell replicates.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CellState {
    pub last_applied: Option<LogId<NodeId>>,
    pub membership: StoredMembership<NodeId, BasicNode>,
    pub sessions: HashMap<SessionId, SessionState>,
    /// Sessions this cell handed to another cell, or dropped a stale copy
    /// of. A copy that comes back removes its entry: that copy's
    /// generation is above it.
    pub forwarded: HashMap<SessionId, Left>,
    /// Directory entries; populated only in the directory cell.
    pub directory: HashMap<SessionId, Placed>,
    /// Cell registry: each cell's members as its leader last reported them.
    /// Populated only in the directory cell; gateways read the fleet's
    /// topology from it, so a cell joins by registering.
    #[serde(default)]
    pub cells: std::collections::BTreeMap<CellId, std::collections::BTreeMap<NodeId, String>>,
    /// Cells being emptied ahead of retirement (directory cell only).
    #[serde(default)]
    pub draining: std::collections::BTreeSet<CellId>,
    /// Cells taken out of the fleet (directory cell only). Kept so that a
    /// retired cell's nodes, still running, cannot register it again.
    #[serde(default)]
    pub retired: std::collections::BTreeSet<CellId>,
    /// Which cell holds each directory shard's entries (directory cell
    /// only). A shard not listed lives in the directory cell.
    #[serde(default)]
    pub shards: std::collections::BTreeMap<u16, CellId>,
    /// Shards this cell handed to another cell, and to which.
    #[serde(default)]
    pub sealed: std::collections::BTreeMap<u16, CellId>,
    /// In a snapshot only: each agent definition once, by key; sessions
    /// point at theirs with `definition_ref`. Empty in memory.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub definitions: std::collections::BTreeMap<String, serde_json::Value>,
    /// Spend allowances, per tenant.
    #[serde(default)]
    pub allowances: crate::allowance::Allowances,
    /// Each org's control plane config (directory cell only).
    #[serde(default)]
    pub orgs: crate::control::Orgs,
    /// Feed records the control plane has not acknowledged yet.
    #[serde(default)]
    pub feed: crate::control::Feed,
    /// Commands arriving in pieces, by id.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub parts: std::collections::BTreeMap<uuid::Uuid, Assembly>,
}

impl CellState {
    /// Before serializing a snapshot: keep each definition once.
    pub fn share_definitions(&mut self) {
        for s in self.sessions.values_mut() {
            if s.definition.is_null() {
                continue;
            }
            let key = definitions::key(&s.definition);
            self.definitions
                .entry(key.clone())
                .or_insert_with(|| (*s.definition).clone());
            s.definition = Default::default();
            s.definition_ref = Some(key);
        }
    }

    /// After reading a snapshot: give sessions their definitions back.
    pub fn resolve_definitions(&mut self) {
        let table = std::mem::take(&mut self.definitions);
        let shared: HashMap<String, std::sync::Arc<serde_json::Value>> = table
            .into_iter()
            .map(|(key, value)| (key, definitions::intern(value)))
            .collect();
        for s in self.sessions.values_mut() {
            if let Some(key) = s.definition_ref.take()
                && let Some(definition) = shared.get(&key)
            {
                s.definition = definition.clone();
            }
        }
    }
}

/// Directory shards. A session's directory entry lives in the cell that
/// holds its shard.
///
/// Decision: one directory cell holding every session's entry would cap the
/// fleet at what one Raft group can store and write. The entries are split
/// into a fixed number of shards by session id, and the directory cell
/// keeps only which cell holds each shard (a small table gateways cache).
/// Shards move between cells as cells join and leave (see
/// `Node::rebalance_shards`).
pub const SHARDS: u16 = 256;

/// The directory shard of `session`: FNV-1a of its id.
pub fn shard_of(session: SessionId) -> u16 {
    let hash = session
        .to_string()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
    (hash % u64::from(SHARDS)) as u16
}

/// Agent definitions shared between sessions.
///
/// Decision: every session carries its agent's definition, and most
/// sessions run one of a few agents. Identical definitions share one
/// allocation (interned by their JSON), so a cell holds each agent once in
/// memory and once per snapshot, however many sessions run it.
pub mod definitions {
    use std::collections::HashMap;
    use std::sync::{Arc, Weak};

    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::Value;

    static TABLE: parking_lot::Mutex<Option<HashMap<String, Weak<Value>>>> =
        parking_lot::Mutex::new(None);

    /// A stable key for `definition`: FNV-1a of its JSON (maps serialize
    /// in key order, so equal definitions have equal JSON).
    pub fn key(definition: &Value) -> String {
        let json = serde_json::to_vec(definition).unwrap_or_default();
        let hash = json.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        });
        format!("{hash:016x}-{}", json.len())
    }

    /// The definition interned under `key`, while any session holds it.
    pub fn lookup(key: &str) -> Option<Arc<Value>> {
        TABLE.lock().as_ref()?.get(key).and_then(Weak::upgrade)
    }

    /// The shared copy of `definition`.
    pub fn intern(definition: Value) -> Arc<Value> {
        if definition.is_null() {
            return Arc::new(Value::Null);
        }
        let key = key(&definition);
        let mut guard = TABLE.lock();
        let table = guard.get_or_insert_with(HashMap::new);
        if let Some(shared) = table.get(&key).and_then(Weak::upgrade) {
            return shared;
        }
        if table.len() > 1024 {
            table.retain(|_, weak| weak.strong_count() > 0);
        }
        let shared = Arc::new(definition);
        table.insert(key, Arc::downgrade(&shared));
        shared
    }

    pub fn serialize<S: Serializer>(value: &Arc<Value>, s: S) -> Result<S::Ok, S::Error> {
        value.as_ref().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Value>, D::Error> {
        Ok(intern(Value::deserialize(d)?))
    }
}

/// What an applied command changed, for local observers.
#[derive(Debug, Clone)]
pub enum Change {
    /// A durable event was appended to `session`.
    Event(SessionId, std::sync::Arc<Event>),
    /// The session's inbox, claim or placement changed.
    Session(SessionId),
}

impl Change {
    pub fn session(&self) -> SessionId {
        match self {
            Change::Event(session, _) | Change::Session(session) => *session,
        }
    }
}

impl CellState {
    /// Apply one committed command. Deterministic: same state and command,
    /// same result, on every replica.
    pub fn apply(&mut self, command: Command, changes: &mut Vec<Change>) -> Response {
        match command {
            Command::AssignShard { shard, to } => {
                if self.shards.insert(shard, to) == Some(to) {
                    return Response::Duplicate;
                }
                Response::Ok
            }
            Command::SealShard { shard, to } => {
                self.sealed.insert(shard, to);
                Response::Ok
            }
            Command::Adopt { shard, entries } => {
                // The shard is back here (or here for the first time).
                self.sealed.remove(&shard);
                for (session, placed) in entries {
                    let entry = self.directory.entry(session).or_insert(placed);
                    if placed.generation > entry.generation {
                        *entry = placed;
                    }
                }
                Response::Ok
            }
            Command::DropShard { shard } => {
                if self.sealed.contains_key(&shard) {
                    self.directory
                        .retain(|session, _| shard_of(*session) != shard);
                }
                Response::Ok
            }
            Command::Stored { session, seg } => {
                if let Some(s) = self.sessions.get_mut(&session) {
                    s.record(seg);
                }
                Response::Ok
            }
            Command::Part {
                id,
                seq,
                total,
                at_ms,
                data,
            } => {
                if seq == 0 {
                    // Times come from the proposer, so every replica drops
                    // the same abandoned assemblies.
                    self.parts.retain(|_, a| a.at_ms + PART_TTL_MS > at_ms);
                    self.parts.insert(
                        id,
                        Assembly {
                            at_ms,
                            total,
                            ..Assembly::default()
                        },
                    );
                }
                let Some(assembly) = self.parts.get_mut(&id) else {
                    return Response::Rejected {
                        reason: format!("no command {id} being assembled"),
                    };
                };
                if assembly.next != seq || assembly.total != total {
                    self.parts.remove(&id);
                    return Response::Rejected {
                        reason: format!("piece {seq} of {id} out of order"),
                    };
                }
                assembly.data.push_str(&data);
                assembly.next += 1;
                if assembly.next < total {
                    return Response::Ok;
                }
                let Some(assembly) = self.parts.remove(&id) else {
                    return Response::Rejected {
                        reason: format!("assembly {id} vanished"),
                    };
                };
                match serde_json::from_str::<Command>(&assembly.data) {
                    Ok(command) => self.apply(command, changes),
                    Err(error) => Response::Rejected {
                        reason: format!("assembled command {id}: {error}"),
                    },
                }
            }
            Command::Chain { commands } => {
                let mut failed = false;
                let responses = commands
                    .into_iter()
                    .map(|command| {
                        if failed {
                            return Response::Rejected {
                                reason: "skipped: an earlier write of the epoch failed".into(),
                            };
                        }
                        let response = self.apply(command, changes);
                        failed = !matches!(response, Response::Value { .. } | Response::Ok);
                        response
                    })
                    .collect();
                Response::Batch { responses }
            }
            Command::Batch { commands } => Response::Batch {
                responses: commands
                    .into_iter()
                    .map(|command| self.apply(command, changes))
                    .collect(),
            },
            Command::Host {
                session,
                command,
                definition,
                now_ms,
                seed,
            } => {
                use everruns::hosting::spi::session_commands::SessionCommand;
                let create = matches!(command, SessionCommand::Create { .. });
                if let Some(left) = self.forwarded.get(&session) {
                    return Response::Moved { cell: left.cell };
                }
                let fresh = create && !self.sessions.contains_key(&session);
                if fresh {
                    let definition = definition.unwrap_or_default();
                    self.sessions.insert(
                        session,
                        SessionState {
                            agent: definition["name"].as_str().unwrap_or_default().to_string(),
                            definition: definitions::intern(definition),
                            created_ms: now_ms,
                            generation: 1,
                            ..SessionState::default()
                        },
                    );
                }
                let Some(state) = self.session_mut(session) else {
                    return self.missing(session);
                };
                if state.placement != Placement::Live && !create {
                    return Response::Frozen;
                }
                match state.command(session, command, now_ms, seed) {
                    Ok((value, events)) => {
                        for event in events {
                            changes.push(Change::Event(session, std::sync::Arc::new(event)));
                        }
                        changes.push(Change::Session(session));
                        Response::Value { value }
                    }
                    Err(reason) => {
                        if fresh {
                            self.sessions.remove(&session);
                        }
                        Response::Rejected { reason }
                    }
                }
            }
            Command::Claim {
                session,
                holder,
                node,
                ttl_ms,
                now_ms,
            } => {
                let Some(state) = self.session_mut(session) else {
                    return self.missing(session);
                };
                if state.placement != Placement::Live {
                    return Response::Frozen;
                }
                let value = state.claim(&holder, node, ttl_ms, now_ms);
                changes.push(Change::Session(session));
                if value.is_some() {
                    let tenant = state.tenant.clone();
                    self.allowances.close_dangling(tenant.as_deref(), session);
                }
                Response::Value {
                    value: value.unwrap_or_default(),
                }
            }
            Command::Assign { session, tenant } => {
                let Some(state) = self.session_mut(session) else {
                    return self.missing(session);
                };
                match &state.tenant {
                    Some(current) if *current == tenant => Response::Duplicate,
                    Some(current) => Response::Rejected {
                        reason: format!("session is billed to {current}"),
                    },
                    None => {
                        state.tenant = Some(tenant);
                        changes.push(Change::Session(session));
                        Response::Ok
                    }
                }
            }
            Command::Grant {
                tenant,
                grant_id,
                grant,
            } => {
                if self.allowances.grant(&tenant, &grant_id, grant) {
                    Response::Ok
                } else {
                    Response::Duplicate
                }
            }
            Command::Allowance {
                tenant,
                request,
                grant,
                refused,
                closed,
            } => {
                let allowance = self.allowances.tenants.entry(tenant).or_default();
                if allowance.answered(request, grant, refused, &closed) {
                    Response::Ok
                } else {
                    Response::Duplicate
                }
            }
            Command::Config {
                org,
                update,
                now_ms,
            } => {
                let ack = self.orgs.apply(&org, &update);
                if let crate::dp::ConfigAck::Ok { applied_version } = ack {
                    let index = self.last_applied.map_or(0, |l| l.index);
                    self.feed.push(
                        index,
                        &org,
                        now_ms,
                        crate::dp::Record::ConfigApplied {
                            version: applied_version,
                        },
                    );
                }
                Response::Value {
                    value: serde_json::to_value(ack).unwrap_or_default(),
                }
            }
            Command::Admin {
                org,
                command,
                now_ms,
            } => {
                let suspends = matches!(
                    &command,
                    crate::control::AdminCommand::State(c)
                        if c.state == crate::dp::OrgState::Suspended
                );
                let fresh = self
                    .orgs
                    .orgs
                    .get(&org)
                    .is_none_or(|c| c.command(command.command_id()).is_none());
                let ack = self.orgs.admin(&org, command);
                if suspends && fresh && ack.accepted {
                    let index = self.last_applied.map_or(0, |l| l.index);
                    self.feed.push(
                        index,
                        &org,
                        now_ms,
                        crate::dp::Record::OrgEvent {
                            kind: "suspended".into(),
                            detail: String::new(),
                        },
                    );
                }
                Response::Value {
                    value: serde_json::to_value(ack).unwrap_or_default(),
                }
            }
            Command::FeedAcked { through } => {
                self.feed.ack(through);
                Response::Ok
            }
            Command::Reserve { session, now_ms } => {
                let Some(state) = self.sessions.get(&session) else {
                    return self.missing(session);
                };
                // A prepared copy runs nothing. A frozen one may: its epoch
                // runs on to the next step boundary, and that step's call
                // is reserved here, where it settles.
                if state.placement == Placement::Prepared {
                    return Response::Frozen;
                }
                let tenant = state.tenant.clone();
                Response::Value {
                    value: self.allowances.reserve(tenant.as_deref(), session, now_ms),
                }
            }
            Command::Renew {
                session,
                holder,
                fence,
                ttl_ms,
                consumed,
                receipts,
                now_ms,
            } => {
                let index = self.last_applied.map_or(0, |l| l.index);
                let riding = !receipts.is_empty();
                let Some(state) = self.sessions.get_mut(&session) else {
                    return self.missing(session);
                };
                let before = state.turns_completed();
                let renewed = state.renew(&holder, fence, ttl_ms, consumed, receipts, now_ms);
                if state.turns_completed() > before {
                    self.feed.turn_finished(index, &session.to_string(), state);
                }
                if riding && renewed.is_some() {
                    changes.push(Change::Session(session));
                }
                match renewed {
                    Some(value) => Response::Value { value },
                    None => Response::Fenced,
                }
            }
            Command::Progress {
                session,
                holder,
                fence,
                now_ms,
            } => {
                let Some(state) = self.session_mut(session) else {
                    return self.missing(session);
                };
                if state.progress(&holder, fence, now_ms) {
                    Response::Ok
                } else {
                    Response::Fenced
                }
            }
            Command::Release {
                session,
                holder,
                fence,
                consumed,
                hand_off,
                receipts,
                now_ms,
            } => {
                let index = self.last_applied.map_or(0, |l| l.index);
                let Some(state) = self.sessions.get_mut(&session) else {
                    return self.missing(session);
                };
                let before = state.turns_completed();
                let released = state.release(&holder, fence, consumed, hand_off, receipts, now_ms);
                if state.turns_completed() > before {
                    self.feed.turn_finished(index, &session.to_string(), state);
                }
                changes.push(Change::Session(session));
                match released {
                    Some(runnable) => Response::Value {
                        value: serde_json::json!(runnable),
                    },
                    None => Response::Fenced,
                }
            }
            Command::Append {
                session,
                fence,
                event_id,
                request,
                expected,
                now_ms,
            } => {
                let Some(state) = self.session_mut(session) else {
                    return self.missing(session);
                };
                let applied = state.append(fence, event_id, request, expected, now_ms);
                self.applied(session, applied, changes)
            }
            Command::Write {
                session,
                fence,
                change,
                now_ms,
            } => {
                let index = self.last_applied.map_or(0, |l| l.index);
                let Some(state) = self.sessions.get_mut(&session) else {
                    return self.missing(session);
                };
                let before = state.turns_completed();
                let applied = state.write(fence, change, now_ms);
                if state.turns_completed() > before {
                    self.feed.turn_finished(index, &session.to_string(), state);
                }
                changes.push(Change::Session(session));
                self.applied(session, applied, changes)
            }
            Command::Files {
                session,
                fence,
                op_id: op,
                changes: commit,
                now_ms,
            } => {
                let Some(state) = self.session_mut(session) else {
                    return self.missing(session);
                };
                if let Some(reply) = state.files.replay(&op) {
                    return Response::Value { value: reply };
                }
                match fence {
                    Some(fence) if !state.authority.current(fence, now_ms) => {
                        return Response::Fenced;
                    }
                    None if state.placement != Placement::Live => return Response::Frozen,
                    _ => {}
                }
                let value = state.files.commit(op, commit, now_ms);
                changes.push(Change::Session(session));
                Response::Value { value }
            }
            Command::Locate { session } => {
                if let Some(to) = self.sealed.get(&shard_of(session)) {
                    return Response::Moved { cell: *to };
                }
                Response::Placement {
                    current: self.directory.get(&session).copied(),
                }
            }
            Command::Place {
                session,
                cell,
                generation,
                previous,
            } => {
                if let Some(to) = self.sealed.get(&shard_of(session)) {
                    return Response::Moved { cell: *to };
                }
                let placed = Placed { cell, generation };
                let current = self.directory.get(&session).copied();
                if current == Some(placed) {
                    return Response::Duplicate;
                }
                if generation <= previous || current.map_or(0, |p| p.generation) != previous {
                    return Response::Placement { current };
                }
                // A gateway placing from a view a second old must not put a
                // new session in a cell that is being emptied.
                if previous == 0 && (self.draining.contains(&cell) || self.retired.contains(&cell))
                {
                    return Response::Rejected {
                        reason: format!("cell {cell} is draining"),
                    };
                }
                self.directory.insert(session, placed);
                Response::Ok
            }
            Command::RegisterCell { cell, members } => {
                let members: std::collections::BTreeMap<NodeId, String> =
                    members.into_iter().collect();
                if self.retired.contains(&cell) {
                    return Response::Rejected {
                        reason: format!("cell {cell} is retired"),
                    };
                }
                if self.cells.get(&cell) == Some(&members) {
                    return Response::Duplicate;
                }
                self.cells.insert(cell, members);
                Response::Ok
            }
            Command::DrainCell { cell, drain } => {
                if self.retired.contains(&cell) || !self.cells.contains_key(&cell) {
                    return Response::NotFound;
                }
                let changed = if drain {
                    self.draining.insert(cell)
                } else {
                    self.draining.remove(&cell)
                };
                if changed {
                    Response::Ok
                } else {
                    Response::Duplicate
                }
            }
            Command::RetireCell { cell } => {
                if self.retired.contains(&cell) {
                    return Response::Duplicate;
                }
                if !self.draining.contains(&cell) {
                    return Response::Rejected {
                        reason: format!("cell {cell} is not draining"),
                    };
                }
                // Decision: the directory does not know a cell's sessions,
                // only the cell does; whoever retires it checks that it is
                // empty first (the gateway does). A placement still naming
                // it is dropped here so lookups fail fast, not into a void.
                self.directory.retain(|_, p| p.cell != cell);
                self.draining.remove(&cell);
                self.cells.remove(&cell);
                self.retired.insert(cell);
                Response::Ok
            }
            Command::Freeze { session, to } => {
                let Some(state) = self.session_mut(session) else {
                    return self.missing(session);
                };
                // Only the copy the directory committed moves on.
                if state.placement == Placement::Prepared {
                    return Response::Frozen;
                }
                // A move already running is never retargeted: it may have
                // committed in the directory already, and a late freeze
                // back to this cell would thaw the leaving copy while the
                // target runs the session (two owners; found by the
                // protocol simulation). It finishes, or is abandoned
                // through the directory.
                if state.placement == Placement::Frozen {
                    return if state.moving_to == to {
                        Response::Duplicate
                    } else {
                        Response::Frozen
                    };
                }
                state.placement = Placement::Frozen;
                state.moving_to = to;
                changes.push(Change::Session(session));
                Response::Ok
            }
            Command::Revoke { session, now_ms } => {
                let Some(state) = self.session_mut(session) else {
                    return self.missing(session);
                };
                if state.placement != Placement::Frozen || state.authority.executing(now_ms) {
                    return Response::Frozen;
                }
                if state.authority.holder.take().is_some() {
                    changes.push(Change::Session(session));
                }
                Response::Ok
            }
            Command::Thaw {
                session,
                generation,
                to,
            } => {
                let Some(state) = self.session_mut(session) else {
                    return self.missing(session);
                };
                if state.placement != Placement::Frozen {
                    return Response::Duplicate;
                }
                if generation.is_none() && state.moving_to != to {
                    return Response::Rejected {
                        reason: "thaw of another freeze".into(),
                    };
                }
                if let Some(generation) = generation {
                    if generation <= state.generation {
                        return Response::Rejected {
                            reason: format!("thaw at stale generation {generation}"),
                        };
                    }
                    state.generation = generation;
                }
                state.placement = Placement::Live;
                state.moving_to = None;
                changes.push(Change::Session(session));
                Response::Ok
            }
            Command::Import { session, image } => {
                let generation = image.generation;
                let held = self.sessions.get(&session).map(|s| s.generation);
                if held == Some(generation) {
                    return Response::Duplicate;
                }
                let left = self.forwarded.get(&session).map_or(0, |l| l.generation);
                if generation <= held.unwrap_or(0).max(left) {
                    return Response::Rejected {
                        reason: format!("import at stale generation {generation}"),
                    };
                }
                self.forwarded.remove(&session);
                let mut image = *image;
                image.placement = Placement::Prepared;
                image.moving_to = None;
                self.sessions.insert(session, image);
                changes.push(Change::Session(session));
                Response::Ok
            }
            Command::Activate {
                session,
                generation,
            } => {
                let Some(state) = self.session_mut(session) else {
                    return self.missing(session);
                };
                match (state.placement, state.generation == generation) {
                    (Placement::Prepared, true) => {
                        state.placement = Placement::Live;
                        changes.push(Change::Session(session));
                        Response::Ok
                    }
                    // Already activated, and possibly leaving again since.
                    (_, true) => Response::Duplicate,
                    _ => Response::Rejected {
                        reason: format!("no prepared copy at generation {generation}"),
                    },
                }
            }
            Command::Forward {
                session,
                cell,
                generation,
            } => {
                match self.sessions.get(&session) {
                    Some(s)
                        if s.placement == Placement::Frozen && s.generation + 1 == generation => {}
                    Some(_) => {
                        return Response::Rejected {
                            reason: format!("no leaving copy for generation {generation}"),
                        };
                    }
                    None => {
                        return match self.forwarded.get(&session) {
                            Some(left) if left.generation + 1 >= generation => Response::Duplicate,
                            _ => Response::NotFound,
                        };
                    }
                }
                // The move waited for the epoch to hand off or expire; a
                // reservation still open is a call that died mid-way.
                if let Some(left) = self.sessions.remove(&session) {
                    self.allowances
                        .close_dangling(left.tenant.as_deref(), session);
                }
                self.forwarded.insert(
                    session,
                    Left {
                        cell,
                        generation: generation - 1,
                    },
                );
                changes.push(Change::Session(session));
                Response::Ok
            }
            Command::Drop {
                session,
                cell,
                generation,
            } => {
                match self.sessions.get(&session) {
                    Some(s) if s.placement == Placement::Prepared && s.generation <= generation => {
                    }
                    Some(_) => {
                        return Response::Rejected {
                            reason: "not a stale prepared copy".into(),
                        };
                    }
                    None => {
                        return match self.forwarded.contains_key(&session) {
                            true => Response::Duplicate,
                            false => Response::NotFound,
                        };
                    }
                }
                if let Some(s) = self.sessions.remove(&session) {
                    self.forwarded.insert(
                        session,
                        Left {
                            cell,
                            generation: s.generation,
                        },
                    );
                }
                changes.push(Change::Session(session));
                Response::Ok
            }
        }
    }

    fn session_mut(&mut self, session: SessionId) -> Option<&mut SessionState> {
        self.sessions.get_mut(&session)
    }

    fn applied(
        &mut self,
        session: SessionId,
        applied: crate::authority::Applied,
        changes: &mut Vec<Change>,
    ) -> Response {
        use crate::authority::Applied;
        match applied {
            Applied::Done(value, events) => {
                let index = self.last_applied.map_or(0, |l| l.index);
                for event in &events {
                    if let Some(micros) = generation_cost_micros(event) {
                        let tenant = self.sessions.get(&session).and_then(|s| s.tenant.clone());
                        self.allowances.settle(tenant.as_deref(), session, micros);
                    }
                    if let Some(s) = self.sessions.get(&session) {
                        self.feed.observe(index, &session.to_string(), s, event);
                    }
                }
                for event in events {
                    changes.push(Change::Event(session, std::sync::Arc::new(event)));
                }
                Response::Value { value }
            }
            Applied::Fenced => Response::Fenced,
            Applied::Refused(reason) => Response::Rejected { reason },
            Applied::Conflict { expected, actual } => Response::Conflict { expected, actual },
        }
    }

    fn missing(&self, session: SessionId) -> Response {
        match self.forwarded.get(&session) {
            Some(left) => Response::Moved { cell: left.cell },
            None => Response::NotFound,
        }
    }
}

/// What a provider call cost, from its `llm.generation` event, in micro
/// dollars (0 when the provider reported no cost).
fn generation_cost_micros(event: &Event) -> Option<i64> {
    let EventData::LlmGeneration(data) = &event.data else {
        return None;
    };
    let usd = data
        .metadata
        .usage
        .as_ref()
        .and_then(|u| u.effective_cost_usd())
        .unwrap_or(0.0);
    Some((usd * 1_000_000.0).ceil() as i64)
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "state_moves_tests.rs"]
mod moves_tests;

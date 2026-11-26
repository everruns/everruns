//! The shard's replicated state machine.
//!
//! Every replica applies the same committed [`Command`]s in the same order and
//! must reach the same [`ShardState`]. So nothing here reads a clock, draws a
//! random number or does I/O: time, ids and finalized event envelopes travel
//! inside the command, chosen once by whoever proposed it.
//!
//! A session's canonical Everruns event log lives here, in memory, beside its
//! inbox and the claim naming the executor allowed to run its current turn.
//! The Raft log in front of it is the write-ahead log; snapshots of this value
//! let the WAL be truncated.

use std::collections::{BTreeMap, HashMap};

use everruns_contracts::typed_id::{EventId, SessionId};
use everruns_core::events::{Event, EventData, EventRequest};
use openraft::{BasicNode, LogId, StoredMembership};
use serde::{Deserialize, Serialize};

use crate::types::NodeId;

/// Identifies one shard (one Raft group) in the fleet.
pub type ShardId = u32;

/// A replicated change to the shard.
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
        /// The command carries payloads by reference (`payload.rs`).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        blobs: bool,
    },
    /// Bill `session`'s spend to `tenant`. Set once, right after create,
    /// before the session takes input; a different tenant is refused.
    Assign {
        session: SessionId,
        tenant: String,
        /// The labels the session was created with (agentd's public API):
        /// set with the tenant, never changed after.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        labels: BTreeMap<String, String>,
    },
    /// Read the authorization tenant through the log, after committed Assign.
    Tenant { session: SessionId },
    /// Hold `session` closed to messages while the control plane imports a
    /// moved v1 session into it (`on`), or open it once the import is done.
    /// Proposed before the create, so the session never takes a message
    /// before its history, storage and files are in.
    Importing { session: SessionId, on: bool },
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
    /// A control plane config update for `org` (directory shard).
    Config {
        org: String,
        update: crate::dp::ConfigUpdate,
        now_ms: i64,
    },
    /// A control plane admin command for `org` (directory shard).
    Admin {
        org: String,
        command: crate::control::AdminCommand,
        now_ms: i64,
    },
    /// A change to `org`'s group: its limits, its spend ledger, the
    /// install's controller (directory shard, `group.rs`).
    Group {
        org: String,
        change: crate::group::GroupOp,
        now_ms: i64,
    },
    /// An executor leasing or refreshing `org`'s secret (directory shard,
    /// `secrets.rs`).
    Secret {
        org: String,
        request: crate::secrets::SecretOp,
        now_ms: i64,
    },
    /// The control plane stored the feed through `(index, ordinal)`.
    FeedAcked { through: (u64, u32) },
    /// The event types each org taps, as the directory holds them (`tap.rs`).
    Taps {
        taps: std::collections::BTreeMap<String, Vec<String>>,
    },
    /// The leader's minute report for Monitoring: its node measurements;
    /// applying it counts the population and queues the feed record.
    Report { report: crate::control::NodeReport },
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
    /// this epoch again. `forgive`: instead, the epoch was lost to slow
    /// renewals while its holder stayed alive (`Authority::forgive`).
    Progress {
        session: SessionId,
        holder: String,
        fence: u64,
        now_ms: i64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        forgive: bool,
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
        /// The event carries payloads by reference (`payload.rs`).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        blobs: bool,
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
    /// A change to `session`'s SQL databases, fenced as `Files` (`sqldb`).
    SqlDb {
        session: SessionId,
        #[serde(default)]
        fence: Option<u64>,
        op_id: String,
        change: crate::sqldb::SqlChange,
        now_ms: i64,
    },
    /// A change to `session`'s schedules (`platform.rs`).
    Platform {
        session: SessionId,
        action: crate::platform::Op,
        now_ms: i64,
        seed: EventId,
    },
    /// Directory: `session` lives in `shard` at placement `generation`. Only
    /// the directory shard applies it.
    ///
    /// It commits only over the entry at generation `previous` (0: none),
    /// a compare-and-swap that is a move's ownership transfer: a late or
    /// duplicated placement of an older move can never repoint the session
    /// (`Moves` model in `tests/models/data-plane-v2/`). A failed swap
    /// answers [`Response::Placement`] with the entry as it is. A new
    /// session's first placement (`previous` 0) is refused by a draining
    /// shard.
    Place {
        session: SessionId,
        shard: ShardId,
        generation: u64,
        previous: u64,
    },
    /// Directory: read `session`'s entry through the log, so the answer
    /// ([`Response::Placement`]) is current, not a replica's lagging view.
    Locate { session: SessionId },
    /// Record `shard`'s current members in the registry (directory shard).
    /// A list, not a map: integer map keys do not survive serde's
    /// internally tagged enums.
    RegisterShard {
        shard: ShardId,
        members: Vec<(NodeId, String)>,
    },
    /// Directory: start (or stop) draining `shard`. New sessions are not
    /// placed in a draining shard, and the rebalancer moves its sessions out.
    DrainShard { shard: ShardId, drain: bool },
    /// Directory: drop a drained, empty `shard` from the registry for good.
    /// Its nodes can then be shut down; it can never register again.
    RetireShard { shard: ShardId },
    /// Stop starting turns and accepting input, ahead of a move to `to`.
    /// The shard's leader finishes the move once the session is idle, so a
    /// move survives the death of whoever asked for it.
    Freeze {
        session: SessionId,
        #[serde(default)]
        to: Option<ShardId>,
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
    /// Take a live session back to `image`, its state at an earlier point
    /// (a one-org restore, `restore::restore_org`): history, inbox and
    /// workspace as they were then, written on from here on `branch`.
    /// Placement and generation stay this copy's; the epoch counter jumps
    /// past anything handed out and any claim ends, so a running turn's
    /// executor is fenced out. Refused while the session is moving.
    Revert {
        session: SessionId,
        image: Box<SessionState>,
        branch: u64,
        now_ms: i64,
    },
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
        to: Option<ShardId>,
    },
    /// Install a session moved from another shard, prepared: it runs nothing
    /// until `Activate`. Only a generation newer than any copy this shard
    /// holds or has seen leave is taken.
    Import {
        session: SessionId,
        image: Box<SessionState>,
    },
    /// The directory committed this shard's prepared copy at `generation`:
    /// it may run now.
    Activate { session: SessionId, generation: u64 },
    /// The session now lives in `shard` at `generation`; drop the leaving
    /// copy it was taken from (`generation - 1`) and redirect callers.
    Forward {
        session: SessionId,
        shard: ShardId,
        generation: u64,
    },
    /// Drop a prepared copy the directory never committed: it placed the
    /// session in `shard` at `generation` or later.
    Drop {
        session: SessionId,
        shard: ShardId,
        generation: u64,
    },
    /// Directory: slot `slot`'s entries now live in shard `to`.
    AssignSlot { slot: u16, to: ShardId },
    /// Stop taking directory writes for `slot`: it is being handed to
    /// `to`, and a `Place` for it is answered with `Moved { shard: to }`.
    SealSlot { slot: u16, to: ShardId },
    /// Take directory entries handed over from another shard. Of an entry
    /// this shard already has, the higher generation stays.
    Adopt {
        slot: u16,
        entries: Vec<(SessionId, Placed)>,
    },
    /// Forget a sealed slot's entries once its new owner serves them.
    DropSlot { slot: u16 },
    /// The shard's leader stored the session's events `seg` in the object
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

impl Command {
    /// When the command was proposed, if it says: the newest time among
    /// the commands it holds.
    pub fn time(&self) -> Option<i64> {
        match self {
            Command::Batch { commands } | Command::Chain { commands } => {
                commands.iter().filter_map(Command::time).max()
            }
            Command::Host { now_ms, .. }
            | Command::Config { now_ms, .. }
            | Command::Admin { now_ms, .. }
            | Command::Group { now_ms, .. }
            | Command::Secret { now_ms, .. }
            | Command::Reserve { now_ms, .. }
            | Command::Claim { now_ms, .. }
            | Command::Renew { now_ms, .. }
            | Command::Progress { now_ms, .. }
            | Command::Release { now_ms, .. }
            | Command::Append { now_ms, .. }
            | Command::Write { now_ms, .. }
            | Command::Files { now_ms, .. }
            | Command::SqlDb { now_ms, .. }
            | Command::Platform { now_ms, .. }
            | Command::Revoke { now_ms, .. }
            | Command::Revert { now_ms, .. } => Some(*now_ms),
            _ => None,
        }
    }
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

#[path = "state_response.rs"]
mod response;
pub use response::Response;

/// Lifecycle of a session within this shard.
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
    /// Leaving for another shard: no new turns or input.
    Frozen,
    /// Imported, waiting for the directory to commit its generation.
    Prepared,
}

/// A directory entry: the session's shard and placement generation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Placed {
    pub shard: ShardId,
    pub generation: u64,
}

/// A session that left this shard: where it went, and the highest
/// generation that left (its tombstone: an import at or below it is late).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Left {
    pub shard: ShardId,
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
    /// In a snapshot: the key of `definition` in [`ShardState::definitions`].
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
    /// each with a `Stored` command, so the shard keeps one copy.
    #[serde(default)]
    pub segments: Vec<crate::store::SegRef>,
    pub events: Vec<Event>,
    /// What Engine's hosting contract asks the shard to own for the session:
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
    /// The shard this session is being moved to, while frozen for a move.
    #[serde(default)]
    pub moving_to: Option<ShardId>,
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
    /// The session's schedules (`platform.rs`).
    #[serde(default, skip_serializing_if = "crate::platform::Platform::is_empty")]
    pub platform: crate::platform::Platform,
    /// The session's SQL databases (see `sqldb`).
    #[serde(default, skip_serializing_if = "crate::sqldb::Databases::is_empty")]
    pub sqldb: crate::sqldb::Databases,
    /// Closed to messages while a moved v1 session is imported into it
    /// ([`Command::Importing`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub importing: bool,
    /// The session's history branch: 0 until a restore takes it back to an
    /// earlier point, then a newer number no earlier branch of it used
    /// (`new_branch`). The segments it writes carry it (`store::SegRef`).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub branch: u64,
    /// The labels it was created with ([`Command::Assign`]); copied onto
    /// every feed record about it, so agentd's stream filters on them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    /// Some event holds a payload by reference (`payload.rs`): readers
    /// that hand events out resolve them. Never unset.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub blobs: bool,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// A history branch number for a restore at `now_ms`: the restore's time
/// in seconds in the high half, so later branches order above earlier ones
/// (`control::turn_finished`), and 32 random bits below, so two restores of
/// one session never share a branch's segment names.
pub fn new_branch(now_ms: i64) -> u64 {
    ((now_ms.max(0) / 1000) as u64) << 32 | u64::from(rand::random::<u32>().max(1))
}

/// Everything a shard replicates.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShardState {
    pub last_applied: Option<LogId<NodeId>>,
    pub membership: StoredMembership<NodeId, BasicNode>,
    /// Which life of the shard this is: 0 until its first restore, then the
    /// number the restore claimed (`restore.rs`). Feed records, segment
    /// names and the shipped log carry it, so nothing a lost incarnation
    /// wrote is mistaken for this one's.
    #[serde(default)]
    pub incarnation: u64,
    /// The newest command time applied: the shard's clock, which a
    /// point-in-time restore stops at. Replicated, so every replica agrees.
    #[serde(default)]
    pub clock_ms: i64,
    pub sessions: HashMap<SessionId, SessionState>,
    /// Sessions this shard handed to another shard, or dropped a stale copy
    /// of. A copy that comes back removes its entry: that copy's
    /// generation is above it.
    pub forwarded: HashMap<SessionId, Left>,
    /// Directory entries; populated only in the directory shard.
    pub directory: HashMap<SessionId, Placed>,
    /// Shard registry: each shard's members as its leader last reported them.
    /// Populated only in the directory shard; gateways read the fleet's
    /// topology from it, so a shard joins by registering.
    #[serde(default)]
    pub shards: std::collections::BTreeMap<ShardId, std::collections::BTreeMap<NodeId, String>>,
    /// Shards being emptied ahead of retirement (directory shard only).
    #[serde(default)]
    pub draining: std::collections::BTreeSet<ShardId>,
    /// Shards taken out of the fleet (directory shard only). Kept so that a
    /// retired shard's nodes, still running, cannot register it again.
    #[serde(default)]
    pub retired: std::collections::BTreeSet<ShardId>,
    /// Which shard holds each directory slot's entries (directory shard
    /// only). A slot not listed lives in the directory shard.
    #[serde(default)]
    pub slots: std::collections::BTreeMap<u16, ShardId>,
    /// Slots this shard handed to another shard, and to which.
    #[serde(default)]
    pub sealed: std::collections::BTreeMap<u16, ShardId>,
    /// In a snapshot only: each agent definition once, by key; sessions
    /// point at theirs with `definition_ref`. Empty in memory.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub definitions: std::collections::BTreeMap<String, serde_json::Value>,
    /// Spend allowances, per tenant.
    #[serde(default)]
    pub allowances: crate::allowance::Allowances,
    /// Each org's control plane config (directory shard only).
    #[serde(default)]
    pub orgs: crate::control::Orgs,
    /// Feed records the control plane has not acknowledged yet.
    #[serde(default)]
    pub feed: crate::control::Feed,
    /// Commands arriving in pieces, by id.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub parts: std::collections::BTreeMap<uuid::Uuid, Assembly>,
    /// Sessions a create will make closed to messages ([`Command::Importing`]):
    /// the import's mark, taken before the session exists.
    #[serde(default, skip_serializing_if = "std::collections::HashSet::is_empty")]
    pub importing: std::collections::HashSet<SessionId>,
    /// Durations (ms) of turns finished since the state machine last took
    /// them, for the turn SLO gauge. Not replicated state.
    #[serde(skip)]
    pub turns_finished: Vec<u64>,
}

impl ShardState {
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

/// Directory slots. A session's directory entry lives in the shard that
/// holds its slot.
///
/// Decision: one directory shard holding every session's entry would cap the
/// fleet at what one Raft group can store and write. The entries are split
/// into a fixed number of slots by session id, and the directory shard
/// keeps only which shard holds each slot (a small table gateways cache).
/// Slots move between shards as shards join and leave (see
/// `Node::rebalance_slots`).
pub const SLOTS: u16 = 256;

/// The directory slot of `session`: FNV-1a of its id.
pub fn slot_of(session: SessionId) -> u16 {
    let hash = session
        .to_string()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
    (hash % u64::from(SLOTS)) as u16
}

#[path = "state_definitions.rs"]
pub mod definitions;

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

impl ShardState {
    /// [`ShardState::apply`] without the event tap (`tap.rs`): nested
    /// commands come through here so the tap sees each change once.
    fn apply_command(&mut self, command: Command, changes: &mut Vec<Change>) -> Response {
        if let Some(at) = command.time()
            && at > self.clock_ms
        {
            self.clock_ms = at;
        }
        match command {
            Command::AssignSlot { slot, to } => {
                if self.slots.insert(slot, to) == Some(to) {
                    return Response::Duplicate;
                }
                Response::Ok
            }
            Command::SealSlot { slot, to } => {
                self.sealed.insert(slot, to);
                Response::Ok
            }
            Command::Adopt { slot, entries } => {
                // The slot is back here (or here for the first time).
                self.sealed.remove(&slot);
                for (session, placed) in entries {
                    let entry = self.directory.entry(session).or_insert(placed);
                    if placed.generation > entry.generation {
                        *entry = placed;
                    }
                }
                Response::Ok
            }
            Command::DropSlot { slot } => {
                if self.sealed.contains_key(&slot) {
                    self.directory
                        .retain(|session, _| slot_of(*session) != slot);
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
                    Ok(command) => self.apply_command(command, changes),
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
                        let response = self.apply_command(command, changes);
                        failed = !matches!(response, Response::Value { .. } | Response::Ok);
                        response
                    })
                    .collect();
                Response::Batch { responses }
            }
            Command::Batch { commands } => Response::Batch {
                responses: commands
                    .into_iter()
                    .map(|command| self.apply_command(command, changes))
                    .collect(),
            },
            Command::Host {
                session,
                command,
                definition,
                now_ms,
                seed,
                blobs,
            } => {
                use everruns::hosting::spi::session_commands::SessionCommand;
                let create = matches!(command, SessionCommand::Create { .. });
                if let Some(left) = self.forwarded.get(&session) {
                    return Response::Moved { shard: left.shard };
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
                            importing: self.importing.contains(&session),
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
                        state.blobs |= blobs;
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
            Command::Platform {
                session,
                action,
                now_ms,
                seed,
            } => self.apply_platform(session, action, now_ms, seed, changes),
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
            Command::Tenant { session } => match self.sessions.get(&session) {
                Some(state) => Response::Value {
                    value: serde_json::json!(state.tenant),
                },
                None => self.missing(session),
            },
            Command::Importing { session, on } => {
                if on {
                    self.importing.insert(session);
                } else {
                    self.importing.remove(&session);
                }
                let Some(state) = self.session_mut(session) else {
                    // A mark waits for the create.
                    return if on {
                        Response::Ok
                    } else {
                        self.missing(session)
                    };
                };
                if state.importing != on {
                    state.importing = on;
                    changes.push(Change::Session(session));
                }
                Response::Ok
            }
            Command::Assign {
                session,
                tenant,
                labels,
            } => {
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
                        state.labels = labels;
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
                let index = self.last_applied.map_or(0, |l| l.index);
                crate::control::config(&mut self.orgs, &mut self.feed, index, &org, &update, now_ms)
            }
            Command::Admin {
                org,
                command,
                now_ms,
            } => {
                let index = self.last_applied.map_or(0, |l| l.index);
                crate::control::admin(&mut self.orgs, &mut self.feed, index, &org, command, now_ms)
            }
            Command::Group {
                org,
                change,
                now_ms,
            } => crate::group::apply(&mut self.orgs, &org, change, now_ms),
            Command::Secret {
                org,
                request,
                now_ms,
            } => {
                let index = self.last_applied.map_or(0, |l| l.index);
                crate::secrets::apply(&mut self.orgs, &mut self.feed, index, &org, request, now_ms)
            }
            Command::Taps { taps } => self.set_taps(taps),
            Command::FeedAcked { through } => {
                self.feed.ack(through);
                Response::Ok
            }
            Command::Report { report } => {
                let index = self.last_applied.map_or(0, |l| l.index);
                self.feed
                    .report(index, self.sessions.values(), self.forwarded.len(), &report);
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
                    self.turns_finished.extend(state.last_turn_ms());
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
                forgive,
            } => {
                let Some(state) = self.session_mut(session) else {
                    return self.missing(session);
                };
                let done = if forgive {
                    state.forgive(&holder, fence)
                } else {
                    state.progress(&holder, fence, now_ms)
                };
                if done { Response::Ok } else { Response::Fenced }
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
                    self.turns_finished.extend(state.last_turn_ms());
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
                blobs,
            } => {
                let Some(state) = self.session_mut(session) else {
                    return self.missing(session);
                };
                state.blobs |= blobs;
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
                    self.turns_finished.extend(state.last_turn_ms());
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
                if let Some(refused) = state.unwritable(fence, now_ms) {
                    return refused;
                }
                let value = state.files.commit(op, commit, now_ms);
                changes.push(Change::Session(session));
                Response::Value { value }
            }
            Command::SqlDb {
                session,
                fence,
                op_id: op,
                change,
                now_ms,
            } => {
                let Some(state) = self.session_mut(session) else {
                    return self.missing(session);
                };
                changes.push(Change::Session(session));
                state.sqldb_commit(fence, op, change, now_ms)
            }
            Command::Locate { session } => {
                if let Some(to) = self.sealed.get(&slot_of(session)) {
                    return Response::Moved { shard: *to };
                }
                Response::Placement {
                    current: self.directory.get(&session).copied(),
                }
            }
            Command::Place {
                session,
                shard,
                generation,
                previous,
            } => {
                if let Some(to) = self.sealed.get(&slot_of(session)) {
                    return Response::Moved { shard: *to };
                }
                let placed = Placed { shard, generation };
                let current = self.directory.get(&session).copied();
                if current == Some(placed) {
                    return Response::Duplicate;
                }
                if generation <= previous || current.map_or(0, |p| p.generation) != previous {
                    return Response::Placement { current };
                }
                // A gateway placing from a view a second old must not put a
                // new session in a shard that is being emptied.
                if previous == 0
                    && (self.draining.contains(&shard) || self.retired.contains(&shard))
                {
                    return Response::Rejected {
                        reason: format!("shard {shard} is draining"),
                    };
                }
                self.directory.insert(session, placed);
                Response::Ok
            }
            Command::RegisterShard { shard, members } => {
                let members: std::collections::BTreeMap<NodeId, String> =
                    members.into_iter().collect();
                if self.retired.contains(&shard) {
                    return Response::Rejected {
                        reason: format!("shard {shard} is retired"),
                    };
                }
                if self.shards.get(&shard) == Some(&members) {
                    return Response::Duplicate;
                }
                self.shards.insert(shard, members);
                Response::Ok
            }
            Command::DrainShard { shard, drain } => {
                if self.retired.contains(&shard) || !self.shards.contains_key(&shard) {
                    return Response::NotFound;
                }
                let changed = if drain {
                    self.draining.insert(shard)
                } else {
                    self.draining.remove(&shard)
                };
                if changed {
                    Response::Ok
                } else {
                    Response::Duplicate
                }
            }
            Command::RetireShard { shard } => {
                if self.retired.contains(&shard) {
                    return Response::Duplicate;
                }
                if !self.draining.contains(&shard) {
                    return Response::Rejected {
                        reason: format!("shard {shard} is not draining"),
                    };
                }
                // Decision: the directory does not know a shard's sessions,
                // only the shard does; whoever retires it checks that it is
                // empty first (the gateway does). A placement still naming
                // it is dropped here so lookups fail fast, not into a void.
                self.directory.retain(|_, p| p.shard != shard);
                self.draining.remove(&shard);
                self.shards.remove(&shard);
                self.retired.insert(shard);
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
                // back to this shard would thaw the leaving copy while the
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
            Command::Revert {
                session,
                image,
                branch,
                now_ms: _,
            } => {
                let index = self.last_applied.map_or(0, |l| l.index);
                let Some(state) = self.sessions.get_mut(&session) else {
                    return self.missing(session);
                };
                if state.branch == branch {
                    return Response::Duplicate;
                }
                if state.placement != Placement::Live {
                    return Response::Rejected {
                        reason: "the session is moving; restore it once it settles".into(),
                    };
                }
                let mut image = *image;
                image.authority.fence = state.authority.fence.max(image.authority.fence) + 1;
                image.authority.holder = None;
                image.placement = state.placement;
                image.generation = state.generation;
                image.moving_to = None;
                image.tenant = state.tenant.clone();
                image.branch = branch;
                *state = image;
                self.feed.turn_finished(index, &session.to_string(), state);
                changes.push(Change::Session(session));
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
                shard,
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
                        shard,
                        generation: generation - 1,
                    },
                );
                changes.push(Change::Session(session));
                Response::Ok
            }
            Command::Drop {
                session,
                shard,
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
                            shard,
                            generation: s.generation,
                        },
                    );
                }
                changes.push(Change::Session(session));
                Response::Ok
            }
        }
    }

    pub(crate) fn session_mut(&mut self, session: SessionId) -> Option<&mut SessionState> {
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
                    if let Some(micros) = tap::generation_cost_micros(event) {
                        let tenant = self.sessions.get(&session).and_then(|s| s.tenant.clone());
                        self.allowances.settle(tenant.as_deref(), session, micros);
                        if let Some(s) = self.sessions.get_mut(&session)
                            && s.platform.charge(event, micros)
                        {
                            changes.push(Change::Session(session));
                        }
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

    pub(crate) fn missing(&self, session: SessionId) -> Response {
        match self.forwarded.get(&session) {
            Some(left) => Response::Moved { shard: left.shard },
            None => Response::NotFound,
        }
    }
}

#[path = "state_session.rs"]
mod session;

#[path = "tap.rs"]
mod tap;

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "state_moves_tests.rs"]
mod moves_tests;

#[cfg(test)]
#[path = "state_format_tests.rs"]
mod format_tests;

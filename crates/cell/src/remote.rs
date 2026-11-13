//! Executors outside the cell (data plane v2 step 6): the wire between an
//! executor and the cells whose sessions it runs, and the executor's mirror
//! of those sessions.
//!
//! Decisions:
//! - An executor runs Engine's own activation against the same hosting code
//!   a cell node uses ([`crate::hosting::RaftHosting`]). Only where that
//!   code reaches the cell differs: a node proposes through Raft and reads
//!   its replica; an executor sends the same [`Command`]s over HTTP and
//!   reads a **mirror**, a [`CellState`] holding just the sessions it runs.
//!   One authority implementation, so the executor cannot drift from it.
//! - Every proposal's reply carries the session as the cell holds it right
//!   after the write (a [`SessionDelta`]): metadata whole, events only past
//!   what the mirror has. Reads after an executor's own writes therefore
//!   cost no extra round trip, and a turn's waited writes stay the only
//!   waits. Writes by others (input, cancel, a freeze for a move) reach the
//!   mirror through a long poll per running session ([`watch`](Remote::watch)).
//! - A delta is the session at one applied index, so a newer one replaces
//!   an older one whole; a late reply never rolls the mirror back.
//! - History stays where the cell put it: the mirror reads stored segments
//!   from the object store directly, through the executor's segment cache.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use everruns_contracts::typed_id::SessionId;
use everruns_core::events::Event;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::sm::Shared;
use crate::state::{CellId, CellState, Command, Left, Response, SessionState};
use crate::types::NodeId;

/// An executor's id. Executors and cell nodes share one id space in an
/// epoch's holder record; executors use ids from [`EXECUTOR_IDS`] up.
pub type ExecutorId = NodeId;

/// The first executor id: below it, ids name cell nodes.
pub const EXECUTOR_IDS: ExecutorId = 1 << 32;

/// A session as a cell sends it to an executor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDelta {
    /// The cell's applied index the view was taken at.
    pub applied: u64,
    /// The session, with only the events past the executor's `since` (and
    /// its definition only when the executor asked for the whole session).
    #[serde(default)]
    pub session: Option<Box<SessionState>>,
    /// Where the session went, when it is no longer here.
    #[serde(default)]
    pub left: Option<Left>,
    /// The view is the whole session (the executor sent no `since`).
    /// Decision: an explicit flag, because a session's definition may be
    /// null (one bound through Engine's own binding), so a null definition
    /// cannot mark a partial view.
    #[serde(default)]
    pub whole: bool,
}

/// `POST /cell/exec/propose`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ExecPropose {
    pub session: SessionId,
    /// The last sequence the executor's mirror holds (`None`: send it all).
    pub since: Option<i32>,
    pub command: Command,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExecProposed {
    pub response: Response,
    pub index: u64,
    pub delta: SessionDelta,
    /// The cell's leader, as a hint for the next call.
    #[serde(default)]
    pub leader: Option<String>,
}

/// `POST /cell/exec/pull`: `credits` free slots offered to this cell.
#[derive(Debug, Serialize, Deserialize)]
pub struct PullRequest {
    pub executor: ExecutorId,
    pub credits: usize,
    /// How long the cell may hold the credits before returning them.
    pub wait_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PullReply {
    /// Sessions to activate, one credit each.
    pub sessions: Vec<SessionId>,
    /// Set when the node asked is not the leader: ask this one.
    #[serde(default)]
    pub leader: Option<String>,
    /// Each session's tenant, in the order of `sessions` (the executor's
    /// own suspension check, see `tenants.rs`).
    #[serde(default)]
    pub tenants: Vec<Option<String>>,
}

pub use crate::stream::LiveBatch;

/// `session` as an executor needs it: whole but for the events at or below
/// `since` (and without its definition when `since` is set: the executor
/// already holds it).
pub fn view(s: &SessionState, since: Option<i32>) -> SessionState {
    let after = since.unwrap_or(0);
    SessionState {
        agent: s.agent.clone(),
        definition: if since.is_none() {
            s.definition.clone()
        } else {
            Arc::new(serde_json::Value::Null)
        },
        definition_ref: None,
        created_ms: s.created_ms,
        offloaded: s.offloaded,
        segments: s.segments.clone(),
        events: s.resident_after(after).to_vec(),
        authority: s.authority.clone(),
        placement: s.placement,
        tenant: s.tenant.clone(),
        generation: s.generation,
        moving_to: s.moving_to,
        turn_start: s.turn_start,
        checkpoints: s.checkpoints.clone(),
        files: s.files.clone(),
        branch: s.branch,
    }
}

/// The delta of `session` at `applied` from `state`.
pub fn delta(
    state: &CellState,
    applied: u64,
    session: SessionId,
    since: Option<i32>,
) -> SessionDelta {
    SessionDelta {
        applied,
        session: state
            .sessions
            .get(&session)
            .map(|s| Box::new(view(s, since))),
        left: state.forwarded.get(&session).copied(),
        whole: since.is_none(),
    }
}

/// Fold `delta` into the mirror. `false` when the mirror's events and the
/// delta's do not join up (the caller then fetches the session whole).
pub fn merge(state: &mut CellState, session: SessionId, delta: SessionDelta) -> bool {
    let Some(mut new) = delta.session else {
        state.sessions.remove(&session);
        if let Some(left) = delta.left {
            state.forwarded.insert(session, left);
        }
        return true;
    };
    state.forwarded.remove(&session);
    let seq = |e: &Event| e.sequence.unwrap_or(0);
    if delta.whole {
        // Replaces whatever the mirror held.
    } else if let Some(old) = state.sessions.remove(&session) {
        // A restore took the session back (`Command::Revert`): the mirror's
        // events past the point are not its history any more.
        if old.branch != new.branch {
            return false;
        }
        new.definition = old.definition;
        let first_new = new.events.first().map(seq);
        let mut events: Vec<Event> = old
            .events
            .into_iter()
            .filter(|e| seq(e) > new.offloaded && first_new.is_none_or(|first| seq(e) < first))
            .collect();
        events.append(&mut new.events);
        new.events = events;
    } else {
        return false; // a partial view of a session the mirror never held
    }
    let dense = new
        .events
        .iter()
        .enumerate()
        .all(|(i, e)| seq(e) == new.offloaded + 1 + i as i32);
    if !dense {
        return false;
    }
    state.sessions.insert(session, *new);
    true
}

/// An executor's link to the cells: proposals over HTTP, and the mirror
/// its hosting reads.
pub struct Remote {
    pub id: ExecutorId,
    /// The mirror: the sessions this executor runs (or ran recently).
    pub shared: Arc<Shared>,
    http: reqwest::Client,
    /// Each cell's members, from the topology.
    cells: parking_lot::RwLock<BTreeMap<CellId, BTreeMap<NodeId, String>>>,
    /// Each cell's leader, as last heard.
    leaders: Mutex<HashMap<CellId, String>>,
    /// The cell each mirrored session was pulled from.
    homes: Mutex<HashMap<SessionId, CellId>>,
    /// Per session, the cell's applied index of the mirror's copy.
    applied: Mutex<HashMap<SessionId, u64>>,
    /// Round trips and payload, for the bench.
    pub calls: crate::prom::Counters,
    /// Where live deltas go (each session's stream node).
    deliver: crate::stream::Deliver,
}

/// How long one call to one member may take.
const ATTEMPT: Duration = Duration::from_secs(10);
/// How long a call may take over every member it tries.
const DEADLINE: Duration = Duration::from_secs(20);

impl Remote {
    pub fn new(id: ExecutorId, store: crate::store::Store) -> anyhow::Result<Arc<Self>> {
        let http = reqwest::Client::builder()
            .no_proxy()
            .pool_max_idle_per_host(256)
            .tcp_nodelay(true)
            .build()?;
        Ok(Arc::new(Self {
            id,
            shared: Shared::new(store, CellId::MAX),
            http,
            cells: Default::default(),
            leaders: Default::default(),
            homes: Default::default(),
            applied: Default::default(),
            calls: Default::default(),
            deliver: Default::default(),
        }))
    }

    /// Set the cells this executor serves and their members.
    pub fn set_cells(&self, cells: BTreeMap<CellId, BTreeMap<NodeId, String>>) {
        *self.cells.write() = cells;
    }

    pub fn cells(&self) -> Vec<CellId> {
        self.cells.read().keys().copied().collect()
    }

    /// `session` runs from `cell`.
    ///
    /// Decision: a session that moved drops its mirror. Applied indices
    /// belong to one cell's log, so the old cell's index would make the new
    /// cell's views look late and be ignored (found by the qualification
    /// suite: a session moved back and forth stalled on a stale mirror).
    /// `true` when it moved: the caller drops its own positions too.
    pub fn bind(&self, session: SessionId, cell: CellId) -> bool {
        let previous = self.homes.lock().insert(session, cell);
        let moved = previous.is_some_and(|previous| previous != cell);
        if moved {
            self.shared.state.write().sessions.remove(&session);
            self.applied.lock().remove(&session);
        }
        moved
    }

    /// The cell `session` runs from. Decision: a session nothing bound
    /// here belongs to the only cell when the fleet has one (a link that
    /// serves one cell, as the hosting conformance drives it); with more
    /// cells only a pull says where a session lives.
    pub fn home(&self, session: SessionId) -> Option<CellId> {
        if let Some(cell) = self.homes.lock().get(&session).copied() {
            return Some(cell);
        }
        let cells = self.cells.read();
        (cells.len() == 1)
            .then(|| cells.keys().next().copied())
            .flatten()
    }

    /// Drop `session` from the mirror.
    pub fn evict(&self, session: SessionId) {
        self.shared.state.write().sessions.remove(&session);
        self.applied.lock().remove(&session);
        self.homes.lock().remove(&session);
    }

    pub fn mirrored(&self) -> usize {
        self.shared.state.read().sessions.len()
    }

    fn since(&self, session: SessionId) -> Option<i32> {
        let state = self.shared.state.read();
        state
            .sessions
            .get(&session)
            .map(|s| s.last_sequence().unwrap_or(0))
    }

    /// Fold `delta` in unless the mirror already holds a newer copy, and
    /// announce a newer copy on the mirror (`Shared::publish`).
    ///
    /// Decision: every path that refreshes the mirror announces, not only
    /// the watch, as every applied change does on a node. An input that
    /// reaches the mirror in the reply to this executor's own write (a
    /// claim renewal, an append) moves the mirror past it, so the watch,
    /// which waits for changes after the mirror, would never report it.
    fn absorb(&self, session: SessionId, delta: SessionDelta) -> bool {
        let (merged, advanced) = {
            let mut applied = self.applied.lock();
            let current = applied.get(&session).copied().unwrap_or(0);
            if delta.applied < current {
                return true; // late: the mirror is newer
            }
            let at = delta.applied;
            let mut state = self.shared.state.write();
            if merge(&mut state, session, delta) {
                applied.insert(session, at);
                (true, at > current)
            } else {
                state.sessions.remove(&session);
                applied.remove(&session);
                (false, false)
            }
        };
        if advanced {
            self.shared.publish(crate::state::Change::Session(session));
        }
        merged
    }

    /// The members of `cell`, its last known leader first.
    fn targets(&self, cell: CellId) -> Vec<String> {
        let mut targets: Vec<String> = self
            .cells
            .read()
            .get(&cell)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default();
        if let Some(leader) = self.leaders.lock().get(&cell).cloned() {
            targets.retain(|t| *t != leader);
            targets.insert(0, leader);
        }
        targets
    }

    fn heard_leader(&self, cell: CellId, leader: Option<String>) {
        if let Some(leader) = leader {
            self.leaders.lock().insert(cell, leader);
        }
    }

    /// POST `body` to `path` on `cell`, trying its members in turn.
    pub async fn post<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        cell: CellId,
        path: &str,
        body: &B,
        timeout: Duration,
    ) -> anyhow::Result<T> {
        let deadline = tokio::time::Instant::now() + DEADLINE.max(timeout);
        let mut last = anyhow::anyhow!("cell {cell} has no members");
        loop {
            for addr in self.targets(cell) {
                let sent = self
                    .http
                    .post(format!("http://{addr}{path}"))
                    .json(body)
                    .timeout(timeout)
                    .send()
                    .await;
                match sent {
                    Ok(reply) if reply.status().is_success() => {
                        self.calls.inc(&[("call", path_kind(path))]);
                        return Ok(reply.json().await?);
                    }
                    Ok(reply) => {
                        last = anyhow::anyhow!("{addr}{path}: {}", reply.status());
                    }
                    Err(error) => last = error.into(),
                }
                if self.leaders.lock().get(&cell) == Some(&addr) {
                    self.leaders.lock().remove(&cell);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(last);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn cell_of(&self, session: SessionId) -> anyhow::Result<CellId> {
        self.home(session)
            .ok_or_else(|| anyhow::anyhow!("session {session} is not bound to a cell here"))
    }

    /// Commit `command` in `session`'s cell and fold the session as it
    /// stands after the write into the mirror.
    pub async fn propose(
        &self,
        session: SessionId,
        command: Command,
    ) -> anyhow::Result<(Response, u64)> {
        let cell = self.cell_of(session)?;
        let mut since = self.since(session);
        for _ in 0..2 {
            let body = ExecPropose {
                session,
                since,
                command: command.clone(),
            };
            let reply: ExecProposed = self
                .post(cell, "/cell/exec/propose", &body, ATTEMPT)
                .await?;
            self.heard_leader(cell, reply.leader);
            if self.absorb(session, reply.delta) {
                return Ok((reply.response, reply.index));
            }
            // The mirror lost its place; the command is idempotent, so
            // send it again for the whole session.
            since = None;
        }
        anyhow::bail!("mirror of {session} could not catch up")
    }

    /// Fetch `session` from its cell, at `min_index` or later.
    pub async fn refresh(&self, session: SessionId, min_index: u64) -> anyhow::Result<()> {
        let cell = self.cell_of(session)?;
        let mut since = self.since(session);
        for _ in 0..2 {
            let delta = self.fetch(cell, session, since, min_index, 0, 0).await?;
            if self.absorb(session, delta) {
                return Ok(());
            }
            since = None;
        }
        anyhow::bail!("mirror of {session} could not catch up")
    }

    async fn fetch(
        &self,
        cell: CellId,
        session: SessionId,
        since: Option<i32>,
        min_index: u64,
        changed_after: u64,
        wait_ms: u64,
    ) -> anyhow::Result<SessionDelta> {
        let body = ViewRequest {
            session,
            since,
            min_index,
            changed_after,
            wait_ms,
        };
        self.post(
            cell,
            "/cell/exec/view",
            &body,
            ATTEMPT + Duration::from_millis(wait_ms),
        )
        .await
        .map_err(|e| anyhow::anyhow!("view of {session} in cell {cell} at {min_index}: {e:#}"))
    }

    /// The mirror holds `session` as of `index` or later.
    pub async fn caught_up(&self, session: SessionId, index: u64) -> anyhow::Result<()> {
        let have = self.applied.lock().get(&session).copied();
        let present = self.shared.state.read().sessions.contains_key(&session);
        if present && have.is_some_and(|have| have >= index) {
            return Ok(());
        }
        self.calls.inc(&[("waited", "view")]);
        self.refresh(session, index).await
    }

    /// Wait (up to `wait`) for `session` to change in its cell past what the
    /// mirror holds, and fold the change in. `true` when it changed.
    pub async fn watch(&self, session: SessionId, wait: Duration) -> anyhow::Result<bool> {
        let cell = self.cell_of(session)?;
        let after = self.applied.lock().get(&session).copied().unwrap_or(0);
        let since = self.since(session);
        let delta = self
            .fetch(cell, session, since, 0, after, wait.as_millis() as u64)
            .await?;
        let changed = delta.applied > after;
        if !self.absorb(session, delta) {
            self.refresh(session, 0).await?;
        }
        Ok(changed)
    }

    /// Offer `credits` slots to `cell` and take the sessions it hands out.
    pub async fn pull(
        &self,
        cell: CellId,
        credits: usize,
        wait: Duration,
    ) -> anyhow::Result<Vec<(SessionId, Option<String>)>> {
        let body = PullRequest {
            executor: self.id,
            credits,
            wait_ms: wait.as_millis() as u64,
        };
        for _ in 0..3 {
            let reply: PullReply = self
                .post(cell, "/cell/exec/pull", &body, wait + ATTEMPT)
                .await?;
            match reply.leader {
                Some(leader) if reply.sessions.is_empty() => {
                    self.leaders.lock().insert(cell, leader);
                }
                _ => {
                    let mut tenants = reply.tenants.into_iter();
                    return Ok(reply
                        .sessions
                        .into_iter()
                        .map(|s| (s, tenants.next().flatten()))
                        .collect());
                }
            }
        }
        Ok(Vec::new())
    }

    /// Send an epoch's live deltas to the session's stream node. Best
    /// effort.
    pub async fn live(&self, batch: LiveBatch) {
        let Ok(cell) = self.cell_of(batch.session) else {
            return;
        };
        let seeds = self.targets(cell);
        if self.deliver.send(&self.http, cell, &seeds, batch).await {
            self.calls.inc(&[("call", "live")]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::events::{EventContext, SessionActivatedData};

    fn event(session: SessionId, seq: i32) -> Event {
        let mut event = Event::new(
            session,
            EventContext::default(),
            SessionActivatedData {
                turn_id: everruns_contracts::typed_id::TurnId::new(),
                input_message_id: everruns_contracts::typed_id::MessageId::new(),
            },
        );
        event.sequence = Some(seq);
        event
    }

    fn session(
        id: SessionId,
        events: std::ops::RangeInclusive<i32>,
        offloaded: i32,
    ) -> SessionState {
        SessionState {
            definition: Arc::new(serde_json::json!({ "name": "a" })),
            offloaded,
            events: events.map(|seq| event(id, seq)).collect(),
            ..Default::default()
        }
    }

    /// However the mirror moves forward (a watch, or the reply to the
    /// executor's own write), Engine's waiters hear it; a stale reply does
    /// not wake them.
    #[test]
    fn every_newer_copy_is_announced_and_a_late_one_is_not() {
        let remote = Remote::new(
            crate::remote::EXECUTOR_IDS,
            crate::store::Store::open("memory://").unwrap(),
        )
        .unwrap();
        let heard = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = heard.clone();
        let _ = remote.shared.on_change.set(Box::new(move |_| {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        let id = SessionId::new();
        let mut cell = CellState::default();
        cell.sessions.insert(id, session(id, 1..=5, 0));
        assert!(remote.absorb(id, delta(&cell, 10, id, None)));
        cell.sessions.insert(id, session(id, 1..=6, 0));
        // The reply to a renewal brings the new input with it.
        assert!(remote.absorb(id, delta(&cell, 12, id, Some(5))));
        assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 2);
        // An older reply that arrives late changes nothing.
        assert!(remote.absorb(id, delta(&cell, 11, id, Some(5))));
        assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(
            remote.shared.state.read().sessions[&id].last_sequence(),
            Some(6)
        );
    }

    #[test]
    fn a_delta_appends_only_new_events() {
        let id = SessionId::new();
        let mut cell = CellState::default();
        cell.sessions.insert(id, session(id, 1..=5, 0));
        let mut mirror = CellState::default();
        assert!(merge(&mut mirror, id, delta(&cell, 10, id, None)));
        cell.sessions.insert(id, session(id, 1..=8, 0));
        let partial = delta(&cell, 12, id, Some(5));
        assert_eq!(partial.session.as_ref().unwrap().events.len(), 3);
        assert!(partial.session.as_ref().unwrap().definition.is_null());
        assert!(merge(&mut mirror, id, partial));
        let held = &mirror.sessions[&id];
        assert_eq!(held.last_sequence(), Some(8));
        assert!(!held.definition.is_null(), "the definition stays");
    }

    #[test]
    fn an_offload_drops_stored_events_from_the_mirror() {
        let id = SessionId::new();
        let mut mirror = CellState::default();
        let mut cell = CellState::default();
        cell.sessions.insert(id, session(id, 1..=6, 0));
        assert!(merge(&mut mirror, id, delta(&cell, 1, id, None)));
        cell.sessions.insert(id, session(id, 5..=7, 4));
        assert!(merge(&mut mirror, id, delta(&cell, 2, id, Some(6))));
        let held = &mirror.sessions[&id];
        assert_eq!(held.offloaded, 4);
        assert_eq!(held.events.first().and_then(|e| e.sequence), Some(5));
        assert_eq!(held.last_sequence(), Some(7));
    }

    #[test]
    fn a_gap_asks_for_the_whole_session() {
        let id = SessionId::new();
        let mut mirror = CellState::default();
        let mut cell = CellState::default();
        cell.sessions.insert(id, session(id, 1..=3, 0));
        assert!(merge(&mut mirror, id, delta(&cell, 1, id, None)));
        cell.sessions.insert(id, session(id, 1..=9, 0));
        // The executor claims to hold 6, but holds only 3.
        assert!(!merge(&mut mirror, id, delta(&cell, 2, id, Some(6))));
        // A partial view of a session the mirror never held.
        let mut other = CellState::default();
        assert!(!merge(&mut other, id, delta(&cell, 2, id, Some(2))));
    }

    #[test]
    fn a_restored_session_is_fetched_whole_again() {
        let id = SessionId::new();
        let mut mirror = CellState::default();
        let mut cell = CellState::default();
        cell.sessions.insert(id, session(id, 1..=9, 0));
        assert!(merge(&mut mirror, id, delta(&cell, 1, id, None)));
        // Taken back to 5 on a new branch: the mirror's 6..=9 are not its
        // history any more, so a partial view does not join up.
        let mut restored = session(id, 1..=6, 0);
        restored.branch = 7;
        cell.sessions.insert(id, restored);
        assert!(!merge(&mut mirror, id, delta(&cell, 2, id, Some(9))));
        assert!(merge(&mut mirror, id, delta(&cell, 3, id, None)));
        assert_eq!(mirror.sessions[&id].last_sequence(), Some(6));
    }

    #[test]
    fn a_session_that_left_leaves_the_mirror() {
        let id = SessionId::new();
        let mut mirror = CellState::default();
        let mut cell = CellState::default();
        cell.sessions.insert(id, session(id, 1..=2, 0));
        assert!(merge(&mut mirror, id, delta(&cell, 1, id, None)));
        cell.sessions.remove(&id);
        cell.forwarded.insert(
            id,
            Left {
                cell: 3,
                generation: 2,
            },
        );
        assert!(merge(&mut mirror, id, delta(&cell, 2, id, Some(2))));
        assert!(!mirror.sessions.contains_key(&id));
        assert_eq!(mirror.forwarded[&id].cell, 3);
    }
}

/// `POST /cell/exec/view`: the session, once the node applied `min_index`;
/// with `wait_ms`, once it changed after `changed_after` (or the wait ran
/// out).
#[derive(Debug, Serialize, Deserialize)]
pub struct ViewRequest {
    pub session: SessionId,
    pub since: Option<i32>,
    #[serde(default)]
    pub min_index: u64,
    #[serde(default)]
    pub changed_after: u64,
    #[serde(default)]
    pub wait_ms: u64,
}

fn path_kind(path: &str) -> &'static str {
    match path {
        "/cell/exec/propose" => "propose",
        "/cell/exec/view" => "view",
        "/cell/exec/pull" => "pull",
        _ => "other",
    }
}

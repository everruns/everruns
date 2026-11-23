//! Executors outside the shard (data plane v2 step 6): the wire between an
//! executor and the shards whose sessions it runs, and the executor's mirror
//! of those sessions.
//!
//! Decisions:
//! - An executor runs Engine's own activation against the same hosting code
//!   a shard node uses ([`crate::hosting::RaftHosting`]). Only where that
//!   code reaches the shard differs: a node proposes through Raft and reads
//!   its replica; an executor sends the same [`Command`]s over HTTP and
//!   reads a **mirror**, a [`ShardState`] holding just the sessions it runs.
//!   One authority implementation, so the executor cannot drift from it.
//! - Every proposal's reply carries the session as the shard holds it right
//!   after the write (a [`SessionDelta`]): metadata whole, events only past
//!   what the mirror has. Reads after an executor's own writes therefore
//!   cost no extra round trip, and a turn's waited writes stay the only
//!   waits. Writes by others (input, cancel, a freeze for a move) reach the
//!   mirror through a long poll per running session ([`watch`](Remote::watch)).
//! - A delta is the session at one applied index, so a newer one replaces
//!   an older one whole; a late reply never rolls the mirror back.
//! - History stays where the shard put it: the mirror reads stored segments
//!   from the object store directly, through the executor's segment cache.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use everruns_contracts::typed_id::SessionId;
use everruns_core::events::Event;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::sm::Shared;
use crate::state::{Command, Left, Response, SessionState, ShardId, ShardState};
use crate::types::NodeId;

/// An executor's id. Executors and shard nodes share one id space in an
/// epoch's holder record; executors use ids from [`EXECUTOR_IDS`] up.
pub type ExecutorId = NodeId;

/// The first executor id: below it, ids name shard nodes.
pub const EXECUTOR_IDS: ExecutorId = 1 << 32;

/// A session as a shard sends it to an executor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDelta {
    /// The shard's applied index the view was taken at.
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

/// `POST /shard/exec/propose`.
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
    /// The shard's leader, as a hint for the next call.
    #[serde(default)]
    pub leader: Option<String>,
}

/// `POST /shard/exec/pull`: `credits` free slots offered to this shard.
#[derive(Debug, Serialize, Deserialize)]
pub struct PullRequest {
    pub executor: ExecutorId,
    pub credits: usize,
    /// How long the shard may hold the credits before returning them.
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
        importing: s.importing,
        turn_start: s.turn_start,
        checkpoints: s.checkpoints.clone(),
        files: s.files.clone(),
        platform: s.platform.clone(),
        sqldb: s.sqldb.clone(),
        branch: s.branch,
        labels: s.labels.clone(),
        blobs: s.blobs,
    }
}

/// The delta of `session` at `applied` from `state`.
pub fn delta(
    state: &ShardState,
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
pub fn merge(state: &mut ShardState, session: SessionId, delta: SessionDelta) -> bool {
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

/// An executor's link to the shards: proposals over HTTP, and the mirror
/// its hosting reads.
pub struct Remote {
    pub id: ExecutorId,
    /// The mirror: the sessions this executor runs (or ran recently).
    pub shared: Arc<Shared>,
    http: reqwest::Client,
    /// Each shard's members, from the topology.
    shards: parking_lot::RwLock<BTreeMap<ShardId, BTreeMap<NodeId, String>>>,
    /// Each shard's leader, as last heard.
    leaders: Mutex<HashMap<ShardId, String>>,
    /// The shard each mirrored session was pulled from.
    homes: Mutex<HashMap<SessionId, ShardId>>,
    /// Per session, the shard's applied index of the mirror's copy.
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
            shared: Shared::new(store, ShardId::MAX),
            http,
            shards: Default::default(),
            leaders: Default::default(),
            homes: Default::default(),
            applied: Default::default(),
            calls: Default::default(),
            deliver: Default::default(),
        }))
    }

    /// Set the shards this executor serves and their members.
    pub fn set_shards(&self, shards: BTreeMap<ShardId, BTreeMap<NodeId, String>>) {
        *self.shards.write() = shards;
    }

    pub fn shards(&self) -> Vec<ShardId> {
        self.shards.read().keys().copied().collect()
    }

    /// `session` runs from `shard`.
    ///
    /// Decision: a session that moved drops its mirror. Applied indices
    /// belong to one shard's log, so the old shard's index would make the new
    /// shard's views look late and be ignored (found by the qualification
    /// suite: a session moved back and forth stalled on a stale mirror).
    /// `true` when it moved: the caller drops its own positions too.
    pub fn bind(&self, session: SessionId, shard: ShardId) -> bool {
        let previous = self.homes.lock().insert(session, shard);
        let moved = previous.is_some_and(|previous| previous != shard);
        if moved {
            // `applied` before the state, as `absorb` takes them.
            let mut applied = self.applied.lock();
            let mut state = self.shared.state.write();
            state.sessions.remove(&session);
            state.forwarded.remove(&session);
            applied.remove(&session);
        }
        moved
    }

    /// The shard `session` runs from. Decision: a session nothing bound
    /// here belongs to the only shard when the fleet has one (a link that
    /// serves one shard, as the hosting conformance drives it); with more
    /// shards only a pull says where a session lives.
    pub fn home(&self, session: SessionId) -> Option<ShardId> {
        if let Some(shard) = self.homes.lock().get(&session).copied() {
            return Some(shard);
        }
        let shards = self.shards.read();
        (shards.len() == 1)
            .then(|| shards.keys().next().copied())
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

    /// Fold `delta`, as `shard` answered it, in unless the mirror already
    /// holds a newer copy, and announce a newer copy on the mirror
    /// (`Shared::publish`).
    ///
    /// Decision: only the shard the session is bound to may move the mirror.
    /// Applied indices belong to one shard's log, so a reply from the shard a
    /// session left (a long poll or write still in flight across the move)
    /// would set an index the new shard's log has not reached: every reply
    /// from the new shard then looks late and is dropped, the mirror keeps
    /// the old shard's "moved", and the turn retries for good (load run 5's
    /// lost turn). The check holds the `applied` lock, which `bind` takes
    /// after rebinding, so a reply checked before a rebind is cleared by it.
    ///
    /// Decision: every path that refreshes the mirror announces, not only
    /// the watch, as every applied change does on a node. An input that
    /// reaches the mirror in the reply to this executor's own write (a
    /// claim renewal, an append) moves the mirror past it, so the watch,
    /// which waits for changes after the mirror, would never report it.
    fn absorb(&self, session: SessionId, shard: ShardId, delta: SessionDelta) -> bool {
        let (merged, advanced) = {
            let mut applied = self.applied.lock();
            if self.home(session) != Some(shard) {
                return true; // from a shard the session is not bound to
            }
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

    /// The members of `shard`, its last known leader first.
    fn targets(&self, shard: ShardId) -> Vec<String> {
        let mut targets: Vec<String> = self
            .shards
            .read()
            .get(&shard)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default();
        if let Some(leader) = self.leaders.lock().get(&shard).cloned() {
            targets.retain(|t| *t != leader);
            targets.insert(0, leader);
        }
        targets
    }

    fn heard_leader(&self, shard: ShardId, leader: Option<String>) {
        if let Some(leader) = leader {
            self.leaders.lock().insert(shard, leader);
        }
    }

    /// POST `body` to `path` on `shard`, trying its members in turn.
    pub async fn post<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        shard: ShardId,
        path: &str,
        body: &B,
        timeout: Duration,
    ) -> anyhow::Result<T> {
        let deadline = tokio::time::Instant::now() + DEADLINE.max(timeout);
        let mut last = anyhow::anyhow!("shard {shard} has no members");
        loop {
            for addr in self.targets(shard) {
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
                if self.leaders.lock().get(&shard) == Some(&addr) {
                    self.leaders.lock().remove(&shard);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(last);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// POST `body` to `path` on `shard` once: a write whose outcome must not
    /// be guessed (a host call that changes something). Only a member that
    /// could not be connected to is passed over for the next, since the
    /// request never left; a timeout, a dropped connection or an error
    /// status is returned as it is, because the write may have happened.
    pub async fn post_once<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        shard: ShardId,
        path: &str,
        body: &B,
        timeout: Duration,
    ) -> anyhow::Result<T> {
        let mut last = anyhow::anyhow!("shard {shard} has no members");
        for addr in self.targets(shard) {
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
                Ok(reply) => anyhow::bail!("{addr}{path}: {}", reply.status()),
                Err(error) if error.is_connect() => {
                    last = error.into();
                    if self.leaders.lock().get(&shard) == Some(&addr) {
                        self.leaders.lock().remove(&shard);
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(last)
    }

    fn shard_of(&self, session: SessionId) -> anyhow::Result<ShardId> {
        self.home(session)
            .ok_or_else(|| anyhow::anyhow!("session {session} is not bound to a shard here"))
    }

    /// Commit `command` in `session`'s shard and fold the session as it
    /// stands after the write into the mirror.
    pub async fn propose(
        &self,
        session: SessionId,
        command: Command,
    ) -> anyhow::Result<(Response, u64)> {
        let shard = self.shard_of(session)?;
        let mut since = self.since(session);
        for _ in 0..2 {
            let body = ExecPropose {
                session,
                since,
                command: command.clone(),
            };
            let reply: ExecProposed = self
                .post(shard, "/shard/exec/propose", &body, ATTEMPT)
                .await?;
            self.heard_leader(shard, reply.leader);
            if self.absorb(session, shard, reply.delta) {
                return Ok((reply.response, reply.index));
            }
            // The mirror lost its place; the command is idempotent, so
            // send it again for the whole session.
            since = None;
        }
        anyhow::bail!("mirror of {session} could not catch up")
    }

    /// Fetch `session` from its shard, at `min_index` or later.
    pub async fn refresh(&self, session: SessionId, min_index: u64) -> anyhow::Result<()> {
        let shard = self.shard_of(session)?;
        let mut since = self.since(session);
        for _ in 0..2 {
            let delta = self.fetch(shard, session, since, min_index, 0, 0).await?;
            if self.absorb(session, shard, delta) {
                return Ok(());
            }
            since = None;
        }
        anyhow::bail!("mirror of {session} could not catch up")
    }

    async fn fetch(
        &self,
        shard: ShardId,
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
            shard,
            "/shard/exec/view",
            &body,
            ATTEMPT + Duration::from_millis(wait_ms),
        )
        .await
        .map_err(|e| anyhow::anyhow!("view of {session} in shard {shard} at {min_index}: {e:#}"))
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

    /// Wait (up to `wait`) for `session` to change in its shard past what the
    /// mirror holds, and fold the change in. `true` when it changed.
    pub async fn watch(&self, session: SessionId, wait: Duration) -> anyhow::Result<bool> {
        let shard = self.shard_of(session)?;
        let after = self.applied.lock().get(&session).copied().unwrap_or(0);
        let since = self.since(session);
        let delta = self
            .fetch(shard, session, since, 0, after, wait.as_millis() as u64)
            .await?;
        let changed = delta.applied > after;
        if !self.absorb(session, shard, delta) {
            self.refresh(session, 0).await?;
        }
        Ok(changed)
    }

    /// Offer `credits` slots to `shard` and take the sessions it hands out.
    pub async fn pull(
        &self,
        shard: ShardId,
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
                .post(shard, "/shard/exec/pull", &body, wait + ATTEMPT)
                .await?;
            match reply.leader {
                Some(leader) if reply.sessions.is_empty() => {
                    self.leaders.lock().insert(shard, leader);
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
        let Ok(shard) = self.shard_of(batch.session) else {
            return;
        };
        let seeds = self.targets(shard);
        if self.deliver.send(&self.http, shard, &seeds, batch).await {
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
        remote.bind(id, 0);
        let mut shard = ShardState::default();
        shard.sessions.insert(id, session(id, 1..=5, 0));
        assert!(remote.absorb(id, 0, delta(&shard, 10, id, None)));
        shard.sessions.insert(id, session(id, 1..=6, 0));
        // The reply to a renewal brings the new input with it.
        assert!(remote.absorb(id, 0, delta(&shard, 12, id, Some(5))));
        assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 2);
        // An older reply that arrives late changes nothing.
        assert!(remote.absorb(id, 0, delta(&shard, 11, id, Some(5))));
        assert_eq!(heard.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(
            remote.shared.state.read().sessions[&id].last_sequence(),
            Some(6)
        );
    }

    /// A reply from the shard a session left that lands after the session
    /// was bound to its new shard changes nothing. Found by load run 5: the
    /// old shard's long poll returned after the move with its own, higher
    /// log index, so every reply from the new shard looked late, the mirror
    /// kept saying "moved", and the turn retried for good.
    #[test]
    fn a_late_reply_from_the_shard_a_session_left_is_ignored() {
        let remote = Remote::new(
            crate::remote::EXECUTOR_IDS,
            crate::store::Store::open("memory://").unwrap(),
        )
        .unwrap();
        let id = SessionId::new();
        let mut old = ShardState::default();
        old.sessions.insert(id, session(id, 1..=5, 0));
        remote.bind(id, 0);
        assert!(remote.absorb(id, 0, delta(&old, 900, id, None)));
        // Moved: shard 1 holds it now, at a lower index of its own log.
        assert!(remote.bind(id, 1));
        let mut new = ShardState::default();
        new.sessions.insert(id, session(id, 1..=6, 0));
        assert!(remote.absorb(id, 1, delta(&new, 40, id, None)));
        // The old shard's long poll returns now: the session left it.
        old.sessions.remove(&id);
        old.forwarded.insert(
            id,
            Left {
                shard: 1,
                generation: 2,
            },
        );
        assert!(remote.absorb(id, 0, delta(&old, 950, id, Some(5))));
        // The new shard's next reply still lands.
        new.sessions.insert(id, session(id, 1..=7, 0));
        assert!(remote.absorb(id, 1, delta(&new, 41, id, Some(6))));
        let state = remote.shared.state.read();
        assert_eq!(state.sessions[&id].last_sequence(), Some(7));
        assert!(!state.forwarded.contains_key(&id));
    }

    #[test]
    fn a_delta_appends_only_new_events() {
        let id = SessionId::new();
        let mut shard = ShardState::default();
        shard.sessions.insert(id, session(id, 1..=5, 0));
        let mut mirror = ShardState::default();
        assert!(merge(&mut mirror, id, delta(&shard, 10, id, None)));
        shard.sessions.insert(id, session(id, 1..=8, 0));
        let partial = delta(&shard, 12, id, Some(5));
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
        let mut mirror = ShardState::default();
        let mut shard = ShardState::default();
        shard.sessions.insert(id, session(id, 1..=6, 0));
        assert!(merge(&mut mirror, id, delta(&shard, 1, id, None)));
        shard.sessions.insert(id, session(id, 5..=7, 4));
        assert!(merge(&mut mirror, id, delta(&shard, 2, id, Some(6))));
        let held = &mirror.sessions[&id];
        assert_eq!(held.offloaded, 4);
        assert_eq!(held.events.first().and_then(|e| e.sequence), Some(5));
        assert_eq!(held.last_sequence(), Some(7));
    }

    #[test]
    fn a_gap_asks_for_the_whole_session() {
        let id = SessionId::new();
        let mut mirror = ShardState::default();
        let mut shard = ShardState::default();
        shard.sessions.insert(id, session(id, 1..=3, 0));
        assert!(merge(&mut mirror, id, delta(&shard, 1, id, None)));
        shard.sessions.insert(id, session(id, 1..=9, 0));
        // The executor claims to hold 6, but holds only 3.
        assert!(!merge(&mut mirror, id, delta(&shard, 2, id, Some(6))));
        // A partial view of a session the mirror never held.
        let mut other = ShardState::default();
        assert!(!merge(&mut other, id, delta(&shard, 2, id, Some(2))));
    }

    #[test]
    fn a_restored_session_is_fetched_whole_again() {
        let id = SessionId::new();
        let mut mirror = ShardState::default();
        let mut shard = ShardState::default();
        shard.sessions.insert(id, session(id, 1..=9, 0));
        assert!(merge(&mut mirror, id, delta(&shard, 1, id, None)));
        // Taken back to 5 on a new branch: the mirror's 6..=9 are not its
        // history any more, so a partial view does not join up.
        let mut restored = session(id, 1..=6, 0);
        restored.branch = 7;
        shard.sessions.insert(id, restored);
        assert!(!merge(&mut mirror, id, delta(&shard, 2, id, Some(9))));
        assert!(merge(&mut mirror, id, delta(&shard, 3, id, None)));
        assert_eq!(mirror.sessions[&id].last_sequence(), Some(6));
    }

    #[test]
    fn a_session_that_left_leaves_the_mirror() {
        let id = SessionId::new();
        let mut mirror = ShardState::default();
        let mut shard = ShardState::default();
        shard.sessions.insert(id, session(id, 1..=2, 0));
        assert!(merge(&mut mirror, id, delta(&shard, 1, id, None)));
        shard.sessions.remove(&id);
        shard.forwarded.insert(
            id,
            Left {
                shard: 3,
                generation: 2,
            },
        );
        assert!(merge(&mut mirror, id, delta(&shard, 2, id, Some(2))));
        assert!(!mirror.sessions.contains_key(&id));
        assert_eq!(mirror.forwarded[&id].shard, 3);
    }
}

/// `POST /shard/exec/view`: the session, once the node applied `min_index`;
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
        "/shard/exec/propose" => "propose",
        "/shard/exec/view" => "view",
        "/shard/exec/pull" => "pull",
        _ => "other",
    }
}

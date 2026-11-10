//! The cell as an Everruns Engine hosting profile.
//!
//! [`RaftHosting`] implements Engine's `SessionHosting`: every authority
//! operation (create, admit, claim, renew, release, append, receipt, ...) is
//! one Raft command, applied by `authority.rs` on every replica. Engine owns
//! everything about the agent (turns, inbox semantics, recovery, waits,
//! lifecycle hooks); the cell owns replication, placement and moves.
//!
//! Reads come from this node's replica. A node that just wrote to a session
//! (an admission, an append) first waits until its replica applied that
//! write, so reads see their own writes; reads of sessions this node did not
//! write may trail the leader by a heartbeat.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock, Weak};
use std::time::Duration;

use async_trait::async_trait;
use everruns::hosting::spi::actor::{
    ActorFence, ActorJournal, ActorRelease, ActorRenewal, LeasedActor, SessionActorStore,
    actor_lease_lost,
};
use everruns::hosting::spi::session_commands::SessionCommand;
use everruns::hosting::{
    AdmissionReceipt, AgentBuild, AgentDefinition, AgentDefinitions, HostingStatus, RuntimeAgent,
    SessionActivation, SessionBinding, SessionEpoch, SessionHosting, SessionQuerySnapshot,
};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{EventId, MessageId, SessionId};
use everruns_core::events::{Event, EventRequest};
use everruns_core::host::{
    EventCursor, EventDurability, EventLog, EventLogError, EventPage, EventReadRequest,
    EventReader, HostBackends, RuntimeSessionStore, SessionMutator,
};
use everruns_core::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
use everruns_core::{
    CompactionCheckpoint, CompactionCheckpointStore, ExecutionSession, ProactiveCompactionAttempt,
};
use serde_json::Value;

use crate::authority::{ProactiveAttempt, WriteOp};
use crate::node::{Node, now_ms};
use crate::state::{Checkpoint, Command, Response, SessionState};

/// The agent behavior version sessions are bound to. Decision: this is NOT
/// the binary's version. Engine's `AgentBuild::build_id` names retained Agent
/// behavior "independent of deployment revisions", and a session's behavior
/// here is its client-supplied definition, already content-addressed by the
/// definition key and replicated with the session. Any node of any deploy can
/// rebuild it, so a rolling deploy needs no build-aware dispatch and suspends
/// nothing. Bump this only for an incompatible change in how a definition is
/// turned into an Agent, and ship a resolver for the old value with it.
pub const BUILD_ID: &str = "definition-v1";

/// The build every session of this host runs as: the definition's key, and
/// the definition format (`build_id`) that turns it into an Agent.
pub fn build_for(definition: &Value, build_id: &str) -> everruns::hosting::AgentBuild {
    everruns::hosting::AgentBuild::new(crate::state::definitions::key(definition), build_id)
}

struct Inner {
    node: Weak<Node>,
    /// Per session, the newest log index this node wrote: reads wait for it.
    fences: parking_lot::Mutex<HashMap<SessionId, u64>>,
    /// Per session, Engine's change hint (`SessionHosting::changes`).
    hints: parking_lot::Mutex<HashMap<SessionId, Weak<tokio::sync::Notify>>>,
    /// Per session, the hand-off flag of the epoch running here.
    hand_offs: parking_lot::Mutex<HashMap<SessionId, Weak<AtomicBool>>>,
}

/// Engine's session authority, backed by this node's cell.
#[derive(Clone)]
pub struct RaftHosting {
    inner: Arc<Inner>,
}

fn store_err(error: impl std::fmt::Display) -> AgentLoopError {
    AgentLoopError::store(error.to_string())
}

impl RaftHosting {
    pub fn new(node: Weak<Node>) -> Self {
        Self {
            inner: Arc::new(Inner {
                node,
                fences: Default::default(),
                hints: Default::default(),
                hand_offs: Default::default(),
            }),
        }
    }

    /// Wake Engine's waiters on `session` after this replica applied a
    /// change to it; ask a running turn to hand off if the session is
    /// frozen for a move.
    pub fn changed(&self, session: SessionId, frozen: bool) {
        let hint = self
            .inner
            .hints
            .lock()
            .get(&session)
            .and_then(Weak::upgrade);
        if let Some(hint) = hint {
            hint.notify_waiters();
        }
        if frozen
            && let Some(flag) = self
                .inner
                .hand_offs
                .lock()
                .get(&session)
                .and_then(Weak::upgrade)
        {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Wait until this replica applied this node's writes to `session`.
    pub async fn caught_up(&self, session: SessionId) -> Result<()> {
        self.inner.caught_up(session).await.map(|_| ())
    }

    /// Forget hints and flags nobody holds any more.
    pub fn sweep(&self) {
        self.inner.hints.lock().retain(|_, h| h.strong_count() > 0);
        self.inner
            .hand_offs
            .lock()
            .retain(|_, h| h.strong_count() > 0);
        let node = self.inner.node.upgrade();
        if let Some(node) = node {
            let applied = *node.shared.applied.borrow();
            self.inner.fences.lock().retain(|_, index| *index > applied);
        }
    }

    fn ports(&self, session: SessionId, epoch: Option<Epoch>) -> Ports {
        Ports {
            inner: self.inner.clone(),
            session,
            epoch: epoch.map(Arc::new),
        }
    }
}

impl Inner {
    fn node(&self) -> Result<Arc<Node>> {
        self.node
            .upgrade()
            .ok_or_else(|| AgentLoopError::store("node stopped"))
    }

    async fn propose(&self, session: SessionId, command: Command) -> Result<Response> {
        let node = self.node()?;
        let t = std::time::Instant::now();
        let (response, index) = node.propose(command).await.map_err(store_err)?;
        crate::prof::add("propose", t, 1);
        let mut fences = self.fences.lock();
        let fence = fences.entry(session).or_insert(0);
        *fence = (*fence).max(index);
        Ok(response)
    }

    /// This replica, once it has applied this node's writes to `session`.
    async fn caught_up(&self, session: SessionId) -> Result<Arc<Node>> {
        let node = self.node()?;
        let index = self.fences.lock().get(&session).copied().unwrap_or(0);
        if index > 0 {
            let t = std::time::Instant::now();
            let _g = scopeguard(move || crate::prof::add("catchup", t, 1));
            tokio::time::timeout(Duration::from_secs(10), node.shared.wait_applied(index))
                .await
                .map_err(|_| AgentLoopError::store("replica did not catch up"))?;
        }
        Ok(node)
    }

    async fn read<T>(&self, session: SessionId, f: impl FnOnce(&SessionState) -> T) -> Result<T> {
        let node = self.caught_up(session).await?;
        let state = node.shared.state.read();
        match state.sessions.get(&session) {
            Some(s) => Ok(f(s)),
            None => Err(missing(&state, session)),
        }
    }
}

/// Why a session is not here, as an error a gateway can act on.
fn missing(state: &crate::state::CellState, session: SessionId) -> AgentLoopError {
    match state.forwarded.get(&session) {
        Some(cell) => AgentLoopError::store(format!("{MOVED}{cell}")),
        None => AgentLoopError::SessionNotFound(session),
    }
}

/// Prefix of the error a command gets for a session that moved away.
pub const MOVED: &str = "cell: session moved to cell ";
/// The error a command gets for a session frozen for a move.
pub const FROZEN: &str = "cell: session frozen for a move";

fn refused(session: SessionId, response: Response) -> AgentLoopError {
    match response {
        Response::Moved { cell } => AgentLoopError::store(format!("{MOVED}{cell}")),
        Response::Frozen => AgentLoopError::store(FROZEN),
        Response::NotFound => AgentLoopError::SessionNotFound(session),
        Response::Rejected { reason } => AgentLoopError::config(reason),
        other => AgentLoopError::store(format!("unexpected reply {other:?}")),
    }
}

fn status_of(s: &SessionState) -> Result<HostingStatus> {
    let a = &s.authority;
    let now = now_ms();
    Ok(HostingStatus {
        executing: a.executing(now),
        now_ms: now,
        wake_at: a.wake_at,
        created_at: a.created_at.clone(),
        binding: a
            .binding
            .clone()
            .ok_or_else(|| AgentLoopError::store("session has no binding"))?,
        runnable: a.runnable,
        suspended: a.suspended,
        suspension_reason: if a.suspended {
            a.records
                .get("suspension")
                .and_then(|value| value["reason"].as_str().map(str::to_owned))
        } else {
            None
        },
        open_turn: a.open,
        waiting: a.pending.clone(),
    })
}

/// Definitions travel with their sessions: a session is created with its
/// agent's definition, which the cell replicates and interns by key
/// (`state::definitions`). Publishing is a no-op for that reason; a lookup
/// answers for the definition format this host reads.
#[async_trait]
impl AgentDefinitions for RaftHosting {
    async fn publish_definition(&self, _definition: &AgentDefinition) -> Result<()> {
        Ok(())
    }

    async fn definition(&self, build: &AgentBuild) -> Result<Option<AgentDefinition>> {
        if build.build_id != BUILD_ID {
            return Ok(None);
        }
        Ok(
            crate::state::definitions::lookup(&build.agent).map(|definition| {
                AgentDefinition::new(
                    build.agent.clone(),
                    BUILD_ID,
                    RuntimeAgent::new((*definition).clone()),
                )
            }),
        )
    }
}

#[async_trait]
impl SessionHosting for RaftHosting {
    fn definitions(&self) -> Option<&dyn AgentDefinitions> {
        Some(self)
    }

    async fn command(&self, session: SessionId, command: SessionCommand) -> Result<Value> {
        let definition = match &command {
            SessionCommand::Create { binding } => {
                let binding: SessionBinding =
                    serde_json::from_value(binding.clone()).map_err(store_err)?;
                // A build whose definition this node does not hold is still
                // created (Engine suspends it at activation if no node can
                // build it).
                crate::state::definitions::lookup(&binding.behavior.agent)
                    .map(|definition| (*definition).clone())
            }
            _ => None,
        };
        let response = self
            .inner
            .propose(
                session,
                Command::Host {
                    session,
                    command,
                    definition,
                    now_ms: now_ms(),
                    seed: EventId::new(),
                },
            )
            .await?;
        match response {
            Response::Value { value } => Ok(value),
            other => Err(refused(session, other)),
        }
    }

    fn changes(&self, session: SessionId) -> Option<Arc<tokio::sync::Notify>> {
        let mut hints = self.inner.hints.lock();
        if let Some(hint) = hints.get(&session).and_then(Weak::upgrade) {
            return Some(hint);
        }
        let hint = Arc::new(tokio::sync::Notify::new());
        hints.insert(session, Arc::downgrade(&hint));
        Some(hint)
    }

    async fn query_snapshot(&self, session: SessionId) -> Result<SessionQuerySnapshot> {
        self.inner
            .read(session, |s| {
                let a = &s.authority;
                Ok(SessionQuerySnapshot {
                    configuration: serde_json::from_value(a.configuration()).map_err(store_err)?,
                    binding: a
                        .binding
                        .clone()
                        .ok_or_else(|| AgentLoopError::store("session has no binding"))?,
                    high_watermark: s.last_sequence().unwrap_or(0),
                    execution: a
                        .execution
                        .clone()
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(store_err)?,
                    rejected_inputs: a
                        .records
                        .keys()
                        .filter_map(|key| key.strip_prefix("rejected/"))
                        .filter_map(|key| key.parse().ok())
                        .collect(),
                })
            })
            .await?
    }

    async fn status(&self, session: SessionId) -> Result<HostingStatus> {
        self.inner.read(session, status_of).await?
    }

    async fn receipt(
        &self,
        session: SessionId,
        message: MessageId,
    ) -> Result<Option<AdmissionReceipt>> {
        self.inner
            .read(session, |s| {
                s.authority
                    .commands
                    .get(&message.to_string())
                    .map(|receipt| serde_json::from_value(receipt.value.clone()).map_err(store_err))
                    .transpose()
            })
            .await?
    }

    fn reader(&self, session: SessionId) -> HostBackends {
        self.ports(session, None).backends()
    }

    async fn record(&self, session: SessionId, key: &str) -> Result<Option<Value>> {
        self.inner
            .read(session, |s| s.authority.records.get(key).cloned())
            .await
    }

    async fn claim(
        &self,
        session: SessionId,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<SessionActivation>> {
        let node = self.inner.node()?;
        let response = self
            .inner
            .propose(
                session,
                Command::Claim {
                    session,
                    holder: holder.to_owned(),
                    node: node.cfg.node_id,
                    ttl_ms: ttl.as_millis() as i64,
                    now_ms: now_ms(),
                },
            )
            .await?;
        let value = match response {
            Response::Value { value } if value.is_null() => return Ok(None),
            Response::Value { value } => value,
            Response::Frozen => return Ok(None),
            other => return Err(refused(session, other)),
        };
        let fence = value["fence"].as_u64().unwrap_or(0);
        let actor = LeasedActor {
            session_id: session,
            org_id: 1,
            fence,
            inbox_seq: value["inbox_seq"].as_i64().unwrap_or(0),
            cancel_inbox_seq: value["cancel_inbox_seq"].as_i64(),
            consumed_seq: value["consumed_seq"].as_i64().unwrap_or(0),
            next_input_seq: value["next_input_seq"].as_i64().unwrap_or(0),
            attempts: value["attempts"].as_u64().unwrap_or(1) as u32,
        };
        let (binding, frozen) = self
            .inner
            .read(session, |s| {
                (
                    s.authority.binding.clone(),
                    s.placement == crate::state::Placement::Frozen,
                )
            })
            .await?;
        let binding = binding.ok_or_else(|| AgentLoopError::store("session has no binding"))?;
        let hand_off = Arc::new(AtomicBool::new(frozen));
        self.inner
            .hand_offs
            .lock()
            .insert(session, Arc::downgrade(&hand_off));
        let epoch = Epoch {
            fence,
            actor_fence: RwLock::new(ActorFence::new(session, holder, fence)),
            hand_off,
            stored: parking_lot::Mutex::new(None),
            pipe: Pipe::start(self.inner.clone(), session),
        };
        Ok(Some(SessionActivation {
            binding,
            actor,
            epoch: Arc::new(self.ports(session, Some(epoch))),
        }))
    }
}

/// One activation's write capability.
struct Epoch {
    fence: u64,
    actor_fence: RwLock<ActorFence>,
    hand_off: Arc<AtomicBool>,
    /// Stored history this activation already paged in (sparse: context
    /// events before `full_from`). Stored events never change, so one
    /// activation reads each segment once however often Engine asks.
    stored: parking_lot::Mutex<Option<StoredView>>,
    /// This epoch's writes, in order (see [`Pipe`]).
    pipe: Pipe,
}

/// One epoch's writes to the cell, in order, a batch per round trip.
///
/// Decision: an Engine turn makes about 23 authority writes one after
/// another (each event, the turn records, schedules, renewals, the
/// release), and each waited for its own Raft commit: most of a turn's
/// time on a cell. The epoch is its session's only writer, so most
/// lifecycle events need not wait: the epoch predicts their sequence,
/// returns at once, and sends them with whatever follows in one ordered
/// `Chain`. Everything that needs its answer or must be durable first
/// (terminal and tool events, `turn.started`, inputs, records, renewals,
/// the release) waits for its chain, so a write is acknowledged only once
/// every earlier one is committed, and an effect never runs ahead of the
/// events before it. Engine's emitter contract allows this: a host may
/// store lifecycle events in the background, never the `tool.started` of
/// an at-most-once call (tool events always wait here). A chain stops at
/// its first failure; a failed background append (an input admitted
/// meanwhile took the predicted sequence, or the epoch was superseded)
/// fails every later write of the epoch, and Engine resumes the turn from
/// what is durable, as after a crash.
struct Pipe {
    tx: tokio::sync::mpsc::UnboundedSender<Op>,
    state: Arc<parking_lot::Mutex<PipeState>>,
    /// Held while predicting a sequence and queueing, so queue order is
    /// sequence order.
    order: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct PipeState {
    /// Background appends not yet seen applied on this replica.
    pending: Vec<Event>,
    /// The sequence the next append of this epoch gets, when known.
    next: Option<i32>,
    failed: Option<String>,
}

struct Op {
    command: Command,
    reply: Option<tokio::sync::oneshot::Sender<Result<Response>>>,
}

impl Pipe {
    fn start(inner: Arc<Inner>, session: SessionId) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Op>();
        let state = Arc::new(parking_lot::Mutex::new(PipeState::default()));
        let shared = state.clone();
        tokio::spawn(async move {
            while let Some(first) = rx.recv().await {
                let mut ops = vec![first];
                while ops.len() < 128 {
                    match rx.try_recv() {
                        Ok(op) => ops.push(op),
                        Err(_) => break,
                    }
                }
                let failed = shared.lock().failed.clone();
                if let Some(reason) = failed {
                    for op in ops {
                        if let Some(reply) = op.reply {
                            let _ = reply.send(Err(AgentLoopError::store(reason.clone())));
                        }
                    }
                    continue;
                }
                let (commands, replies): (Vec<_>, Vec<_>) =
                    ops.into_iter().map(|op| (op.command, op.reply)).unzip();
                let result = inner.propose(session, Command::Chain { commands }).await;
                let responses = match result {
                    Ok(Response::Batch { responses }) if responses.len() == replies.len() => {
                        responses
                    }
                    Ok(other) => {
                        let reason = format!("unexpected chain reply: {other:?}");
                        shared.lock().failed = Some(reason.clone());
                        for reply in replies.into_iter().flatten() {
                            let _ = reply.send(Err(AgentLoopError::store(reason.clone())));
                        }
                        continue;
                    }
                    Err(error) => {
                        let reason = error.to_string();
                        shared.lock().failed = Some(reason.clone());
                        for reply in replies.into_iter().flatten() {
                            let _ = reply.send(Err(AgentLoopError::store(reason.clone())));
                        }
                        continue;
                    }
                };
                for (reply, response) in replies.into_iter().zip(responses) {
                    match reply {
                        Some(reply) => {
                            let _ = reply.send(Ok(response));
                        }
                        None if !matches!(response, Response::Value { .. }) => {
                            let mut state = shared.lock();
                            if state.failed.is_none() {
                                state.failed =
                                    Some(format!("a background append failed: {response:?}"));
                            }
                        }
                        None => {}
                    }
                }
            }
        });
        Self {
            tx,
            state,
            order: tokio::sync::Mutex::new(()),
        }
    }

    fn failure(&self) -> Option<String> {
        self.state.lock().failed.clone()
    }
}

/// Whether an append may go in the background: a lifecycle event no reader
/// of the authority depends on before the next waited write, and one the
/// reducer stores unchanged.
fn background(request: &EventRequest) -> bool {
    use everruns_core::events::EventData;
    let annotated = request
        .metadata
        .as_ref()
        .and_then(Value::as_object)
        .is_some_and(|m| m.contains_key("everruns.human_waits"));
    !annotated
        && matches!(
            request.data,
            EventData::SessionActivated(_)
                | EventData::SessionIdled(_)
                | EventData::ReasonStarted(_)
                | EventData::ReasonCompleted(_)
                | EventData::OutputMessageStarted(_)
                | EventData::OutputMessageCompleted(_)
                | EventData::LlmGeneration(_)
                | EventData::CapabilityUsage(_)
        )
}

struct StoredView {
    from: i32,
    to: i32,
    full_from: i32,
    events: Vec<Event>,
}

/// Engine's ports for one session: fenced by an epoch, or read-only.
#[derive(Clone)]
struct Ports {
    inner: Arc<Inner>,
    session: SessionId,
    epoch: Option<Arc<Epoch>>,
}

impl Ports {
    fn scope(&self, session: SessionId) -> Result<()> {
        if session != self.session {
            return Err(AgentLoopError::config("cross-session authority access"));
        }
        Ok(())
    }

    fn epoch(&self) -> Result<&Epoch> {
        self.epoch
            .as_deref()
            .ok_or_else(|| AgentLoopError::config("read-only session ports"))
    }

    fn lost(&self) -> AgentLoopError {
        let error = actor_lease_lost(self.session, "superseded by a newer epoch");
        if let Some(epoch) = &self.epoch {
            let fence = epoch.actor_fence.read().unwrap_or_else(|p| p.into_inner());
            fence.observe::<()>(&Err(actor_lease_lost(self.session, "ownership lost")));
        }
        error
    }

    /// Propose `command` after this epoch's earlier writes (read-only
    /// ports: directly), and wait for its reply.
    async fn order(&self, command: Command) -> Result<Response> {
        let Some(epoch) = self.epoch.as_deref() else {
            return self.inner.propose(self.session, command).await;
        };
        let (reply, answer) = tokio::sync::oneshot::channel();
        {
            let _order = epoch.pipe.order.lock().await;
            if let Some(reason) = epoch.pipe.failure() {
                return Err(AgentLoopError::store(reason));
            }
            let append = matches!(command, Command::Append { .. });
            epoch
                .pipe
                .tx
                .send(Op {
                    command,
                    reply: Some(reply),
                })
                .map_err(|_| AgentLoopError::store("epoch writer stopped"))?;
            if append {
                // Its sequence is the authority's to assign; predict again
                // from its reply.
                epoch.pipe.state.lock().next = None;
            }
        }
        answer
            .await
            .map_err(|_| AgentLoopError::store("epoch writer stopped"))?
    }

    /// Queue a lifecycle event behind this epoch's earlier writes and
    /// return it with the sequence it will get (see [`Pipe`]).
    async fn append_in_background(&self, epoch: &Epoch, request: EventRequest) -> Result<Event> {
        let _order = epoch.pipe.order.lock().await;
        if let Some(reason) = epoch.pipe.failure() {
            return Err(AgentLoopError::store(reason));
        }
        let next = epoch.pipe.state.lock().next;
        let sequence = match next {
            Some(next) => next,
            None => {
                self.inner
                    .read(self.session, |s| s.last_sequence().unwrap_or(0))
                    .await?
                    + 1
            }
        };
        let id = EventId::new();
        let expected = Some((sequence > 1).then_some(sequence - 1));
        let event = request.clone().into_event(id, sequence);
        {
            let mut state = epoch.pipe.state.lock();
            state.pending.push(event.clone());
            state.next = Some(sequence + 1);
        }
        epoch
            .pipe
            .tx
            .send(Op {
                command: Command::Append {
                    session: self.session,
                    fence: epoch.fence,
                    event_id: id,
                    request,
                    expected,
                    now_ms: now_ms(),
                },
                reply: None,
            })
            .map_err(|_| AgentLoopError::store("epoch writer stopped"))?;
        Ok(event)
    }

    /// Background appends of this epoch past `high`, the replica's end.
    fn pending_after(&self, high: i32) -> Vec<Event> {
        let Some(epoch) = self.epoch.as_deref() else {
            return Vec::new();
        };
        let mut state = epoch.pipe.state.lock();
        state
            .pending
            .retain(|e| e.sequence.is_some_and(|seq| seq > high));
        state.pending.clone()
    }

    async fn write(&self, op: WriteOp) -> Result<Value> {
        let fence = self.epoch()?.fence;
        let response = self
            .order(Command::Write {
                session: self.session,
                fence,
                change: op,
                now_ms: now_ms(),
            })
            .await?;
        match response {
            Response::Value { value } => Ok(value),
            Response::Fenced => Err(self.lost()),
            other => Err(refused(self.session, other)),
        }
    }

    async fn append(
        &self,
        request: EventRequest,
        expected: Option<Option<i32>>,
    ) -> std::result::Result<Event, EventLogError> {
        let backend = |detail: String| EventLogError::Backend { detail };
        self.scope(request.session_id)
            .map_err(|e| backend(e.to_string()))?;
        if request.is_ephemeral() {
            return Err(EventLogError::InvalidAppend {
                detail: format!("ephemeral event {} is sink-only", request.event_type),
            });
        }
        let epoch = self.epoch().map_err(|e| backend(e.to_string()))?;
        let fence = epoch.fence;
        let mut request = request;
        elide_prompt(&mut request);
        let started = std::time::Instant::now();
        if expected.is_none() && background(&request) {
            let event = self
                .append_in_background(epoch, request)
                .await
                .map_err(|e| backend(e.to_string()))?;
            if let Ok(node) = self.inner.node() {
                node.prom
                    .events_appended
                    .inc(&[("type", event.event_type.as_str())]);
                node.prom.observe_event(&event);
            }
            return Ok(event);
        }
        let response = self
            .order(Command::Append {
                session: self.session,
                fence,
                event_id: EventId::new(),
                request,
                expected,
                now_ms: now_ms(),
            })
            .await
            .map_err(|e| backend(e.to_string()))?;
        match response {
            Response::Value { value } => {
                let event: Event =
                    serde_json::from_value(value).map_err(|e| backend(e.to_string()))?;
                epoch.pipe.state.lock().next = event.sequence.map(|seq| seq + 1);
                if let Ok(node) = self.inner.node() {
                    node.metrics.append.record(started.elapsed());
                    node.prom.append_seconds.observe(&[], started.elapsed());
                    node.prom
                        .events_appended
                        .inc(&[("type", event.event_type.as_str())]);
                    node.prom.observe_event(&event);
                }
                Ok(event)
            }
            Response::Conflict { expected, actual } => Err(EventLogError::SequenceConflict {
                session_id: self.session,
                expected,
                actual,
            }),
            Response::Fenced => Err(backend(self.lost().to_string())),
            other => Err(backend(refused(self.session, other).to_string())),
        }
    }

    /// Events after `after`, at most `limit + 1`, paged in from the object
    /// store as needed, with the log's last sequence.
    ///
    /// Decision: an executor's ports read earlier turns sparsely. Engine
    /// builds a turn's model input from messages only and resumes a turn
    /// from its own `turn.started` on, so before the current turn an epoch
    /// reads just the context events ([`crate::store::is_context`]): from
    /// the messages-only companion of each stored segment, and filtered
    /// from memory. Model calls, steps and tool lifecycle of old turns
    /// (about four in five events) are never loaded by a turn, so a long
    /// session costs a turn its messages, not its log. Read-only ports
    /// (clients, Engine's frontends) see every event.
    /// Events after `after` (see [`window_applied`](Self::window_applied)),
    /// with this epoch's background appends; `true` when the page stops
    /// short of `limit` with more to read.
    async fn window(&self, after: i32, limit: usize) -> Result<(Vec<Event>, i32, bool)> {
        let (mut events, high, partial) = self.window_applied(after, limit).await?;
        let pending = self.pending_after(high);
        let Some(last) = pending.last().and_then(|e| e.sequence) else {
            return Ok((events, high, partial));
        };
        if partial {
            return Ok((events, high.max(last), true));
        }
        // This epoch's own background appends read as written.
        if events.len() <= limit {
            events.extend(
                pending
                    .into_iter()
                    .filter(|e| e.sequence.is_some_and(|seq| seq > after)),
            );
            events.truncate(limit + 1);
        }
        Ok((events, high.max(last), false))
    }

    async fn window_applied(&self, after: i32, limit: usize) -> Result<(Vec<Event>, i32, bool)> {
        let node = self.inner.caught_up(self.session).await?;
        let sparse = self.epoch.is_some();
        let (offloaded, segments, resident, high, full_from) = {
            let state = node.shared.state.read();
            let s = state
                .sessions
                .get(&self.session)
                .ok_or_else(|| missing(&state, self.session))?;
            let full_from = if sparse { s.turn_from() } else { 1 };
            (
                s.offloaded,
                s.segments.clone(),
                s.resident_after(after)
                    .iter()
                    .filter(|e| {
                        e.sequence.is_some_and(|seq| seq >= full_from)
                            || crate::store::is_context(&e.event_type)
                    })
                    .take(limit + 1)
                    .cloned()
                    .collect::<Vec<_>>(),
                s.last_sequence().unwrap_or(0),
                full_from,
            )
        };
        if after >= offloaded {
            return Ok((resident, high, false));
        }
        let store = &node.shared.store;
        let mut events = if sparse && after + 1 < full_from {
            let epoch = self.epoch()?;
            let cached = {
                let view = epoch.stored.lock();
                view.as_ref()
                    .filter(|v| v.full_from == full_from && v.from <= after + 1)
                    .map(|v| (v.from, v.to, v.events.clone()))
            };
            let (view_from, from, mut events) = match cached {
                Some((view_from, to, events)) => (view_from, to + 1, events),
                None => (after + 1, after + 1, Vec::new()),
            };
            if from <= offloaded {
                // Context events up to the current turn (small), then the
                // current turn's stored events whole.
                let context_to = offloaded.min(full_from - 1);
                // A read near the session's start (Engine checks for
                // `session.started` on every activation) needs only its first
                // events: load context a segment at a time and stop at the
                // first that yields any, returning a short page that says
                // more follows. A stored segment is read whole, and the
                // oldest ones hold most of a long session's history; the
                // first segment stays small (see `compaction_range`).
                let mut next = from;
                while next <= context_to {
                    let end = segments
                        .iter()
                        .find(|seg| seg.from <= next && next <= seg.to)
                        .map_or(context_to, |seg| seg.to.min(context_to));
                    events.extend(
                        store
                            .load_context(self.session, &segments, next, end)
                            .await
                            .map_err(store_err)?,
                    );
                    next = end + 1;
                    let page = events
                        .iter()
                        .filter(|e| e.sequence.is_some_and(|seq| seq > after))
                        .count();
                    if page > 0 && next <= context_to {
                        events.retain(|e| e.sequence.is_some_and(|seq| seq > after));
                        let short = events.len() <= limit;
                        events.truncate(limit + 1);
                        return Ok((events, high, short));
                    }
                }
                let full = from.max(context_to + 1);
                if full <= offloaded {
                    events.extend(
                        store
                            .load(self.session, &segments, full, offloaded)
                            .await
                            .map_err(store_err)?,
                    );
                }
                events.retain(|e| {
                    e.sequence
                        .is_some_and(|seq| seq >= view_from && seq <= offloaded)
                });
                *epoch.stored.lock() = Some(StoredView {
                    from: view_from,
                    to: offloaded,
                    full_from,
                    events: events.clone(),
                });
            }
            events
        } else {
            let to = offloaded.min(after.saturating_add(limit as i32 + 1));
            let mut events = store
                .load(self.session, &segments, after + 1, to)
                .await
                .map_err(store_err)?;
            events.retain(|e| e.sequence.is_some_and(|seq| seq > after && seq <= to));
            if to < offloaded {
                return Ok((events, high, false));
            }
            events
        };
        events.retain(|e| {
            e.sequence
                .is_some_and(|seq| seq > after && seq <= offloaded)
        });
        events.extend(resident);
        events.truncate(limit + 1);
        Ok((events, high, false))
    }
}

/// Drop the copy of the prompt an `llm.generation` event carries.
///
/// Decision: Everruns records each model call with the whole prompt it sent
/// (`messages`, and the provider's `request_body` when captured). That prompt
/// is the session's own history up to the call, so storing it again makes a
/// session's log grow with the square of its length: in the llmsim mix these
/// copies were two thirds of all bytes in memory, in the Raft log and in the
/// object store. Nothing replays from them (resume folds the turn events),
/// so the cell keeps the call's output, tools and metadata and drops the copy.
pub fn elide_prompt(request: &mut EventRequest) {
    if let everruns_core::events::EventData::LlmGeneration(data) = &mut request.data {
        data.messages = Vec::new();
        data.request_body = None;
    }
}

/// A page of `events` (in sequence order, gaps allowed) with the snapshot
/// semantics `EventReader` requires; `current_high` is the log's end.
pub fn read_page_at(
    events: &[Event],
    request: EventReadRequest,
    current_high: i32,
    partial: bool,
) -> std::result::Result<EventPage, EventLogError> {
    let session_id = request.session_id();
    let (after, snapshot) = match request.cursor() {
        Some(cursor) => {
            if cursor.session_id() != session_id {
                return Err(EventLogError::CrossSessionCursor {
                    detail: "cursor belongs to another session".into(),
                });
            }
            let snapshot = cursor.snapshot_high_watermark().unwrap_or(current_high);
            if cursor.after_sequence() > snapshot {
                return Err(EventLogError::IncompatibleCursor {
                    detail: "cursor position exceeds its snapshot".into(),
                });
            }
            if snapshot > current_high {
                return Err(EventLogError::ExpiredCursor {
                    detail: "cursor snapshot is not available on this replica".into(),
                });
            }
            (cursor.after_sequence(), snapshot)
        }
        None => (0, current_high),
    };
    if snapshot == 0 {
        return EventPage::new(Vec::new(), None, 0);
    }
    let limit = request.limit().get();
    let seq = |e: &Event| e.sequence.unwrap_or(0);
    let start = events.partition_point(|e| seq(e) <= after);
    let end = events.partition_point(|e| seq(e) <= snapshot);
    let mut selected: Vec<Event> = events[start.min(end)..end]
        .iter()
        .take(limit + 1)
        .cloned()
        .collect();
    let mut has_more = selected.len() > limit;
    if has_more {
        selected.pop();
    }
    // A short page with more to read (see `Ports::window`).
    if partial
        && !has_more
        && selected
            .last()
            .and_then(|e| e.sequence)
            .is_some_and(|last| last < snapshot)
    {
        has_more = true;
    }
    let next = if has_more {
        let last = selected.last().and_then(|e| e.sequence).unwrap_or(after);
        Some(EventCursor::continuation(session_id, last, snapshot)?)
    } else {
        None
    };
    EventPage::new(selected, next, snapshot)
}

#[async_trait]
impl SessionEpoch for Ports {
    fn hand_off(&self) -> Option<Arc<AtomicBool>> {
        self.epoch.as_ref().map(|e| e.hand_off.clone())
    }
    async fn schedule(&self, wake_at: Option<i64>) -> Result<()> {
        // Nothing waits on a timer's answer, and the release (or any later
        // waited write) commits it first: queue it behind the epoch's
        // writes. A superseded or expired epoch is refused here, from the
        // replica, as it would be at apply.
        let epoch = self.epoch()?;
        let fence = epoch.fence;
        let now = now_ms();
        if !self
            .inner
            .read(self.session, |s| s.authority.current(fence, now))
            .await?
        {
            return Err(self.lost());
        }
        let _order = epoch.pipe.order.lock().await;
        if let Some(reason) = epoch.pipe.failure() {
            return Err(AgentLoopError::store(reason));
        }
        epoch
            .pipe
            .tx
            .send(Op {
                command: Command::Write {
                    session: self.session,
                    fence,
                    change: WriteOp::Schedule { wake_at },
                    now_ms: now,
                },
                reply: None,
            })
            .map_err(|_| AgentLoopError::store("epoch writer stopped"))
    }
    fn backends(&self) -> HostBackends {
        let mut backends = HostBackends::in_memory();
        backends.event_log = Arc::new(self.clone());
        backends.session_store = Arc::new(self.clone());
        backends.storage_store = Arc::new(self.clone());
        backends.compaction_checkpoint_store = Arc::new(self.clone());
        backends
    }
    fn actor_store(&self) -> Arc<dyn SessionActorStore> {
        Arc::new(self.clone())
    }
    fn observe_fence(&self, fence: &ActorFence) {
        if let Some(epoch) = &self.epoch {
            let mut bound = epoch.actor_fence.write().unwrap_or_else(|p| p.into_inner());
            if bound.holder == fence.holder && bound.fence == fence.fence {
                *bound = fence.clone();
            }
        }
    }
    async fn record(&self, key: &str) -> Result<Option<Value>> {
        self.inner
            .read(self.session, |s| s.authority.records.get(key).cloned())
            .await
    }
    async fn records(&self, prefix: &str) -> Result<Vec<(String, Value)>> {
        let records = self
            .inner
            .read(self.session, |s| {
                s.authority
                    .records
                    .range(prefix.to_owned()..)
                    .take_while(|(k, _)| k.starts_with(prefix))
                    .take(4097)
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<Vec<_>>()
            })
            .await?;
        if records.len() > 4096 {
            return Err(AgentLoopError::store(
                "lifecycle receipt scan limit exceeded",
            ));
        }
        Ok(records)
    }
    async fn commit(&self, key: &str, value: Value, absent: bool) -> Result<bool> {
        let reply = self
            .write(WriteOp::Commit {
                key: key.to_owned(),
                value,
                absent,
            })
            .await?;
        Ok(reply.as_bool().unwrap_or(false))
    }
    async fn suspend(&self, reason: &str) -> Result<()> {
        self.write(WriteOp::Suspend {
            reason: reason.to_owned(),
        })
        .await
        .map(|_| ())
    }
}

#[async_trait]
impl EventReader for Ports {
    async fn read_page(
        &self,
        request: EventReadRequest,
    ) -> std::result::Result<EventPage, EventLogError> {
        let backend = |detail: String| EventLogError::Backend { detail };
        self.scope(request.session_id())
            .map_err(|e| backend(e.to_string()))?;
        let after = request.cursor().map_or(0, |c| c.after_sequence());
        let limit = request.limit().get();
        let t = std::time::Instant::now();
        let (events, high, partial) = self
            .window(after, limit)
            .await
            .map_err(|e| backend(e.to_string()))?;
        crate::prof::add("read_page", t, 1);
        crate::prof::add(
            "read_page_events",
            std::time::Instant::now(),
            events.len() as u64,
        );
        read_page_at(&events, request, high, partial)
    }

    async fn last_sequence(
        &self,
        session: SessionId,
    ) -> std::result::Result<Option<i32>, EventLogError> {
        if session != self.session {
            return Ok(None);
        }
        let high = self
            .inner
            .read(session, |s| s.last_sequence().unwrap_or(0))
            .await
            .map_err(|e| EventLogError::Backend {
                detail: e.to_string(),
            })?;
        let pending = self.pending_after(high);
        Ok(Some(
            pending
                .last()
                .and_then(|e| e.sequence)
                .unwrap_or(high)
                .max(high),
        ))
    }
}

#[async_trait]
impl EventLog for Ports {
    async fn append(&self, request: EventRequest) -> std::result::Result<Event, EventLogError> {
        Ports::append(self, request, None).await
    }
    async fn append_conditional(
        &self,
        request: EventRequest,
        expected: Option<i32>,
    ) -> std::result::Result<Event, EventLogError> {
        Ports::append(self, request, Some(expected)).await
    }
    fn durability(&self) -> EventDurability {
        EventDurability::CrashDurable
    }
}

#[async_trait]
impl everruns_core::execution_loading::SessionStore for Ports {
    async fn get_session(&self, session: SessionId) -> Result<Option<ExecutionSession>> {
        self.scope(session)?;
        let (record, configuration) = self
            .inner
            .read(session, |s| {
                (s.authority.execution.clone(), s.authority.configuration())
            })
            .await?;
        let Some(record) = record else {
            return Ok(None);
        };
        let mut record: ExecutionSession = serde_json::from_value(record).map_err(store_err)?;
        let configuration: everruns::hosting::SessionConfiguration =
            serde_json::from_value(configuration).map_err(store_err)?;
        configuration.apply(&mut record);
        Ok(Some(record))
    }
}

#[async_trait]
impl RuntimeSessionStore for Ports {
    async fn add_session(&self, session: ExecutionSession) -> Result<()> {
        self.scope(session.id)?;
        self.write(WriteOp::Execution {
            session: serde_json::to_value(session).map_err(store_err)?,
        })
        .await
        .map(|_| ())
    }
}

#[async_trait]
impl SessionMutator for Ports {
    async fn update_session_title(
        &self,
        session: SessionId,
        title: String,
    ) -> Result<ExecutionSession> {
        self.scope(session)?;
        let record = self.write(WriteOp::Title { title }).await?;
        serde_json::from_value(record).map_err(store_err)
    }
}

fn key_info(key: &str) -> KeyInfo {
    let epoch = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
    KeyInfo {
        key: key.to_owned(),
        created_at: epoch,
        updated_at: epoch,
    }
}

#[async_trait]
impl SessionStorageStore for Ports {
    async fn set_value(&self, session: SessionId, key: &str, value: &str) -> Result<()> {
        self.scope(session)?;
        self.write(WriteOp::SetValue {
            key: key.to_owned(),
            value: value.to_owned(),
        })
        .await
        .map(|_| ())
    }
    async fn get_value(&self, session: SessionId, key: &str) -> Result<Option<String>> {
        self.scope(session)?;
        self.inner
            .read(session, |s| s.authority.values.get(key).cloned())
            .await
    }
    async fn take_value(&self, session: SessionId, key: &str) -> Result<Option<String>> {
        self.scope(session)?;
        let reply = self
            .write(WriteOp::TakeValue {
                key: key.to_owned(),
            })
            .await?;
        Ok(reply.as_str().map(str::to_owned))
    }
    async fn delete_value(&self, session: SessionId, key: &str) -> Result<bool> {
        self.scope(session)?;
        let reply = self
            .write(WriteOp::DeleteValue {
                key: key.to_owned(),
            })
            .await?;
        Ok(reply.as_bool().unwrap_or(false))
    }
    async fn list_keys(&self, session: SessionId) -> Result<Vec<KeyInfo>> {
        self.scope(session)?;
        self.inner
            .read(session, |s| {
                s.authority.values.keys().map(|k| key_info(k)).collect()
            })
            .await
    }
    async fn set_secret(&self, session: SessionId, name: &str, value: &str) -> Result<()> {
        self.scope(session)?;
        self.write(WriteOp::SetSecret {
            key: name.to_owned(),
            value: value.to_owned(),
        })
        .await
        .map(|_| ())
    }
    async fn get_secret(&self, session: SessionId, name: &str) -> Result<Option<String>> {
        self.scope(session)?;
        self.inner
            .read(session, |s| s.authority.secrets.get(name).cloned())
            .await
    }
    async fn delete_secret(&self, session: SessionId, name: &str) -> Result<bool> {
        self.scope(session)?;
        let reply = self
            .write(WriteOp::DeleteSecret {
                key: name.to_owned(),
            })
            .await?;
        Ok(reply.as_bool().unwrap_or(false))
    }
    async fn list_secrets(&self, session: SessionId) -> Result<Vec<SecretInfo>> {
        self.scope(session)?;
        let epoch = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
        self.inner
            .read(session, |s| {
                s.authority
                    .secrets
                    .keys()
                    .map(|name| SecretInfo {
                        name: name.clone(),
                        created_at: epoch,
                        updated_at: epoch,
                    })
                    .collect()
            })
            .await
    }
}

fn to_engine(session: SessionId, c: &Checkpoint) -> CompactionCheckpoint {
    CompactionCheckpoint {
        id: c.id,
        session_id: session,
        source_sequence: c.source_sequence,
        provider_type: c.provider_type.clone(),
        model: c.model.clone(),
        format_version: c.format_version,
        payload: c.payload.clone(),
    }
}

impl Ports {
    async fn latest(
        &self,
        session: SessionId,
        provider_type: &str,
        model: &str,
        format: Option<u32>,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.scope(session)?;
        self.inner
            .read(session, |s| {
                s.checkpoints
                    .iter()
                    .filter(|c| c.provider_type == provider_type && c.model == model)
                    .filter(|c| format.is_none_or(|f| c.format_version == f))
                    .max_by_key(|c| c.source_sequence)
                    .map(|c| to_engine(session, c))
            })
            .await
    }
}

#[async_trait]
impl CompactionCheckpointStore for Ports {
    async fn get_latest(
        &self,
        session: SessionId,
        provider_type: &str,
        model: &str,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.latest(session, provider_type, model, None).await
    }

    async fn get_latest_format(
        &self,
        session: SessionId,
        provider_type: &str,
        model: &str,
        format: u32,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.latest(session, provider_type, model, Some(format))
            .await
    }

    async fn install(&self, checkpoint: CompactionCheckpoint) -> Result<bool> {
        if checkpoint.session_id != self.session {
            return Ok(false);
        }
        let reply = self
            .write(WriteOp::Checkpoint {
                checkpoint: Checkpoint {
                    id: checkpoint.id,
                    source_sequence: checkpoint.source_sequence,
                    provider_type: checkpoint.provider_type,
                    model: checkpoint.model,
                    format_version: checkpoint.format_version,
                    payload: checkpoint.payload,
                },
            })
            .await?;
        Ok(reply.as_bool().unwrap_or(false))
    }

    async fn latest_source_sequence(&self, session: SessionId) -> Result<Option<i64>> {
        if session != self.session {
            return Ok(None);
        }
        self.inner
            .read(session, |s| {
                s.checkpoints.iter().map(|c| c.source_sequence).max()
            })
            .await
    }

    async fn get_proactive_attempt(
        &self,
        session: SessionId,
        provider_type: &str,
        model: &str,
    ) -> Result<Option<ProactiveCompactionAttempt>> {
        self.scope(session)?;
        let key = format!("{provider_type}/{model}");
        self.inner
            .read(session, |s| {
                s.authority
                    .proactive
                    .get(&key)
                    .map(|a| ProactiveCompactionAttempt {
                        source_sequence: a.source_sequence,
                        estimated_input_tokens: a.estimated_input_tokens,
                        input_message_count: a.input_message_count,
                        source_fingerprint: a.source_fingerprint,
                    })
            })
            .await
    }

    async fn record_proactive_attempt(
        &self,
        session: SessionId,
        provider_type: &str,
        model: &str,
        attempt: ProactiveCompactionAttempt,
    ) -> Result<()> {
        self.scope(session)?;
        self.write(WriteOp::Proactive {
            key: format!("{provider_type}/{model}"),
            attempt: ProactiveAttempt {
                source_sequence: attempt.source_sequence,
                estimated_input_tokens: attempt.estimated_input_tokens,
                input_message_count: attempt.input_message_count,
                source_fingerprint: attempt.source_fingerprint,
            },
        })
        .await
        .map(|_| ())
    }
}

struct Guard<F: FnOnce()>(Option<F>);
impl<F: FnOnce()> Drop for Guard<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f()
        }
    }
}
fn scopeguard<F: FnOnce()>(f: F) -> Guard<F> {
    Guard(Some(f))
}

#[path = "hosting_actors.rs"]
mod actors;

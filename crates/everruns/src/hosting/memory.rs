//! Volatile authority. Control, admission, journal writes and receipts share
//! one per-session gate; epochs are checked while that gate is held.
use super::*;
#[path = "memory_command.rs"]
mod commands;
use everruns_contracts::{error::AgentLoopError, rt::Instant, typed_id::TurnId};
use everruns_core::{
    CompactionCheckpoint, CompactionCheckpointStore, ExecutionSession, ProactiveCompactionAttempt,
    events::{Event, EventContext, EventData, EventRequest},
    execution_loading::SessionStore,
    host::{
        EventCursor, EventDurability, EventLog, EventLogError, EventPage, EventReadLimit,
        EventReadRequest, EventReader, InMemoryEventLog, RuntimeSessionStore, SessionMutator,
        session_actor::store::{ActorJournal, ActorRelease, ActorRenewal, actor_lease_lost},
    },
    session_services::{KeyInfo, SecretInfo, SessionStorageStore},
};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, RwLock};

#[cfg(all(test, feature = "builtins", feature = "filesystem"))]
mod dreaming_tests {
    use super::*;

    #[tokio::test]
    async fn completed_experience_can_dream_while_an_idle_epoch_is_leased() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = crate::DreamingConfig::new(crate::DreamDestination {
            mount: "/memory/user".into(),
            path: "/learned".into(),
            instructions: String::new(),
        });
        config.trigger.mode = crate::DreamTriggerMode::Manual;
        config.budget.max_cost_usd_per_day = None;
        let agent = crate::Agent::builder()
            .instructions("Acknowledge")
            .model(crate::Model::simulated("{\"notes\":[]}"))
            .memory_directory("/memory/user", directory.path())
            .capability(config)
            .build()
            .unwrap();
        let profile = MemoryHosting::default();
        let build = AgentBuild::new("dreaming-idle-lease", "v1");
        let engine = crate::Engine::builder()
            .hosting(profile.clone())
            .register(build.clone(), agent)
            .build();
        let session = engine
            .create_registered(SessionId::new(), &build)
            .await
            .unwrap();
        session.send_and_wait("Remember UTC.").await.unwrap();
        let id = session.session_id();
        let deadline = Instant::now() + Duration::from_secs(5);
        while profile.status(id).await.unwrap().executing {
            assert!(Instant::now() < deadline, "foreground did not settle");
            everruns_contracts::rt::sleep(Duration::from_millis(10)).await;
        }
        // Completion receipts can become visible before the actor releases its
        // epoch. Reproduce that authority state without scheduler timing races.
        let authority = profile.session(id);
        {
            let mut control = authority.control.lock().await;
            assert!(control.open.is_none());
            control.holder = Some("finishing-completed-turn".into());
            control.expires = Some(Instant::now() + Duration::from_secs(30));
        }
        assert!(profile.status(id).await.unwrap().executing);
        assert!(engine.dream(id).await.unwrap());
        assert!(!engine.dream(id).await.unwrap());
    }
}

/// A coherent process-lifetime authority. Clones share sessions. There is no
/// claim that accepted work survives losing this process or Wasm isolate.
#[derive(Clone, Default)]
pub struct MemoryHosting {
    dreaming: Arc<everruns_core::engine::dreaming::store::InMemoryDreamingStore>,
    sessions: Arc<Mutex<HashMap<SessionId, Arc<MemorySession>>>>,
    counts: Arc<AuthorityCounters>,
    drop_prompts: bool,
    live_deltas: Option<Arc<dyn super::LiveDeltaSink>>,
}

/// How many operations an execution epoch asked of the authority.
///
/// On a replicated authority (a data plane v2 cell) every one of these is a
/// round trip to the leader, and every write also waits for a quorum, so
/// writes per turn times that latency is the turn's overhead floor. Benches
/// read it to hold that count down; it is diagnostic, not a contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthorityCounts {
    /// Operations that change authority state (journal appends, records,
    /// lease renewals and releases, claims, schedules).
    pub writes: u64,
    /// The writes among them that ran behind the work and that no caller
    /// waited on, so they add to the leader's load but not to the turn's
    /// latency floor (`writes - behind` waited round trips).
    pub behind: u64,
    /// Operations that only read authority state.
    pub reads: u64,
}

#[derive(Default)]
struct AuthorityCounters {
    writes: std::sync::atomic::AtomicU64,
    behind: std::sync::atomic::AtomicU64,
    reads: std::sync::atomic::AtomicU64,
}

#[derive(Clone, Copy)]
enum Op {
    Write,
    /// A write already counted, that its caller does not wait on.
    Behind,
    Read,
}

impl AuthorityCounters {
    fn count(&self, op: Op) {
        let counter = match op {
            Op::Write => &self.writes,
            Op::Behind => &self.behind,
            Op::Read => &self.reads,
        };
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

struct MemorySession {
    id: SessionId,
    renew_poke: Arc<tokio::sync::Notify>,
    /// [`SessionHosting::changes`]: signalled after every receipt or control write.
    changed: Arc<tokio::sync::Notify>,
    control: tokio::sync::Mutex<Control>,
    log: InMemoryEventLog,
    services: HostBackends,
    counts: Arc<AuthorityCounters>,
    drop_prompts: bool,
}

#[derive(Default)]
struct Control {
    wake_at: Option<i64>,
    created_at: String,
    binding: Option<SessionBinding>,
    inbox: i64,
    consumed: i64,
    next_input: i64,
    holder: Option<String>,
    fence: u64,
    expires: Option<Instant>,
    attempts: u32,
    cancel: Option<i64>,
    cancel_reported: bool,
    resume_generation: u64,
    resume_claimed: u64,
    runnable: bool,
    suspended: bool,
    open: Option<TurnId>,
    /// Sequence of the latest `turn.started`, where an actor's tail read
    /// starts, so it does not scan the log from its beginning.
    last_turn_started: Option<i32>,
    pending: Vec<crate::ToolCall>,
    human_waits: everruns_core::human_wait::HumanWaitState,
    finalizing: HashSet<TurnId>,
    /// Ordered, so a prefix scan reads only its range: a session keeps a
    /// receipt or two per turn, and a turn scans prefixes.
    records: std::collections::BTreeMap<String, Value>,
    pending_inputs: std::collections::VecDeque<i64>,
    commands: HashMap<String, super::spi::session_commands::CommandReceipt>,
    open_input: Option<MessageId>,
    has_history: bool,
    target_cancels: HashMap<MessageId, i64>,
    terminal: HashMap<TurnId, Event>,
}

impl MemoryHosting {
    /// Keep `llm.generation` events without the prompt they carry (its
    /// messages and captured request body); the rest of the event stays.
    ///
    /// Decision: a data plane v2 cell keeps full prompts in S3 objects, off
    /// the replicated authority, and no turn reads them back. This lets a
    /// long run on this authority hold what a cell holds, so a 200 000-turn
    /// bench fits in memory. Listeners see the event as stored. Applies to
    /// sessions created after the call.
    pub fn without_prompts(mut self) -> Self {
        self.drop_prompts = true;
        self
    }

    /// Send every epoch's live deltas to `sink`, as a host whose watchers
    /// are in another process would (see [`super::LiveDelta`]).
    pub fn with_live_deltas(mut self, sink: impl super::LiveDeltaSink) -> Self {
        self.live_deltas = Some(Arc::new(sink));
        self
    }

    /// Authority operations the execution epochs of every session have made
    /// so far (see [`AuthorityCounts`]).
    pub fn authority_counts(&self) -> AuthorityCounts {
        use std::sync::atomic::Ordering::Relaxed;
        AuthorityCounts {
            writes: self.counts.writes.load(Relaxed),
            behind: self.counts.behind.load(Relaxed),
            reads: self.counts.reads.load(Relaxed),
        }
    }
    fn new_session(&self, id: SessionId) -> Arc<MemorySession> {
        let mut services = HostBackends::in_memory();
        services.dreaming_store = self.dreaming.clone();
        Arc::new(MemorySession {
            id,
            renew_poke: Arc::default(),
            changed: Arc::default(),
            control: tokio::sync::Mutex::new(Control::default()),
            log: InMemoryEventLog::new(),
            services,
            counts: self.counts.clone(),
            drop_prompts: self.drop_prompts,
        })
    }
    fn session(&self, id: SessionId) -> Arc<MemorySession> {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
            .cloned()
            .unwrap_or_else(|| self.new_session(id))
    }
    fn ports(&self, id: SessionId) -> Ports {
        Ports {
            session: self.session(id),
            fence: Arc::new(RwLock::new(None)),
            claimed_wake_at: None,
            riding: Arc::default(),
            live_deltas: None,
        }
    }
}

impl Control {
    fn binding(&self, id: SessionId) -> Result<SessionBinding> {
        self.binding
            .clone()
            .ok_or(AgentLoopError::SessionNotFound(id))
    }
    fn owns(&self, holder: &str, fence: u64) -> bool {
        self.holder.as_deref() == Some(holder)
            && self.fence == fence
            && self.expires.is_some_and(|expires| expires > Instant::now())
    }
}

#[async_trait]
impl SessionHosting for MemoryHosting {
    async fn command(
        &self,
        session: SessionId,
        command: super::spi::session_commands::SessionCommand,
    ) -> Result<Value> {
        let reply = self.execute_command(session, command).await;
        if reply.is_ok() {
            self.session(session).changed.notify_waiters();
        }
        reply
    }

    fn changes(&self, session: SessionId) -> Option<Arc<tokio::sync::Notify>> {
        Some(self.session(session).changed.clone())
    }

    async fn query_snapshot(&self, id: SessionId) -> Result<SessionQuerySnapshot> {
        use everruns_core::host::{EventReadLimit, EventReadRequest};
        let session = self.session(id);
        let state = session.control.lock().await;
        let binding = state.binding(id)?;
        let page = session
            .log
            .read_page(EventReadRequest::new(
                id,
                EventReadLimit::new(1).map_err(core_error)?,
            ))
            .await
            .map_err(core_error)?;
        let execution = session.services.session_store.get_session(id).await?;
        Ok(SessionQuerySnapshot {
            configuration: serde_json::from_value(
                state
                    .records
                    .get("configuration")
                    .cloned()
                    .unwrap_or(json!({})),
            )
            .map_err(core_error)?,
            binding,
            high_watermark: page.snapshot_high_watermark(),
            execution,
            rejected_inputs: state
                .records
                .keys()
                .filter_map(|key| key.strip_prefix("rejected/"))
                .map(|key| {
                    key.parse()
                        .map_err(|_| AgentLoopError::store("invalid rejection receipt identity"))
                })
                .collect::<Result<_>>()?,
        })
    }
    async fn status(&self, id: SessionId) -> Result<HostingStatus> {
        let session = self.session(id);
        session.counts.count(Op::Read);
        let state = session.control.lock().await;
        Ok(HostingStatus {
            executing: state.holder.is_some()
                && state.expires.is_some_and(|at| at > Instant::now()),
            now_ms: everruns_contracts::rt::utc_now().timestamp_millis(),
            wake_at: state.wake_at,
            created_at: state.created_at.clone(),
            binding: state.binding(id)?,
            runnable: state.runnable,
            suspended: state.suspended,
            suspension_reason: if state.suspended {
                state
                    .records
                    .get("suspension")
                    .and_then(|value| value["reason"].as_str().map(str::to_owned))
            } else {
                None
            },
            open_turn: state.open,
            waiting: state.pending.clone(),
        })
    }
    async fn receipt(&self, id: SessionId, message: MessageId) -> Result<Option<AdmissionReceipt>> {
        let session = self.session(id);
        let state = session.control.lock().await;
        state.binding(id)?;
        Ok(state
            .commands
            .get(&message.to_string())
            .map(|receipt| serde_json::from_value(receipt.value.clone()).map_err(core_error))
            .transpose()?)
    }
    fn reader(&self, id: SessionId) -> HostBackends {
        self.ports(id).backends()
    }
    async fn record(&self, id: SessionId, key: &str) -> Result<Option<Value>> {
        SessionEpoch::record(&self.ports(id), key).await
    }
    async fn claim(
        &self,
        id: SessionId,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<SessionActivation>> {
        let session = self.session(id);
        session.counts.count(Op::Write);
        let mut state = session.control.lock().await;
        let binding = state.binding(id)?;
        if state
            .wake_at
            .is_some_and(|at| at <= everruns_contracts::rt::utc_now().timestamp_millis())
        {
            state.wake_at = None;
            state.resume_generation += 1;
            state.runnable = true;
        }
        if !state.runnable
            || state.suspended
            || state
                .expires
                .is_some_and(|expires| expires > Instant::now())
        {
            return Ok(None);
        }
        state.holder = Some(holder.to_owned());
        state.fence += 1;
        state.expires = Some(Instant::now() + ttl);
        state.attempts += 1;
        state.cancel_reported = state.cancel.is_some();
        state.resume_claimed = state.resume_generation;
        let actor = LeasedActor {
            session_id: id,
            org_id: 1,
            fence: state.fence,
            inbox_seq: state.inbox,
            cancel_inbox_seq: state.cancel,
            consumed_seq: state.consumed,
            next_input_seq: state.next_input,
            attempts: state.attempts,
        };
        Ok(Some(SessionActivation {
            binding,
            actor,
            epoch: Arc::new(Ports {
                claimed_wake_at: Some(state.wake_at),
                riding: Arc::default(),
                session: session.clone(),
                fence: Arc::new(RwLock::new(Some(ActorFence::new(id, holder, state.fence)))),
                live_deltas: self.live_deltas.clone(),
            }),
        }))
    }
    fn process_scheduler(&self) -> bool {
        true
    }
}

#[derive(Clone)]
struct Ports {
    session: Arc<MemorySession>,
    fence: Arc<RwLock<Option<ActorFence>>>,
    claimed_wake_at: Option<Option<i64>>,
    /// Receipts that commit with this epoch's next renew or release.
    riding: Arc<std::sync::Mutex<Vec<(String, Value)>>>,
    live_deltas: Option<Arc<dyn super::LiveDeltaSink>>,
}

/// Insert one receipt, with its effect on the control state; `false` when
/// `absent` and the key exists.
fn apply_record(state: &mut Control, key: &str, value: Value, absent: bool) -> Result<bool> {
    if absent && state.records.contains_key(key) {
        return Ok(false);
    }
    if let Some(input) = key
        .strip_prefix("assignment/")
        .and_then(|key| key.parse::<MessageId>().ok())
        && let Some(position) = state.target_cancels.get(&input).copied()
    {
        let assignment: super::TurnAssignment = serde_json::from_value(value.clone())
            .map_err(|_| AgentLoopError::store("invalid turn assignment"))?;
        if state.open.is_none() || state.open == Some(assignment.turn_id) {
            state.cancel = Some(position);
            state.cancel_reported = false;
            state.runnable = true;
        }
    }
    if let Some(turn) = key
        .strip_prefix("outcome/")
        .and_then(|key| key.parse::<TurnId>().ok())
    {
        state.finalizing.remove(&turn);
    }
    state.records.insert(key.to_owned(), value);
    Ok(true)
}

fn core_error(error: impl std::fmt::Display) -> AgentLoopError {
    AgentLoopError::store(error.to_string())
}
fn log_error(error: impl std::fmt::Display) -> EventLogError {
    EventLogError::Backend {
        detail: error.to_string(),
    }
}

impl Ports {
    /// Commit the receipts riding on a lease write, under its ownership
    /// check. Returns whether any committed.
    fn apply_riding(&self, state: &mut Control) -> Result<bool> {
        let riding = std::mem::take(&mut *self.riding.lock().unwrap_or_else(|p| p.into_inner()));
        let mut any = false;
        for (key, value) in riding {
            any |= apply_record(state, &key, value, true)?;
        }
        Ok(any)
    }
    fn check(&self, state: &Control) -> Result<()> {
        // THREAT[TM-DURABLE-012]: callers hold the authority gate while checking
        // ownership and committing; lease expiry fences delayed writers too.
        let fence = self.fence.read().unwrap_or_else(|p| p.into_inner());
        if fence
            .as_ref()
            .is_some_and(|fence| !fence.is_lost() && state.owns(&fence.holder, fence.fence))
        {
            return Ok(());
        }
        let error = actor_lease_lost(
            self.session.id,
            "memory authority ownership expired or absent",
        );
        if let Some(fence) = &*fence {
            fence.observe::<()>(&Err(actor_lease_lost(self.session.id, "ownership lost")));
        }
        Err(error)
    }
    fn scope(&self, id: SessionId) -> Result<()> {
        if id != self.session.id {
            return Err(AgentLoopError::config("cross-session authority access"));
        }
        Ok(())
    }
    async fn append(
        &self,
        request: EventRequest,
        expected: Option<Option<i32>>,
    ) -> std::result::Result<Event, EventLogError> {
        let mut state = self.session.control.lock().await;
        self.append_locked(&mut state, request, expected).await
    }
    async fn append_locked(
        &self,
        state: &mut Control,
        mut request: EventRequest,
        expected: Option<Option<i32>>,
    ) -> std::result::Result<Event, EventLogError> {
        self.scope(request.session_id).map_err(log_error)?;
        if self.session.drop_prompts
            && let EventData::LlmGeneration(data) = &mut request.data
        {
            data.messages = Vec::new();
            data.request_body = None;
        }
        if !request.is_dreaming_metadata() {
            self.check(state).map_err(log_error)?;
        } else {
            state.binding(self.session.id).map_err(log_error)?;
        }
        let turn = request.context.turn_id;
        let terminal = matches!(
            request.data,
            EventData::TurnCompleted(_)
                | EventData::TurnFailed(_)
                | EventData::TurnCancelled(_)
                | EventData::TurnSealed(_)
        );
        if terminal && let Some(previous) = turn.and_then(|turn| state.terminal.get(&turn)) {
            if serde_json::to_value(&previous.data).ok() == serde_json::to_value(&request.data).ok()
            {
                return Ok(previous.clone());
            }
            return Err(log_error("conflicting turn terminal outcome"));
        }
        state.human_waits.annotate(&mut request);
        let event = match expected {
            None => self.session.log.append(request).await?,
            Some(expected) => {
                self.session
                    .log
                    .append_conditional(request, expected)
                    .await?
            }
        };
        state.human_waits.observe(&event);
        if matches!(event.data, EventData::TurnStarted(_)) {
            state.last_turn_started = event.sequence;
            state.open = turn;
            state.open_input = event.context.input_message_id;
        }
        if matches!(
            event.data,
            EventData::InputMessage(_) | EventData::OutputMessageCompleted(_)
        ) {
            state.has_history = true;
        }
        match &event.data {
            EventData::ToolCallRequested(data) => state.pending = data.tool_calls.clone(),
            EventData::ToolCompleted(data) => {
                state.pending.retain(|call| call.id != data.tool_call_id)
            }
            _ => {}
        }
        if terminal && let Some(turn) = turn {
            state.open = None;
            state.pending.clear();
            state.finalizing.insert(turn);
            state.terminal.insert(turn, event.clone());
        }
        Ok(event)
    }
}

#[async_trait]
impl SessionEpoch for Ports {
    fn renew_poke(&self) -> Option<Arc<tokio::sync::Notify>> {
        Some(self.session.renew_poke.clone())
    }
    fn pushes_admissions(&self) -> bool {
        true
    }
    fn live_deltas(&self) -> Option<Arc<dyn super::LiveDeltaSink>> {
        self.live_deltas.clone()
    }
    fn claimed_wake_at(&self) -> Option<Option<i64>> {
        self.claimed_wake_at
    }
    async fn schedule(&self, wake_at: Option<i64>) -> Result<()> {
        self.session.counts.count(Op::Write);
        let mut state = self.session.control.lock().await;
        self.check(&state)?;
        state.wake_at = wake_at;
        Ok(())
    }
    fn backends(&self) -> HostBackends {
        let mut backends = HostBackends::in_memory();
        backends.dreaming_store = self.session.services.dreaming_store.clone();
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
        let mut bound = self.fence.write().unwrap_or_else(|p| p.into_inner());
        if bound
            .as_ref()
            .is_some_and(|current| current.holder == fence.holder && current.fence == fence.fence)
        {
            *bound = Some(fence.clone());
        }
    }
    async fn record(&self, key: &str) -> Result<Option<Value>> {
        self.session.counts.count(Op::Read);
        Ok(self.session.control.lock().await.records.get(key).cloned())
    }
    async fn records(&self, prefix: &str) -> Result<Vec<(String, Value)>> {
        self.session.counts.count(Op::Read);
        let state = self.session.control.lock().await;
        let records: Vec<_> = state
            .records
            .range(prefix.to_owned()..)
            .take_while(|(key, _)| key.starts_with(prefix))
            .take(4097)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if records.len() > 4096 {
            return Err(AgentLoopError::store(
                "lifecycle receipt scan limit exceeded",
            ));
        }
        Ok(records)
    }
    async fn commit(&self, key: &str, value: Value, absent: bool) -> Result<bool> {
        self.session.counts.count(Op::Write);
        let mut state = self.session.control.lock().await;
        self.check(&state)?;
        let committed = apply_record(&mut state, key, value, absent)?;
        drop(state);
        if committed {
            self.session.changed.notify_waiters();
        }
        Ok(committed)
    }
    async fn commit_with_lease(&self, key: &str, value: Value) -> Result<()> {
        self.riding
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((key.to_owned(), value));
        Ok(())
    }
    async fn suspend(&self, reason: &str) -> Result<()> {
        self.session.counts.count(Op::Write);
        let mut state = self.session.control.lock().await;
        self.check(&state)?;
        state.suspended = true;
        state
            .records
            .insert("suspension".into(), json!({"reason":reason}));
        drop(state);
        self.session.changed.notify_waiters();
        Ok(())
    }
}

#[async_trait]
impl EventReader for Ports {
    async fn read_page(
        &self,
        request: EventReadRequest,
    ) -> std::result::Result<EventPage, EventLogError> {
        self.session.counts.count(Op::Read);
        self.scope(request.session_id()).map_err(log_error)?;
        self.session.log.read_page(request).await
    }

    async fn last_sequence(
        &self,
        session_id: SessionId,
    ) -> std::result::Result<Option<i32>, EventLogError> {
        self.session.counts.count(Op::Read);
        self.scope(session_id).map_err(log_error)?;
        self.session.log.last_sequence(session_id).await
    }
}
#[async_trait]
impl EventLog for Ports {
    async fn append(&self, request: EventRequest) -> std::result::Result<Event, EventLogError> {
        self.session.counts.count(Op::Write);
        Ports::append(self, request, None).await
    }
    async fn append_conditional(
        &self,
        request: EventRequest,
        expected: Option<i32>,
    ) -> std::result::Result<Event, EventLogError> {
        self.session.counts.count(Op::Write);
        Ports::append(self, request, Some(expected)).await
    }
    /// One authority write: the batch commits under one ownership check.
    async fn append_batch(
        &self,
        requests: Vec<EventRequest>,
    ) -> std::result::Result<Vec<Event>, EventLogError> {
        self.session.counts.count(Op::Write);
        self.commit_batch(requests).await
    }
    /// The same write, counted as one nothing waits on.
    async fn append_batch_behind(
        &self,
        requests: Vec<EventRequest>,
    ) -> std::result::Result<Vec<Event>, EventLogError> {
        self.session.counts.count(Op::Write);
        self.session.counts.count(Op::Behind);
        self.commit_batch(requests).await
    }
    fn durability(&self) -> EventDurability {
        EventDurability::Volatile
    }
}

impl Ports {
    async fn commit_batch(
        &self,
        requests: Vec<EventRequest>,
    ) -> std::result::Result<Vec<Event>, EventLogError> {
        let mut state = self.session.control.lock().await;
        let mut events = Vec::with_capacity(requests.len());
        for request in requests {
            events.push(self.append_locked(&mut state, request, None).await?);
        }
        Ok(events)
    }
}

/// Up to `limit` events after `after`, read page by page.
async fn events_after(
    session: &MemorySession,
    after: Option<i32>,
    limit: usize,
) -> Result<Vec<Event>> {
    let mut events = Vec::new();
    let mut request = EventReadRequest::from_cursor(
        EventCursor::after(session.id, after.unwrap_or(0)).map_err(core_error)?,
        EventReadLimit::default(),
    );
    loop {
        let page = session.log.read_page(request).await.map_err(core_error)?;
        events.extend(page.events);
        if events.len() >= limit {
            events.truncate(limit);
            return Ok(events);
        }
        match page.next_cursor {
            Some(cursor) => {
                request = EventReadRequest::from_cursor(cursor, EventReadLimit::default())
            }
            None => return Ok(events),
        }
    }
}

#[async_trait]
impl SessionActorStore for Ports {
    async fn renew_actor_lease(
        &self,
        id: SessionId,
        holder: &str,
        fence: u64,
        ttl: Duration,
        consumed: i64,
    ) -> Result<Option<ActorRenewal>> {
        self.session.counts.count(Op::Write);
        self.scope(id)?;
        let mut state = self.session.control.lock().await;
        if !state.owns(holder, fence) {
            return Ok(None);
        }
        let receipts = self.apply_riding(&mut state)?;
        state.consumed = state.consumed.max(consumed);
        state.expires = Some(Instant::now() + ttl);
        state.cancel_reported = state.cancel.is_some();
        let renewal = ActorRenewal {
            inbox_seq: state.inbox,
            cancel_inbox_seq: state.cancel,
        };
        drop(state);
        if receipts {
            self.session.changed.notify_waiters();
        }
        Ok(Some(renewal))
    }
    async fn release_actor(
        &self,
        id: SessionId,
        holder: &str,
        fence: u64,
        consumed: i64,
    ) -> Result<ActorRelease> {
        self.session.counts.count(Op::Write);
        self.scope(id)?;
        let mut state = self.session.control.lock().await;
        if !state.owns(holder, fence) {
            return Ok(ActorRelease::LeaseLost);
        }
        let receipts = self.apply_riding(&mut state)?;
        state.consumed = state.consumed.max(consumed);
        state.holder = None;
        state.expires = None;
        state.attempts = 0;
        state.runnable = !state.finalizing.is_empty()
            || state.resume_generation > state.resume_claimed
            || state.inbox > state.consumed
            || (state.cancel.is_some() && !state.cancel_reported);
        if state.cancel_reported {
            state.cancel = None;
        }
        state.cancel_reported = false;
        let still_runnable = state.runnable;
        drop(state);
        if receipts {
            self.session.changed.notify_waiters();
        }
        Ok(ActorRelease::Released { still_runnable })
    }
    async fn read_actor_journal(
        &self,
        id: SessionId,
        after: Option<i32>,
        limit: u32,
    ) -> Result<ActorJournal> {
        self.session.counts.count(Op::Read);
        self.scope(id)?;
        let limit = if limit == 0 {
            256
        } else {
            limit.min(1024) as usize
        };
        // With no cursor the tail starts at the latest `turn.started`.
        let after = match after {
            Some(after) => Some(after),
            None => self
                .session
                .control
                .lock()
                .await
                .last_turn_started
                .map(|sequence| sequence - 1),
        };
        // Read one event past the limit to learn whether more follow.
        let mut events = events_after(&self.session, after, limit + 1).await?;
        let head = self
            .session
            .log
            .last_sequence(id)
            .await
            .map_err(core_error)?;
        let has_more = events.len() > limit;
        events.truncate(limit);
        Ok(ActorJournal {
            events,
            has_more,
            head_sequence: head,
        })
    }
}

#[async_trait]
impl SessionStore for Ports {
    async fn get_session(&self, id: SessionId) -> Result<Option<ExecutionSession>> {
        self.session.counts.count(Op::Read);
        self.scope(id)?;
        let state = self.session.control.lock().await;
        let mut record = self.session.services.session_store.get_session(id).await?;
        let configuration: SessionConfiguration = serde_json::from_value(
            state
                .records
                .get("configuration")
                .cloned()
                .unwrap_or(json!({})),
        )
        .map_err(core_error)?;
        if let Some(record) = record.as_mut() {
            configuration.apply(record);
        }
        Ok(record)
    }
}
#[async_trait]
impl RuntimeSessionStore for Ports {
    async fn add_session(&self, session: ExecutionSession) -> Result<()> {
        self.session.counts.count(Op::Write);
        self.scope(session.id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .session_store
            .add_session(session)
            .await
    }
}
#[async_trait]
impl SessionMutator for Ports {
    async fn update_session_title(&self, id: SessionId, title: String) -> Result<ExecutionSession> {
        self.session.counts.count(Op::Write);
        self.scope(id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .session_store
            .update_session_title(id, title)
            .await
    }
}
#[async_trait]
impl SessionStorageStore for Ports {
    async fn get_value(&self, id: SessionId, key: &str) -> Result<Option<String>> {
        self.session.counts.count(Op::Read);
        self.scope(id)?;
        self.session.services.storage_store.get_value(id, key).await
    }
    async fn set_value(&self, id: SessionId, key: &str, value: &str) -> Result<()> {
        self.session.counts.count(Op::Write);
        self.scope(id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .storage_store
            .set_value(id, key, value)
            .await
    }
    async fn take_value(&self, id: SessionId, key: &str) -> Result<Option<String>> {
        self.session.counts.count(Op::Write);
        self.scope(id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .storage_store
            .take_value(id, key)
            .await
    }
    async fn delete_value(&self, id: SessionId, key: &str) -> Result<bool> {
        self.session.counts.count(Op::Write);
        self.scope(id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .storage_store
            .delete_value(id, key)
            .await
    }
    async fn list_keys(&self, id: SessionId) -> Result<Vec<KeyInfo>> {
        self.session.counts.count(Op::Read);
        self.scope(id)?;
        self.session.services.storage_store.list_keys(id).await
    }
    async fn set_secret(&self, id: SessionId, key: &str, value: &str) -> Result<()> {
        self.session.counts.count(Op::Write);
        self.scope(id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .storage_store
            .set_secret(id, key, value)
            .await
    }
    async fn get_secret(&self, id: SessionId, key: &str) -> Result<Option<String>> {
        self.session.counts.count(Op::Read);
        self.scope(id)?;
        self.session
            .services
            .storage_store
            .get_secret(id, key)
            .await
    }
    async fn delete_secret(&self, id: SessionId, key: &str) -> Result<bool> {
        self.session.counts.count(Op::Write);
        self.scope(id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .storage_store
            .delete_secret(id, key)
            .await
    }
    async fn list_secrets(&self, id: SessionId) -> Result<Vec<SecretInfo>> {
        self.session.counts.count(Op::Read);
        self.scope(id)?;
        self.session.services.storage_store.list_secrets(id).await
    }
}
#[async_trait]
impl CompactionCheckpointStore for Ports {
    // Context assembly loads only the history after the newest checkpoint;
    // without this every turn read the whole log.
    async fn latest_source_sequence(&self, id: SessionId) -> Result<Option<i64>> {
        self.session.counts.count(Op::Read);
        self.scope(id)?;
        self.session
            .services
            .compaction_checkpoint_store
            .latest_source_sequence(id)
            .await
    }
    async fn get_latest(
        &self,
        id: SessionId,
        provider: &str,
        model: &str,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.session.counts.count(Op::Read);
        self.scope(id)?;
        self.session
            .services
            .compaction_checkpoint_store
            .get_latest(id, provider, model)
            .await
    }
    async fn get_latest_format(
        &self,
        id: SessionId,
        provider: &str,
        model: &str,
        format: u32,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.session.counts.count(Op::Read);
        self.scope(id)?;
        self.session
            .services
            .compaction_checkpoint_store
            .get_latest_format(id, provider, model, format)
            .await
    }
    async fn install(&self, checkpoint: CompactionCheckpoint) -> Result<bool> {
        self.session.counts.count(Op::Write);
        self.scope(checkpoint.session_id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .compaction_checkpoint_store
            .install(checkpoint)
            .await
    }
    async fn get_proactive_attempt(
        &self,
        id: SessionId,
        provider: &str,
        model: &str,
    ) -> Result<Option<ProactiveCompactionAttempt>> {
        self.session.counts.count(Op::Read);
        self.scope(id)?;
        self.session
            .services
            .compaction_checkpoint_store
            .get_proactive_attempt(id, provider, model)
            .await
    }
    async fn record_proactive_attempt(
        &self,
        id: SessionId,
        provider: &str,
        model: &str,
        attempt: ProactiveCompactionAttempt,
    ) -> Result<()> {
        self.session.counts.count(Op::Write);
        self.scope(id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .compaction_checkpoint_store
            .record_proactive_attempt(id, provider, model, attempt)
            .await
    }
}
#[cfg(test)]
mod tail_tests {
    use super::*;

    /// The authority's reader and Engine's log wrappers report where the log
    /// ends, so tail lookups (a turn's input, an open model stream) read back
    /// from it instead of paging a long session in from its start.
    #[tokio::test]
    async fn readers_report_the_log_end() {
        let profile = MemoryHosting::default();
        let id = SessionId::new();
        let binding = SessionBinding {
            framework: 1,
            behavior: AgentBuild::new("tail", "v1"),
            harness: None,
            environment: None,
            metadata: json!(null),
        };
        profile.create(id, &binding).await.unwrap();
        let receipt = profile
            .admit(id, RuntimeMessage::user("hi"), false)
            .await
            .unwrap();
        let log = profile.reader(id).event_log;
        assert_eq!(log.last_sequence(id).await.unwrap(), Some(receipt.sequence));
        let filtered = super::super::query::InputFilterLog::new(log.clone(), Default::default());
        assert_eq!(
            filtered.last_sequence(id).await.unwrap(),
            Some(receipt.sequence)
        );
        let snapshot = super::super::query::SnapshotLog::new(log, id, receipt.sequence - 1);
        assert_eq!(
            snapshot.last_sequence(id).await.unwrap(),
            Some(receipt.sequence - 1)
        );
    }
}

#[cfg(test)]
mod resolver_tests {
    use super::*;

    /// A build the host never registered runs once its resolver knows it,
    /// and a build it does not know is refused at creation.
    #[tokio::test]
    async fn an_unregistered_build_runs_through_the_resolver() {
        let profile = MemoryHosting::default();
        let engine = crate::Engine::builder()
            .hosting(profile)
            .resolver(|build: &AgentBuild| {
                (build.agent == "from-definition").then(|| {
                    crate::Agent::builder()
                        .instructions("Answer.")
                        .model(crate::Model::simulated("resolved"))
                        .build()
                        .unwrap()
                })
            })
            .build();
        let build = AgentBuild::new("from-definition", "host-v1");
        let session = engine
            .create_registered(SessionId::new(), &build)
            .await
            .unwrap();
        let turn = session.send_and_wait("Hi").await.unwrap();
        assert_eq!(turn.response, "resolved");
        assert!(engine.has_agent_build(&build));
        // A resolved Agent that needs a workspace gets its own default one.
        let workspace = crate::Engine::builder()
            .hosting(MemoryHosting::default())
            .resolver(|_: &AgentBuild| {
                crate::Agent::builder()
                    .instructions("Answer.")
                    .model(crate::Model::simulated("with files"))
                    .file("notes.md", "hello")
                    .build()
                    .ok()
            })
            .build();
        let files = workspace
            .create_registered(SessionId::new(), &AgentBuild::new("files", "host-v1"))
            .await
            .unwrap();
        assert_eq!(
            files.send_and_wait("Hi").await.unwrap().response,
            "with files"
        );
        let unknown = AgentBuild::new("unknown", "host-v1");
        assert!(
            engine
                .create_registered(SessionId::new(), &unknown)
                .await
                .is_err()
        );
    }
}

#[cfg(test)]
mod trim_floor_tests {
    use super::*;

    /// A session on a provider without native compaction keeps trimming its
    /// prompt. Once the history passes a few budgets a durable floor appears,
    /// and later turns load (and record) only what follows it, so the prompt
    /// stops growing with the session.
    #[tokio::test]
    async fn trimming_sessions_load_a_bounded_suffix() {
        let profile = MemoryHosting::default();
        let build = AgentBuild::new("trim-floor", "v1");
        let agent = crate::Agent::builder()
            .instructions("You are concise.")
            .model(crate::Model::simulated_with_config(
                crate::LlmSimConfig::lorem(150).with_context_window(2_000),
            ))
            .capability(crate::CompactionConfig::new())
            .build()
            .unwrap();
        let engine = crate::Engine::builder()
            .hosting(profile.clone())
            .register(build.clone(), agent)
            .build();
        let session = engine
            .create_registered(SessionId::new(), &build)
            .await
            .unwrap();
        let id = session.session_id();
        for _ in 0..60 {
            session.send_and_wait("hi").await.unwrap();
        }
        let floor = profile
            .ports(id)
            .get_latest(id, "llmsim", "llmsim-model")
            .await
            .unwrap()
            .expect("a trim floor");
        assert!(matches!(
            floor.payload,
            everruns_core::CompactionCheckpointPayload::Trimmed
        ));
        let mut prompts = Vec::new();
        let log = profile.reader(id).event_log;
        let mut request = EventReadRequest::new(id, EventReadLimit::default());
        loop {
            let page = log.read_page(request).await.unwrap();
            for event in &page.events {
                if let EventData::LlmGeneration(data) = &event.data {
                    prompts.push(data.messages.len());
                }
            }
            match page.next_cursor {
                Some(cursor) => {
                    request = EventReadRequest::from_cursor(cursor, EventReadLimit::default())
                }
                None => break,
            }
        }
        let largest = *prompts.iter().max().unwrap();
        assert!(
            *prompts.last().unwrap() < largest,
            "the floor shortened later prompts: {prompts:?}"
        );
    }
}

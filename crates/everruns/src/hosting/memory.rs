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

/// A coherent process-lifetime authority. Clones share sessions. There is no
/// claim that accepted work survives losing this process or Wasm isolate.
#[derive(Clone, Default)]
pub struct MemoryHosting {
    sessions: Arc<Mutex<HashMap<SessionId, Arc<MemorySession>>>>,
}

struct MemorySession {
    id: SessionId,
    renew_poke: Arc<tokio::sync::Notify>,
    /// [`SessionHosting::changes`]: signalled after every receipt or control write.
    changed: Arc<tokio::sync::Notify>,
    control: tokio::sync::Mutex<Control>,
    log: InMemoryEventLog,
    services: HostBackends,
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
    pending: Vec<crate::ToolCall>,
    finalizing: HashSet<TurnId>,
    records: HashMap<String, Value>,
    pending_inputs: std::collections::VecDeque<i64>,
    commands: HashMap<String, super::spi::session_commands::CommandReceipt>,
    open_input: Option<MessageId>,
    has_history: bool,
    target_cancels: HashMap<MessageId, i64>,
    terminal: HashMap<TurnId, Event>,
}

impl MemoryHosting {
    fn new_session(id: SessionId) -> Arc<MemorySession> {
        Arc::new(MemorySession {
            id,
            renew_poke: Arc::default(),
            changed: Arc::default(),
            control: tokio::sync::Mutex::new(Control::default()),
            log: InMemoryEventLog::new(),
            services: HostBackends::in_memory(),
        })
    }
    fn session(&self, id: SessionId) -> Arc<MemorySession> {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
            .cloned()
            .unwrap_or_else(|| Self::new_session(id))
    }
    fn ports(&self, id: SessionId) -> Ports {
        Ports {
            session: self.session(id),
            fence: Arc::new(RwLock::new(None)),
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
                session: session.clone(),
                fence: Arc::new(RwLock::new(Some(ActorFence::new(id, holder, state.fence)))),
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
        self.scope(request.session_id).map_err(log_error)?;
        let mut state = self.session.control.lock().await;
        self.check(&state).map_err(log_error)?;
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
        let event = match expected {
            None => self.session.log.append(request).await?,
            Some(expected) => {
                self.session
                    .log
                    .append_conditional(request, expected)
                    .await?
            }
        };
        if matches!(event.data, EventData::TurnStarted(_)) {
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
    async fn schedule(&self, wake_at: Option<i64>) -> Result<()> {
        let mut state = self.session.control.lock().await;
        self.check(&state)?;
        state.wake_at = wake_at;
        Ok(())
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
        let mut bound = self.fence.write().unwrap_or_else(|p| p.into_inner());
        if bound
            .as_ref()
            .is_some_and(|current| current.holder == fence.holder && current.fence == fence.fence)
        {
            *bound = Some(fence.clone());
        }
    }
    async fn record(&self, key: &str) -> Result<Option<Value>> {
        Ok(self.session.control.lock().await.records.get(key).cloned())
    }
    async fn records(&self, prefix: &str) -> Result<Vec<(String, Value)>> {
        let state = self.session.control.lock().await;
        let records: Vec<_> = state
            .records
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
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
        let mut state = self.session.control.lock().await;
        self.check(&state)?;
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
        drop(state);
        self.session.changed.notify_waiters();
        Ok(true)
    }
    async fn suspend(&self, reason: &str) -> Result<()> {
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
        self.scope(request.session_id()).map_err(log_error)?;
        self.session.log.read_page(request).await
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
        EventDurability::Volatile
    }
}

async fn all_events(session: &MemorySession, after: Option<i32>) -> Result<Vec<Event>> {
    let mut events = Vec::new();
    let mut request = EventReadRequest::from_cursor(
        EventCursor::after(session.id, after.unwrap_or(0)).map_err(core_error)?,
        EventReadLimit::default(),
    );
    loop {
        let page = session.log.read_page(request).await.map_err(core_error)?;
        events.extend(page.events);
        if events.len() > 100_000 {
            return Err(AgentLoopError::store(
                "session journal recovery limit exceeded",
            ));
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
        self.scope(id)?;
        let mut state = self.session.control.lock().await;
        if !state.owns(holder, fence) {
            return Ok(None);
        }
        state.consumed = state.consumed.max(consumed);
        state.expires = Some(Instant::now() + ttl);
        state.cancel_reported = state.cancel.is_some();
        Ok(Some(ActorRenewal {
            inbox_seq: state.inbox,
            cancel_inbox_seq: state.cancel,
        }))
    }
    async fn release_actor(
        &self,
        id: SessionId,
        holder: &str,
        fence: u64,
        consumed: i64,
    ) -> Result<ActorRelease> {
        self.scope(id)?;
        let mut state = self.session.control.lock().await;
        if !state.owns(holder, fence) {
            return Ok(ActorRelease::LeaseLost);
        }
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
        Ok(ActorRelease::Released {
            still_runnable: state.runnable,
        })
    }
    async fn read_actor_journal(
        &self,
        id: SessionId,
        after: Option<i32>,
        limit: u32,
    ) -> Result<ActorJournal> {
        self.scope(id)?;
        let mut events = all_events(&self.session, after).await?;
        let head = events.last().and_then(|event| event.sequence);
        if after.is_none()
            && let Some(start) = events
                .iter()
                .rposition(|event| matches!(event.data, EventData::TurnStarted(_)))
        {
            events.drain(..start);
        }
        let limit = if limit == 0 {
            256
        } else {
            limit.min(1024) as usize
        };
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
        self.scope(id)?;
        self.session.services.storage_store.get_value(id, key).await
    }
    async fn set_value(&self, id: SessionId, key: &str, value: &str) -> Result<()> {
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
        self.scope(id)?;
        self.session.services.storage_store.list_keys(id).await
    }
    async fn set_secret(&self, id: SessionId, key: &str, value: &str) -> Result<()> {
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
        self.scope(id)?;
        self.session
            .services
            .storage_store
            .get_secret(id, key)
            .await
    }
    async fn delete_secret(&self, id: SessionId, key: &str) -> Result<bool> {
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
        self.scope(id)?;
        self.session.services.storage_store.list_secrets(id).await
    }
}
#[async_trait]
impl CompactionCheckpointStore for Ports {
    async fn get_latest(
        &self,
        id: SessionId,
        provider: &str,
        model: &str,
    ) -> Result<Option<CompactionCheckpoint>> {
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
        self.scope(id)?;
        self.session
            .services
            .compaction_checkpoint_store
            .get_latest_format(id, provider, model, format)
            .await
    }
    async fn install(&self, checkpoint: CompactionCheckpoint) -> Result<bool> {
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

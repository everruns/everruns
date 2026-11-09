//! The canonical event router: commit to the log, then notify the sink.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result as CoreResult};
use everruns_contracts::typed_id::{EventId, SessionId};
use tokio::sync::Mutex;

use super::{EventDeliveryStats, EventLog, EventSink, EventSinkError};
use crate::event_emitter::EventEmitter;
use crate::events::{Event, EventData, EventRequest};

/// Canonical event router shared by in-process, worker, and facade hosts.
///
/// Durable requests commit to the log before notification. Ephemeral requests
/// use the protocol's [`EventRequest::is_ephemeral`] classification, receive
/// `sequence = None`, and are sink-only. Per-session serialization preserves
/// append acceptance order through the post-commit sink call.
///
/// With [`HostEventEmitter::enable_committed_batches`], events nothing waits
/// on yet (lifecycle records, and a turn's final answer; see `CommitClass::Held`) are held and committed together with the next event that
/// is not, in one [`EventLog::append_batch`]. A held event is returned provisionally, without
/// a sequence, and reaches the sink only once committed, so no listener ever
/// sees an event the log does not hold. A crash loses only held events, which
/// the turn re-emits when it runs again.
///
/// Progress markers (`CommitClass::Behind`) commit behind the work
/// instead: [`EventLog::append_batch_behind`] runs in a
/// background task, chained after the session's previous one, and the
/// emitter returns the event provisionally. Every later commit, flush and
/// live delta of the session waits for that chain first, so order holds and
/// a listener still sees only committed events; a failed commit fails the
/// next one that waits.
#[derive(Clone)]
pub struct HostEventEmitter {
    log: Arc<dyn EventLog>,
    sink: Arc<dyn EventSink>,
    session_locks: Arc<Mutex<HashMap<SessionId, Weak<Mutex<()>>>>>,
    full: Arc<AtomicU64>,
    closed: Arc<AtomicU64>,
    batching: Arc<std::sync::atomic::AtomicBool>,
    held: Arc<std::sync::Mutex<HashMap<SessionId, Vec<EventRequest>>>>,
    behind: Arc<std::sync::Mutex<HashMap<SessionId, Behind>>>,
}

/// The last commit running behind a session's work.
type Behind = everruns_contracts::rt::JoinHandle<Result<(), String>>;

/// When an event must be in the log, relative to the `emit` that accepted
/// it (see [`HostEventEmitter`]).
///
/// Decision: one typed table decides every event's class, keyed by the
/// protocol's event type constants. An event type the table does not name
/// is a [`CommitClass::Barrier`], the strongest class, so a new event is
/// stored before `emit` returns until someone argues it need not be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommitClass {
    /// A live delta: never stored, sink only.
    Live,
    /// Stored before `emit` returns, with everything held before it.
    Barrier,
    /// Held and stored together with the next barrier.
    ///
    /// Only events that no reader of the same epoch looks for before that
    /// barrier, that no live view shows as progress, and that a rerun of
    /// the turn re-emits:
    /// - lifecycle records: `session.activated`, `reason.completed`,
    ///   `capability.usage`, `llm.generation`;
    /// - the answer that ends a turn (`output.message.completed` without
    ///   tool calls): it rides on the `turn.completed` that follows. A tail
    ///   read, a steered input and the lease release commit it first. An
    ///   answer that calls tools stays a barrier, so no tool runs on a
    ///   decision the log may still lose.
    ///
    /// `turn.started`, inputs, tool events and terminal events stay
    /// barriers. A terminal event is never held: an epoch that ends without
    /// releasing (its lease expired, a step budget ran out) loses what it
    /// holds, and a turn whose `turn.completed` keeps getting lost never
    /// completes.
    Held,
    /// Stored behind the work, in order, without the emitter waiting.
    ///
    /// Only events the executor never reads back before its next commit,
    /// and needs no sequence for:
    /// - ahead of a model call that overlaps their commit: `reason.started`
    ///   ("thinking"), `output.message.started` ("answering") and
    ///   `context.compacted` (the step installs the compacted context in
    ///   memory; a later step or turn reads the event only after a commit
    ///   that waits for it);
    /// - `session.idled`, after the committed terminal event: it overlaps
    ///   the turn's finalization, the release waits for it, and a holder
    ///   that dies first leaves an idle the next one owes and emits
    ///   (`unidled_turn`).
    Behind,
}

impl CommitClass {
    pub(crate) fn of(request: &EventRequest) -> Self {
        use crate::events::{
            CAPABILITY_USAGE, CONTEXT_COMPACTED, LLM_GENERATION, OUTPUT_MESSAGE_STARTED,
            REASON_COMPLETED, REASON_STARTED, SESSION_ACTIVATED, SESSION_IDLED,
        };
        if request.is_ephemeral() {
            return Self::Live;
        }
        if let EventData::OutputMessageCompleted(data) = &request.data {
            return if data.message.has_tool_calls() {
                Self::Barrier
            } else {
                Self::Held
            };
        }
        match request.event_type.as_str() {
            REASON_STARTED | OUTPUT_MESSAGE_STARTED | CONTEXT_COMPACTED | SESSION_IDLED => {
                Self::Behind
            }
            SESSION_ACTIVATED | REASON_COMPLETED | CAPABILITY_USAGE | LLM_GENERATION => Self::Held,
            _ => Self::Barrier,
        }
    }
}

impl HostEventEmitter {
    /// Route requests through a coherent log and optional live sink.
    pub fn new(log: Arc<dyn EventLog>, sink: Arc<dyn EventSink>) -> Self {
        Self {
            log,
            sink,
            session_locks: Arc::new(Mutex::new(HashMap::new())),
            full: Arc::new(AtomicU64::new(0)),
            closed: Arc::new(AtomicU64::new(0)),
            batching: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            held: Arc::new(std::sync::Mutex::new(HashMap::new())),
            behind: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Commit deferrable lifecycle events together with the next barrier
    /// event. Shared by every clone of this emitter. Call [`Self::flush`]
    /// when the work that emits through it ends.
    pub fn enable_committed_batches(&self) {
        self.batching.store(true, Ordering::Release);
    }

    /// Drop every held event uncommitted (the epoch's lease is lost). A
    /// commit already running behind finishes or fails on its own.
    pub fn abandon(&self) {
        self.held_map().clear();
        self.behind_map().clear();
    }

    /// Commit every held event, after the commits running behind.
    pub async fn flush(&self) -> CoreResult<()> {
        let mut sessions: Vec<SessionId> = self.held_map().keys().copied().collect();
        sessions.extend(self.behind_map().keys().copied());
        sessions.sort_unstable_by_key(|id| id.to_string());
        sessions.dedup();
        for session_id in sessions {
            let lock = self.session_lock(session_id).await;
            let _guard = lock.lock().await;
            self.commit(session_id, None).await?;
        }
        Ok(())
    }

    fn held_map(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, Vec<EventRequest>>> {
        self.held
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn behind_map(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, Behind>> {
        self.behind
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Wait for the session's commits running behind. Callers hold the
    /// session lock.
    async fn settle(&self, session_id: SessionId) -> CoreResult<()> {
        let Some(behind) = self.behind_map().remove(&session_id) else {
            return Ok(());
        };
        match behind.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(AgentLoopError::store(error)),
            Err(_) => Err(AgentLoopError::store(
                "event commit stopped before it finished",
            )),
        }
    }

    /// Commit the held events and `barrier` behind the work, then notify
    /// them and `then` (a live delta) in order. Callers hold the session lock.
    fn commit_behind(
        &self,
        session_id: SessionId,
        barrier: Option<EventRequest>,
        then: Option<Event>,
    ) {
        let mut requests = self.held_map().remove(&session_id).unwrap_or_default();
        requests.extend(barrier);
        let previous = self.behind_map().remove(&session_id);
        let emitter = self.clone();
        let task = everruns_contracts::rt::spawn(async move {
            if let Some(previous) = previous {
                match previous.await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => return Err(error),
                    Err(_) => return Err("event commit stopped before it finished".to_owned()),
                }
            }
            if !requests.is_empty() {
                let events = emitter
                    .log
                    .append_batch_behind(requests)
                    .await
                    .map_err(|error| error.to_string())?;
                for event in events {
                    emitter.notify(event);
                }
            }
            if let Some(event) = then {
                emitter.notify(event);
            }
            Ok(())
        });
        self.behind_map().insert(session_id, task);
    }

    /// Whether a commit of the session is still running behind.
    fn running_behind(&self, session_id: SessionId) -> bool {
        self.behind_map()
            .get(&session_id)
            .is_some_and(|behind| !behind.is_finished())
    }

    /// Commit the session's held events, then `barrier` when given, and
    /// notify the sink in order. Returns the committed events. Callers hold
    /// the session lock.
    async fn commit(
        &self,
        session_id: SessionId,
        barrier: Option<EventRequest>,
    ) -> CoreResult<Vec<Event>> {
        self.settle(session_id).await?;
        let mut requests = self.held_map().remove(&session_id).unwrap_or_default();
        requests.extend(barrier);
        let events = if requests.len() == 1 {
            let mut events = Vec::with_capacity(1);
            for request in requests {
                events.push(
                    self.log
                        .append(request)
                        .await
                        .map_err(|error| AgentLoopError::store(error.to_string()))?,
                );
            }
            events
        } else if requests.is_empty() {
            return Ok(Vec::new());
        } else {
            self.log
                .append_batch(requests)
                .await
                .map_err(|error| AgentLoopError::store(error.to_string()))?
        };
        for event in &events {
            self.notify(event.clone());
        }
        Ok(events)
    }

    /// Canonical log used by this router.
    pub fn event_log(&self) -> Arc<dyn EventLog> {
        self.log.clone()
    }

    /// Current observational failure counters.
    pub fn delivery_stats(&self) -> EventDeliveryStats {
        EventDeliveryStats {
            full: self.full.load(Ordering::Relaxed),
            closed: self.closed.load(Ordering::Relaxed),
        }
    }

    async fn session_lock(&self, session_id: SessionId) -> Arc<Mutex<()>> {
        let mut locks = self.session_locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&session_id).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(session_id, Arc::downgrade(&lock));
        lock
    }

    fn notify(&self, event: Event) {
        match self.sink.try_send(event) {
            Ok(()) => {}
            Err(EventSinkError::Full { dropped }) => {
                self.full.fetch_add(dropped, Ordering::Relaxed);
                tracing::debug!("live event sink full; canonical append remains committed");
            }
            Err(EventSinkError::Closed) => {
                self.closed.fetch_add(1, Ordering::Relaxed);
                tracing::debug!("live event sink closed; canonical append remains committed");
            }
        }
    }
}

#[async_trait]
impl EventEmitter for HostEventEmitter {
    fn turn_causal_context(
        &self,
        context: &crate::events::EventContext,
    ) -> crate::causal::CausalContext {
        self.sink.turn_causal_context(context)
    }
    fn restore_turn_context(
        &self,
        context: &crate::events::EventContext,
        metadata: Option<&serde_json::Value>,
    ) {
        self.sink.restore_turn_context(context, metadata)
    }
    async fn emit(&self, mut request: EventRequest) -> CoreResult<Event> {
        let session_id = request.session_id;
        let lock = self.session_lock(session_id).await;
        let _guard = lock.lock().await;
        if let Ok(Some(carrier)) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.sink.prepare_causal_context(&request)
        })) {
            crate::causal::attach_turn_trace(&mut request, &carrier);
        }
        if !self.batching.load(Ordering::Acquire) {
            let event = if request.is_ephemeral() {
                ephemeral_event(request)
            } else {
                self.log
                    .append(request)
                    .await
                    .map_err(|error| AgentLoopError::store(error.to_string()))?
            };
            self.notify(event.clone());
            return Ok(event);
        }
        if request.is_ephemeral() {
            // A live delta must not overtake the events it follows: behind
            // their commit when one runs, else at once.
            let event = ephemeral_event(request);
            let has_held = self
                .held_map()
                .get(&session_id)
                .is_some_and(|held| !held.is_empty());
            if has_held || self.running_behind(session_id) {
                self.commit_behind(session_id, None, Some(event.clone()));
            } else {
                self.notify(event.clone());
            }
            return Ok(event);
        }
        match CommitClass::of(&request) {
            CommitClass::Behind => {
                let provisional = ephemeral_event(request.clone());
                self.commit_behind(session_id, Some(request), None);
                Ok(provisional)
            }
            CommitClass::Held => {
                let provisional = ephemeral_event(request.clone());
                self.held_map().entry(session_id).or_default().push(request);
                Ok(provisional)
            }
            CommitClass::Barrier | CommitClass::Live => self
                .commit(session_id, Some(request))
                .await?
                .pop()
                .ok_or_else(|| AgentLoopError::store("event log returned no committed event")),
        }
    }
    async fn wait_stored(&self, session_id: SessionId) -> CoreResult<()> {
        let lock = self.session_lock(session_id).await;
        let _guard = lock.lock().await;
        self.settle(session_id).await
    }
}

fn ephemeral_event(request: EventRequest) -> Event {
    Event {
        id: EventId::new(),
        event_type: request.event_type,
        ts: request.ts,
        session_id: request.session_id,
        context: request.context,
        data: request.data,
        metadata: request.metadata,
        tags: request.tags,
        sequence: None,
    }
}

#[cfg(test)]
mod commit_class_tests {
    use super::*;
    use crate::events::{EventContext, VALID_EVENT_TYPES};

    /// The whole table. Every other event type of the protocol, and any
    /// type added later, is a barrier.
    #[test]
    fn only_the_listed_event_types_are_deferred() {
        let deferred = [
            ("reason.started", CommitClass::Behind),
            ("output.message.started", CommitClass::Behind),
            ("context.compacted", CommitClass::Behind),
            ("session.idled", CommitClass::Behind),
            ("session.activated", CommitClass::Held),
            ("reason.completed", CommitClass::Held),
            ("capability.usage", CommitClass::Held),
            ("llm.generation", CommitClass::Held),
            ("output.message.delta", CommitClass::Live),
            ("reason.thinking.delta", CommitClass::Live),
            ("tool.output.delta", CommitClass::Live),
            ("voice.input_transcript.delta", CommitClass::Live),
            ("voice.output_transcript.delta", CommitClass::Live),
        ];
        for (event_type, class) in deferred {
            assert!(
                class == CommitClass::Live || VALID_EVENT_TYPES.contains(&event_type),
                "{event_type}"
            );
        }
        let named = deferred.iter().map(|(event_type, _)| event_type);
        for event_type in VALID_EVENT_TYPES
            .iter()
            .chain(named)
            .chain(&["a.new.event"])
        {
            let request = EventRequest {
                event_type: (*event_type).to_owned(),
                ts: chrono::Utc::now(),
                session_id: SessionId::new(),
                context: EventContext::empty(),
                data: EventData::unsupported((*event_type).to_owned(), serde_json::Value::Null),
                metadata: None,
                tags: None,
            };
            let expected = deferred
                .iter()
                .find(|(deferred, _)| deferred == event_type)
                .map_or(CommitClass::Barrier, |(_, class)| *class);
            assert_eq!(CommitClass::of(&request), expected, "{event_type}");
        }
    }
}

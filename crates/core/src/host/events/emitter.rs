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
use crate::events::{Event, EventRequest};

/// Canonical event router shared by in-process, worker, and facade hosts.
///
/// Durable requests commit to the log before notification. Ephemeral requests
/// use the protocol's [`EventRequest::is_ephemeral`] classification, receive
/// `sequence = None`, and are sink-only. Per-session serialization preserves
/// append acceptance order through the post-commit sink call.
///
/// With [`HostEventEmitter::enable_committed_batches`], lifecycle events that
/// nothing waits on (`reason.*`, `llm.generation`, `capability.usage`,
/// `output.message.started`, `session.activated`) are held and committed
/// together with the next event that is not, in one
/// [`EventLog::append_batch`]. A held event is returned provisionally, without
/// a sequence, and reaches the sink only once committed, so no listener ever
/// sees an event the log does not hold. A crash loses only held events, which
/// the turn re-emits when it runs again.
#[derive(Clone)]
pub struct HostEventEmitter {
    log: Arc<dyn EventLog>,
    sink: Arc<dyn EventSink>,
    session_locks: Arc<Mutex<HashMap<SessionId, Weak<Mutex<()>>>>>,
    full: Arc<AtomicU64>,
    closed: Arc<AtomicU64>,
    batching: Arc<std::sync::atomic::AtomicBool>,
    held: Arc<std::sync::Mutex<HashMap<SessionId, Vec<EventRequest>>>>,
}

/// Lifecycle events a committed batch may hold until the next barrier event.
///
/// Decision: only events that no reader of the same epoch looks for and that
/// a rerun of the turn re-emits. `turn.started`, messages, tool events and
/// terminal events stay barriers.
fn is_deferrable_event_type(event_type: &str) -> bool {
    matches!(
        event_type,
        "session.activated"
            | "reason.started"
            | "reason.completed"
            | "capability.usage"
            | "output.message.started"
            | "llm.generation"
    )
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
        }
    }

    /// Commit deferrable lifecycle events together with the next barrier
    /// event. Shared by every clone of this emitter. Call [`Self::flush`]
    /// when the work that emits through it ends.
    pub fn enable_committed_batches(&self) {
        self.batching.store(true, Ordering::Release);
    }

    /// Drop every held event uncommitted (the epoch's lease is lost).
    pub fn abandon(&self) {
        self.held_map().clear();
    }

    /// Commit every held event.
    pub async fn flush(&self) -> CoreResult<()> {
        let sessions: Vec<SessionId> = self.held_map().keys().copied().collect();
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

    /// Commit the session's held events, then `barrier` when given, and
    /// notify the sink in order. Returns the committed events. Callers hold
    /// the session lock.
    async fn commit(
        &self,
        session_id: SessionId,
        barrier: Option<EventRequest>,
    ) -> CoreResult<Vec<Event>> {
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
            // A live delta must not overtake the events it follows.
            self.commit(session_id, None).await?;
            let event = ephemeral_event(request);
            self.notify(event.clone());
            return Ok(event);
        }
        if is_deferrable_event_type(&request.event_type) {
            let provisional = ephemeral_event(request.clone());
            self.held_map().entry(session_id).or_default().push(request);
            return Ok(provisional);
        }
        self.commit(session_id, Some(request))
            .await?
            .pop()
            .ok_or_else(|| AgentLoopError::store("event log returned no committed event"))
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

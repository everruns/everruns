//! The Engine's storage ports for a platform session, over the worker's
//! adapters (data plane v2 step 4).
//!
//! Decisions:
//! - The session's event log is the platform's: the same journal the
//!   worker's own host folds. Reads page the actor journal; writes are
//!   fenced by the actor lease exactly as the worker's own turns are, so a
//!   holder that lost its lease writes nothing and its driver hears of it
//!   (`ActorFence::observe`). A batch of appends is one fenced write that
//!   answers with every stored event, as the worker's own turns write.
//! - A writer keeps the epoch's journal tail ([`JournalTail`]): reads of
//!   recent history are answered from it after one read of what follows
//!   its head, or with no read for a snapshot it already holds.
//! - The Engine names the agent it runs by its own key. The platform's
//!   events name the platform's agent and harness, as a worker turn's do,
//!   so this port stamps them on the events that carry them
//!   ([`EventStamp`]); the Engine never sees a platform id.
//! - The session record the Engine runs on is the Engine's own: it seeds an
//!   id-free `ExecutionSession` per activation, and this store keeps it in
//!   memory for that activation with the session's live configuration
//!   (model, session capabilities) applied on every read. A title the agent
//!   sets also goes to the platform session.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use everruns::hosting::spi::session_commands::SessionConfiguration;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};

use crate::actor_lease_store::{ActorFence, SessionActorStore};
use crate::core::events::{Event, EventData, EventRequest};
use crate::core::host::{
    DEFAULT_EVENT_READ_LIMIT, EventCursor, EventDurability, EventLog, EventLogError, EventPage,
    EventReadRequest, EventReader, RuntimeSessionStore, SessionMutator,
};
use crate::core::{ExecutionSession, SessionStore};
use crate::worker_adapters::WorkerAdapters;

use super::journal::JournalTail;

fn backend(error: impl std::fmt::Display) -> EventLogError {
    EventLogError::Backend {
        detail: error.to_string(),
    }
}

/// The platform agent and harness a session's events name.
#[derive(Debug, Clone, Default)]
pub struct EventStamp {
    pub agent_id: Option<AgentId>,
    pub harness_id: Option<HarnessId>,
}

impl EventStamp {
    /// Name the platform's agent and harness where the Engine named its own,
    /// and tag the event with the agent as a worker turn does.
    pub fn apply(&self, request: &mut EventRequest) {
        match &mut request.data {
            EventData::SessionStarted(data) => {
                data.agent_id = self.agent_id;
                if let Some(harness) = self.harness_id {
                    data.harness_id = harness;
                }
            }
            EventData::TurnStarted(data) => data.agent_id = self.agent_id,
            EventData::ReasonStarted(data) => {
                data.agent_id = self.agent_id;
                if let Some(harness) = self.harness_id {
                    data.harness_id = harness;
                }
            }
            _ => {}
        }
        if let Some(agent) = self.agent_id {
            let metadata = request
                .metadata
                .get_or_insert_with(|| serde_json::Value::Object(Default::default()));
            if let Some(metadata) = metadata.as_object_mut() {
                metadata.insert(
                    "agent_id".to_string(),
                    serde_json::Value::String(agent.to_string()),
                );
            }
        }
    }
}

/// A platform session's journal. Without a fence it is a reader: appends
/// are refused.
pub struct PlatformEventLog<A: WorkerAdapters> {
    adapters: A,
    journal: Arc<dyn SessionActorStore>,
    stamp: EventStamp,
    /// The lease writes present; the epoch swaps in the driver's fence (its
    /// lost signal) with this worker's holder.
    fence: Option<Arc<Mutex<ActorFence>>>,
    /// The epoch's journal tail; a reader keeps none.
    tail: Option<JournalTail>,
}

impl<A: WorkerAdapters> PlatformEventLog<A> {
    pub fn writer(
        adapters: A,
        journal: Arc<dyn SessionActorStore>,
        fence: Arc<Mutex<ActorFence>>,
        stamp: EventStamp,
    ) -> Self {
        Self {
            adapters,
            journal,
            stamp,
            fence: Some(fence),
            tail: Some(JournalTail::default()),
        }
    }

    pub fn reader(adapters: A, journal: Arc<dyn SessionActorStore>) -> Self {
        Self {
            adapters,
            journal,
            stamp: EventStamp::default(),
            fence: None,
            tail: None,
        }
    }

    fn fence(&self) -> std::result::Result<ActorFence, EventLogError> {
        self.fence
            .as_ref()
            .map(|fence| fence.lock().unwrap_or_else(|p| p.into_inner()).clone())
            .ok_or_else(|| EventLogError::InvalidAppend {
                detail: "this session reader refuses writes".into(),
            })
    }
}

#[async_trait]
impl<A: WorkerAdapters> EventReader for PlatformEventLog<A> {
    async fn read_page(
        &self,
        request: EventReadRequest,
    ) -> std::result::Result<EventPage, EventLogError> {
        let session = request.session_id();
        let after = request.cursor().map_or(0, EventCursor::after_sequence);
        let pinned = request
            .cursor()
            .and_then(EventCursor::snapshot_high_watermark);
        if let Some(page) = self.cached_page(&request, after, pinned).await? {
            return Ok(page);
        }
        let limit = u32::try_from(request.limit().get()).unwrap_or(u32::MAX);
        let journal = self
            .journal
            .read_actor_journal(session, Some(after), limit)
            .await
            .map_err(backend)?;
        if let Some(tail) = &self.tail {
            tail.record(after, &journal.events);
        }
        let watermark = pinned.unwrap_or(journal.head_sequence.unwrap_or(0));
        let mut last = after;
        let mut events = Vec::with_capacity(journal.events.len());
        for event in journal.events {
            let Some(sequence) = event.sequence else {
                continue;
            };
            if sequence > watermark {
                break;
            }
            last = sequence;
            if request
                .types()
                .is_none_or(|types| types.contains(&event.event_type.as_str()))
            {
                events.push(event);
            }
        }
        let next = (journal.has_more && last < watermark)
            .then(|| EventCursor::continuation(session, last, watermark))
            .transpose()?;
        EventPage::new(events, next, watermark)
    }

    async fn last_sequence(
        &self,
        session_id: SessionId,
    ) -> std::result::Result<Option<i32>, EventLogError> {
        let Some(tail) = &self.tail else {
            return Ok(None);
        };
        self.refresh(tail, session_id).await
    }
}

impl<A: WorkerAdapters> PlatformEventLog<A> {
    /// Bring the tail up to the log's head: one read of what follows the
    /// cached head. The newest sequence cached, `None` without a tail.
    async fn refresh(
        &self,
        tail: &JournalTail,
        session: SessionId,
    ) -> std::result::Result<Option<i32>, EventLogError> {
        let Some(mut head) = tail.head() else {
            return Ok(None);
        };
        loop {
            let journal = self
                .journal
                .read_actor_journal(session, Some(head), DEFAULT_EVENT_READ_LIMIT as u32)
                .await
                .map_err(backend)?;
            let last = journal.events.last().and_then(|event| event.sequence);
            tail.record(head, &journal.events);
            let Some(newer) = tail.head_after(head) else {
                return Ok(None);
            };
            if last.is_some_and(|last| last != newer) {
                // The read did not continue the tail: answer from the log.
                return Ok(None);
            }
            if journal.has_more && newer > head {
                head = newer;
            } else {
                return Ok(Some(newer));
            }
        }
    }

    /// The page the tail answers: a snapshot it holds at once, any other
    /// read after one read of what follows its head. `None` when the read
    /// goes to the control plane.
    async fn cached_page(
        &self,
        request: &EventReadRequest,
        after: i32,
        pinned: Option<i32>,
    ) -> std::result::Result<Option<EventPage>, EventLogError> {
        let Some(tail) = &self.tail else {
            return Ok(None);
        };
        let session = request.session_id();
        let Some(cached_head) = tail.head_after(after) else {
            return Ok(None);
        };
        let watermark = match pinned {
            Some(watermark) if watermark <= cached_head => watermark,
            _ => match self.refresh(tail, session).await? {
                Some(head) if tail.head_after(after).is_some() => pinned.unwrap_or(head),
                _ => return Ok(None),
            },
        };
        let keep = |event: &Event| {
            request
                .types()
                .is_none_or(|types| types.contains(&event.event_type.as_str()))
        };
        let Some(page) = tail.page(after, watermark, request.limit().get(), keep) else {
            return Ok(None);
        };
        let next = page
            .more
            .then(|| EventCursor::continuation(session, page.last, watermark))
            .transpose()?;
        EventPage::new(page.events, next, watermark).map(Some)
    }
}

#[async_trait]
impl<A: WorkerAdapters> EventLog for PlatformEventLog<A> {
    async fn append(&self, mut request: EventRequest) -> std::result::Result<Event, EventLogError> {
        let fence = self.fence()?;
        self.stamp.apply(&mut request);
        let stored = self
            .adapters
            .emit_events_fenced(vec![request], &fence)
            .await;
        fence.observe(&stored);
        let event = stored
            .map_err(backend)?
            .ok_or_else(|| backend("the control plane did not return the stored event"))?;
        if let Some(tail) = &self.tail {
            tail.appended(std::slice::from_ref(&event));
        }
        Ok(event)
    }

    async fn append_batch(
        &self,
        requests: Vec<EventRequest>,
    ) -> std::result::Result<Vec<Event>, EventLogError> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        let fence = self.fence()?;
        let count = requests.len();
        let requests = requests
            .into_iter()
            .map(|mut request| {
                self.stamp.apply(&mut request);
                request
            })
            .collect();
        let stored = self.adapters.emit_events_fenced_all(requests, &fence).await;
        fence.observe(&stored);
        let events = stored.map_err(backend)?;
        if events.len() != count {
            return Err(backend(format!(
                "the control plane returned {} of {count} stored events",
                events.len()
            )));
        }
        if let Some(tail) = &self.tail {
            tail.appended(&events);
        }
        Ok(events)
    }

    async fn append_conditional(
        &self,
        _request: EventRequest,
        _expected_last_sequence: Option<i32>,
    ) -> std::result::Result<Event, EventLogError> {
        // The actor lease is this log's single-writer guarantee.
        Err(EventLogError::InvalidAppend {
            detail: "platform session logs are fenced by the actor lease".into(),
        })
    }

    fn durability(&self) -> EventDurability {
        EventDurability::CrashDurable
    }
}

/// The Engine's session record for one activation, with the session's live
/// configuration applied on read.
pub struct PlatformSessionStore<A: WorkerAdapters> {
    adapters: A,
    org_id: i64,
    records: RwLock<HashMap<SessionId, ExecutionSession>>,
    configuration: SessionConfiguration,
}

impl<A: WorkerAdapters> PlatformSessionStore<A> {
    pub fn new(adapters: A, org_id: i64, configuration: SessionConfiguration) -> Self {
        Self {
            adapters,
            org_id,
            records: RwLock::default(),
            configuration,
        }
    }
}

#[async_trait]
impl<A: WorkerAdapters> SessionStore for PlatformSessionStore<A> {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        let mut record = self
            .records
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(&session_id)
            .cloned();
        if let Some(record) = record.as_mut() {
            self.configuration.apply(record);
        }
        Ok(record)
    }
}

#[async_trait]
impl<A: WorkerAdapters> SessionMutator for PlatformSessionStore<A> {
    async fn update_session_title(
        &self,
        session_id: SessionId,
        title: String,
    ) -> Result<ExecutionSession> {
        self.adapters
            .set_session_title(self.org_id, session_id.uuid(), title.clone())
            .await?;
        let mut records = self.records.write().unwrap_or_else(|p| p.into_inner());
        let record = records.get_mut(&session_id).ok_or(
            everruns_contracts::error::AgentLoopError::SessionNotFound(session_id),
        )?;
        record.title = Some(title);
        Ok(record.clone())
    }
}

#[async_trait]
impl<A: WorkerAdapters> RuntimeSessionStore for PlatformSessionStore<A> {
    async fn add_session(&self, session: ExecutionSession) -> Result<()> {
        self.records
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(session.id, session);
        Ok(())
    }

    /// The platform reads a session's state from its record (the sessions
    /// page, waits for idle, tool-approval prompts), so it goes to the
    /// control plane as on the worker host.
    async fn set_execution_state(
        &self,
        session_id: SessionId,
        state: everruns_contracts::runtime::SessionExecutionState,
    ) -> Result<()> {
        self.adapters
            .set_session_status(self.org_id, session_id.uuid(), &state.to_string())
            .await
    }
}

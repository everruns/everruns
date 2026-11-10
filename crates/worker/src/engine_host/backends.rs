//! The Engine's storage ports for a platform session, over the worker's
//! adapters (data plane v2 step 4).
//!
//! Decisions:
//! - The session's event log is the platform's: the same journal the
//!   worker's own host folds. Reads page the actor journal; writes are
//!   fenced by the actor lease exactly as the worker's own turns are, so a
//!   holder that lost its lease writes nothing and its driver hears of it
//!   (`ActorFence::observe`).
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
use everruns_contracts::typed_id::SessionId;

use crate::actor_lease_store::{ActorFence, SessionActorStore};
use crate::core::events::{Event, EventRequest};
use crate::core::host::{
    EventCursor, EventDurability, EventLog, EventLogError, EventPage, EventReadRequest,
    EventReader, RuntimeSessionStore, SessionMutator,
};
use crate::core::{ExecutionSession, SessionStore};
use crate::worker_adapters::WorkerAdapters;

fn backend(error: impl std::fmt::Display) -> EventLogError {
    EventLogError::Backend {
        detail: error.to_string(),
    }
}

/// A platform session's journal. Without a fence it is a reader: appends
/// are refused.
pub struct PlatformEventLog<A: WorkerAdapters> {
    adapters: A,
    journal: Arc<dyn SessionActorStore>,
    /// The lease writes present; the epoch swaps in the driver's fence (its
    /// lost signal) with this worker's holder.
    fence: Option<Arc<Mutex<ActorFence>>>,
}

impl<A: WorkerAdapters> PlatformEventLog<A> {
    pub fn writer(
        adapters: A,
        journal: Arc<dyn SessionActorStore>,
        fence: Arc<Mutex<ActorFence>>,
    ) -> Self {
        Self {
            adapters,
            journal,
            fence: Some(fence),
        }
    }

    pub fn reader(adapters: A, journal: Arc<dyn SessionActorStore>) -> Self {
        Self {
            adapters,
            journal,
            fence: None,
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
        let limit = u32::try_from(request.limit().get()).unwrap_or(u32::MAX);
        let journal = self
            .journal
            .read_actor_journal(session, Some(after), limit)
            .await
            .map_err(backend)?;
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
}

#[async_trait]
impl<A: WorkerAdapters> EventLog for PlatformEventLog<A> {
    async fn append(&self, request: EventRequest) -> std::result::Result<Event, EventLogError> {
        let fence = self.fence()?;
        let stored = self
            .adapters
            .emit_events_fenced(vec![request], &fence)
            .await;
        fence.observe(&stored);
        stored
            .map_err(backend)?
            .ok_or_else(|| backend("the control plane did not return the stored event"))
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
}

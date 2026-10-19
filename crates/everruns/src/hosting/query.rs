//! Consistent context reads use pinned authority events and scratch execution
//! metadata. They never claim an epoch, append history, or dispatch a model.
use super::*;
use everruns_core::{
    events::{Event, EventRequest},
    host::{
        EventCursor, EventDurability, EventLog, EventLogError, EventPage, EventReadRequest,
        EventReader,
    },
};

/// One authority transaction captures the canonical boundary and its execution
/// view. Trusted Agent/Harness definitions still come from the Engine registry.
#[derive(Clone, Serialize, Deserialize)]
pub struct SessionQuerySnapshot {
    /// Client configuration captured in the same transaction as the journal boundary.
    #[serde(default)]
    pub configuration: SessionConfiguration,
    /// Immutable retained behavior and host-services reference.
    pub binding: SessionBinding,
    /// Canonical prefix visible to every read during this query.
    pub high_watermark: i32,
    /// Mutable execution settings as of that prefix, if execution has started.
    pub execution: Option<super::spi::ExecutionSession>,
    /// Inputs explicitly rejected before model/tool dispatch.
    #[serde(default)]
    pub rejected_inputs: Vec<MessageId>,
}

pub(crate) struct SnapshotLog {
    inner: Arc<dyn EventLog>,
    session: SessionId,
    through: i32,
}
impl SnapshotLog {
    pub(crate) fn new(inner: Arc<dyn EventLog>, session: SessionId, through: i32) -> Self {
        Self {
            inner,
            session,
            through,
        }
    }
}
#[async_trait]
impl EventReader for SnapshotLog {
    async fn read_page(
        &self,
        request: EventReadRequest,
    ) -> std::result::Result<EventPage, EventLogError> {
        if request.session_id() != self.session {
            return Err(EventLogError::InvalidRead {
                detail: "query snapshot belongs to another session".into(),
            });
        }
        if request
            .cursor()
            .is_some_and(|cursor| cursor.session_id() != self.session)
        {
            return Err(EventLogError::CrossSessionCursor {
                detail: "cursor belongs to another session".into(),
            });
        }
        let after = request.cursor().map_or(0, EventCursor::after_sequence);
        let through = request
            .cursor()
            .and_then(EventCursor::snapshot_high_watermark)
            .unwrap_or(self.through);
        if through > self.through {
            return Err(EventLogError::InvalidRead {
                detail: "query cursor exceeds its captured snapshot".into(),
            });
        }
        self.inner
            .read_page(EventReadRequest::from_cursor(
                EventCursor::continuation(self.session, after, through)?,
                request.limit(),
            ))
            .await
    }
}
#[async_trait]
impl EventLog for SnapshotLog {
    async fn append(&self, request: EventRequest) -> std::result::Result<Event, EventLogError> {
        if request.session_id == self.session && request.is_dreaming_metadata() {
            return self.inner.append(request).await;
        }

        Err(EventLogError::InvalidAppend {
            detail: "context query cannot append canonical events".into(),
        })
    }
    async fn append_conditional(
        &self,
        request: EventRequest,
        _: Option<i32>,
    ) -> std::result::Result<Event, EventLogError> {
        self.append(request).await
    }
    fn durability(&self) -> EventDurability {
        self.inner.durability()
    }
}

/// Only provider context excludes rejected inputs. Public canonical history and
/// the actor's admission journal remain complete and retain their sequences.
pub(crate) struct InputFilterLog {
    inner: Arc<dyn EventLog>,
    rejected: std::collections::HashSet<MessageId>,
}
impl InputFilterLog {
    pub(crate) fn new(
        inner: Arc<dyn EventLog>,
        rejected: std::collections::HashSet<MessageId>,
    ) -> Self {
        Self { inner, rejected }
    }
}
#[async_trait]
impl EventReader for InputFilterLog {
    async fn read_page(
        &self,
        mut request: EventReadRequest,
    ) -> std::result::Result<EventPage, EventLogError> {
        loop {
            let limit = request.limit();
            let session = request.session_id();
            let page = self.inner.read_page(request).await?;
            let high = page.snapshot_high_watermark();
            let events:Vec<_>=page.events.into_iter().filter(|event|!matches!(&event.data,everruns_core::events::EventData::InputMessage(data) if self.rejected.contains(&data.message.id))).collect();
            if events.is_empty() {
                if let Some(cursor) = page.next_cursor {
                    request = EventReadRequest::from_cursor(cursor, limit);
                    continue;
                }
                return EventPage::new(events, None, high);
            }
            let next = if page.next_cursor.is_some() {
                Some(EventCursor::continuation(
                    session,
                    events
                        .last()
                        .and_then(|event| event.sequence)
                        .ok_or_else(|| EventLogError::Corruption {
                            detail: "canonical event has no sequence".into(),
                        })?,
                    high,
                )?)
            } else {
                None
            };
            return EventPage::new(events, next, high);
        }
    }
}
#[async_trait]
impl EventLog for InputFilterLog {
    async fn append(&self, request: EventRequest) -> std::result::Result<Event, EventLogError> {
        self.inner.append(request).await
    }
    async fn append_conditional(
        &self,
        request: EventRequest,
        expected: Option<i32>,
    ) -> std::result::Result<Event, EventLogError> {
        self.inner.append_conditional(request, expected).await
    }
    fn durability(&self) -> EventDurability {
        self.inner.durability()
    }
}

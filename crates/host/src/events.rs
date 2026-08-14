//! Canonical host event persistence, bounded replay, and message projection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use everruns_core::error::{AgentLoopError, Result as CoreResult};
use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{Event, EventData, EventRequest, OutputMessageCompletedData};
use everruns_core::message::{ContentPart, Message};
use everruns_core::message_filter::{MessageFilter, MessageQuery};
use everruns_core::message_retriever::{MessageHistory, MessageRetriever};
use everruns_core::tools::ToolResultImage;
use everruns_core::typed_id::{EventId, MessageId, SessionId};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, RwLock};

/// Default number of canonical envelopes returned by a host event read.
pub const DEFAULT_EVENT_READ_LIMIT: usize = 256;
/// Hard upper bound for one host event read.
pub const MAX_EVENT_PAGE_SIZE: usize = 1024;
/// Maximum envelopes one [`EventHistory`] projection may examine in one call.
pub const MAX_EVENT_HISTORY_REPLAY: usize = 100_000;
/// Maximum projected messages returned by one bounded history page.
pub const MAX_EVENT_HISTORY_PAGE_SIZE: usize = 256;

/// Persistence guarantee provided by an [`EventLog`] implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventDurability {
    /// Accepted writes survive for the lifetime of the log instance only.
    Volatile,
    /// Accepted writes survive process crashes according to the backend contract.
    CrashDurable,
}

/// Errors from canonical event append or bounded replay.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum EventLogError {
    /// A cursor, snapshot, session binding, or limit was invalid.
    #[error("invalid event read: {detail}")]
    InvalidRead { detail: String },
    /// A cursor was presented for a different session.
    #[error("event cursor belongs to another session: {detail}")]
    CrossSessionCursor { detail: String },
    /// A cursor's position and stable snapshot are internally inconsistent.
    #[error("incompatible event cursor: {detail}")]
    IncompatibleCursor { detail: String },
    /// The cursor references a snapshot no longer available from this reader.
    #[error("expired event cursor: {detail}")]
    ExpiredCursor { detail: String },
    /// The request cannot be appended to a durable log.
    #[error("invalid event append: {detail}")]
    InvalidAppend { detail: String },
    /// Persisted canonical envelopes conflict or are malformed.
    #[error("event log corruption: {detail}")]
    Corruption { detail: String },
    /// The storage backend failed.
    #[error("event log backend failure: {detail}")]
    Backend { detail: String },
}

impl From<std::io::Error> for EventLogError {
    fn from(error: std::io::Error) -> Self {
        Self::Backend {
            detail: error.to_string(),
        }
    }
}

/// Validated bound for one [`EventReader::read_page`] call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventReadLimit(u16);

impl EventReadLimit {
    /// Validate a non-zero read limit no larger than [`MAX_EVENT_PAGE_SIZE`].
    pub fn new(limit: usize) -> Result<Self, EventLogError> {
        if limit == 0 || limit > MAX_EVENT_PAGE_SIZE {
            return Err(EventLogError::InvalidRead {
                detail: format!("limit must be between 1 and {MAX_EVENT_PAGE_SIZE}, got {limit}"),
            });
        }
        Ok(Self(limit as u16))
    }

    /// Return the validated limit as a `usize`.
    pub fn get(self) -> usize {
        self.0 as usize
    }
}

impl Default for EventReadLimit {
    fn default() -> Self {
        Self(DEFAULT_EVENT_READ_LIMIT as u16)
    }
}

/// Snapshot-bound continuation cursor for canonical event replay.
///
/// A cursor is always bound to one session and carries a replay position plus
/// an optional pinned snapshot:
///
/// - [`EventCursor::continuation`] keeps an in-progress snapshot read pinned to
///   the high-watermark the first page captured, so later appends stay
///   invisible until that snapshot is exhausted.
/// - [`EventCursor::after`] carries no watermark. The next read captures the
///   then-current high-watermark, which is how a caller polls for appends made
///   after a completed snapshot.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventCursor {
    session_id: SessionId,
    after_sequence: i32,
    snapshot_high_watermark: Option<i32>,
}

impl EventCursor {
    /// Session this continuation is bound to.
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Last sequence returned before this continuation.
    pub fn after_sequence(&self) -> i32 {
        self.after_sequence
    }

    /// Stable high-watermark captured by the first page.
    ///
    /// `Some` means the cursor stays pinned to that snapshot; `None` means the
    /// next read starts a fresh snapshot (the polling form).
    pub fn snapshot_high_watermark(&self) -> Option<i32> {
        self.snapshot_high_watermark
    }

    /// Start a new snapshot after a previously observed high-watermark.
    ///
    /// This is the polling form: the next read captures the then-current
    /// high-watermark instead of remaining pinned to an earlier snapshot.
    pub fn after(session_id: SessionId, after_sequence: i32) -> Result<Self, EventLogError> {
        if after_sequence < 0 {
            return Err(EventLogError::InvalidRead {
                detail: "poll cursor sequence cannot be negative".into(),
            });
        }
        Ok(Self {
            session_id,
            after_sequence,
            snapshot_high_watermark: None,
        })
    }

    /// Continue reading inside an already captured snapshot.
    ///
    /// `after_sequence` is the last sequence the caller has consumed and
    /// `snapshot_high_watermark` is the value the first page of that snapshot
    /// reported through [`EventPage::snapshot_high_watermark`]. Reads resumed
    /// from this cursor never observe appends committed after that watermark.
    ///
    /// This is the cursor an [`EventReader`] returns as
    /// [`EventPage::next_cursor`]. Use [`EventCursor::after`] instead to start a
    /// new snapshot once the pinned one is exhausted.
    pub fn continuation(
        session_id: SessionId,
        after_sequence: i32,
        snapshot_high_watermark: i32,
    ) -> Result<Self, EventLogError> {
        if after_sequence < 0 {
            return Err(EventLogError::InvalidRead {
                detail: "continuation cursor sequence cannot be negative".into(),
            });
        }
        if snapshot_high_watermark < 0 {
            return Err(EventLogError::InvalidRead {
                detail: "snapshot high-watermark cannot be negative".into(),
            });
        }
        if after_sequence > snapshot_high_watermark {
            return Err(EventLogError::IncompatibleCursor {
                detail: format!(
                    "cursor position {after_sequence} exceeds its snapshot {snapshot_high_watermark}"
                ),
            });
        }
        Ok(Self {
            session_id,
            after_sequence,
            snapshot_high_watermark: Some(snapshot_high_watermark),
        })
    }
}

/// One bounded canonical event read.
#[derive(Clone, Debug)]
pub struct EventReadRequest {
    session_id: SessionId,
    cursor: Option<EventCursor>,
    limit: EventReadLimit,
}

impl EventReadRequest {
    /// Start a stable snapshot read for `session_id`.
    pub fn new(session_id: SessionId, limit: EventReadLimit) -> Self {
        Self {
            session_id,
            cursor: None,
            limit,
        }
    }

    /// Continue the stable snapshot represented by `cursor`.
    pub fn from_cursor(cursor: EventCursor, limit: EventReadLimit) -> Self {
        Self {
            session_id: cursor.session_id,
            cursor: Some(cursor),
            limit,
        }
    }

    /// Attach a cursor while retaining the explicitly selected session.
    ///
    /// Readers reject a cursor originating from another session.
    pub fn with_cursor(mut self, cursor: EventCursor) -> Self {
        self.cursor = Some(cursor);
        self
    }

    /// Session selected for this read.
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Continuation this read resumes from, if any.
    ///
    /// `None` is an initial read: the reader captures a fresh snapshot
    /// high-watermark. `Some` carries the caller's position and — when
    /// [`EventCursor::snapshot_high_watermark`] is set — the snapshot the read
    /// must stay pinned to.
    pub fn cursor(&self) -> Option<&EventCursor> {
        self.cursor.as_ref()
    }

    /// Validated page bound.
    pub fn limit(&self) -> EventReadLimit {
        self.limit
    }
}

/// A page of full canonical event envelopes in persisted sequence order.
///
/// Construct pages with [`EventPage::new`], which enforces the invariants every
/// [`EventReader`] must observe: sequences are positive, strictly increasing,
/// and bounded by the page snapshot; all events belong to one session; and a
/// continuation stays pinned to that same snapshot with more to return.
#[derive(Clone, Debug)]
pub struct EventPage {
    /// Full canonical envelopes, never projection records.
    pub events: Vec<Event>,
    /// Continuation within the first page's stable snapshot.
    pub next_cursor: Option<EventCursor>,
    snapshot_high_watermark: i32,
}

/// Validated projected-message bound for [`EventHistory::read_page`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventHistoryReadLimit(u16);

impl EventHistoryReadLimit {
    /// Validate a non-zero message limit no larger than
    /// [`MAX_EVENT_HISTORY_PAGE_SIZE`].
    pub fn new(limit: usize) -> Result<Self, EventLogError> {
        if limit == 0 || limit > MAX_EVENT_HISTORY_PAGE_SIZE {
            return Err(EventLogError::InvalidRead {
                detail: format!(
                    "history limit must be between 1 and {MAX_EVENT_HISTORY_PAGE_SIZE}, got {limit}"
                ),
            });
        }
        Ok(Self(limit as u16))
    }

    /// Validated projected-message count.
    pub fn get(self) -> usize {
        self.0 as usize
    }
}

impl Default for EventHistoryReadLimit {
    fn default() -> Self {
        Self(MAX_EVENT_HISTORY_PAGE_SIZE as u16)
    }
}

/// One bounded projected-history read.
#[derive(Clone, Debug)]
pub struct EventHistoryReadRequest {
    session_id: SessionId,
    cursor: Option<EventCursor>,
    limit: EventHistoryReadLimit,
}

impl EventHistoryReadRequest {
    /// Start a projected-message snapshot for `session_id`.
    pub fn new(session_id: SessionId, limit: EventHistoryReadLimit) -> Self {
        Self {
            session_id,
            cursor: None,
            limit,
        }
    }

    /// Continue or poll using a session-bound event cursor.
    pub fn with_cursor(mut self, cursor: EventCursor) -> Self {
        self.cursor = Some(cursor);
        self
    }

    /// Selected session.
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Validated message-page bound.
    pub fn limit(&self) -> EventHistoryReadLimit {
        self.limit
    }
}

/// Bounded message projection and its stable event snapshot continuation.
#[derive(Clone, Debug)]
pub struct EventHistoryPage {
    /// Canonical messages in persisted event sequence order.
    pub messages: Vec<Message>,
    /// Continuation after the last examined canonical envelope.
    pub next_cursor: Option<EventCursor>,
    snapshot_high_watermark: i32,
}

impl EventHistoryPage {
    /// Stable canonical-event high-watermark used for this page.
    pub fn snapshot_high_watermark(&self) -> i32 {
        self.snapshot_high_watermark
    }
}

impl EventPage {
    /// High-watermark captured by the first read (`0` for an empty session).
    pub fn snapshot_high_watermark(&self) -> i32 {
        self.snapshot_high_watermark
    }

    /// Build a validated page of canonical events.
    ///
    /// This is the construction path for every [`EventReader`], in-crate or
    /// external, so all implementations expose the same observable invariants:
    ///
    /// - `snapshot_high_watermark` is the sequence the snapshot is pinned to.
    ///   `0` means the session has no durable events; such a page must be empty
    ///   and must not offer a continuation.
    /// - `events` are complete canonical envelopes carrying durable sequences.
    ///   Sequences are positive, strictly increasing, and never exceed the
    ///   snapshot. Gaps are allowed: a reader may project an append-only
    ///   physical log into a filtered logical sequence.
    /// - All events belong to one session.
    /// - `next_cursor`, when present, promises more events inside this same
    ///   snapshot. It must belong to the page's session, carry exactly this
    ///   `snapshot_high_watermark`, and sit at or after the last returned
    ///   sequence but strictly before the snapshot. Build it with
    ///   [`EventCursor::continuation`]. Return `None` once the snapshot is
    ///   exhausted; callers poll for later appends with [`EventCursor::after`].
    ///
    /// Violations return the typed [`EventLogError`] variant that describes the
    /// inconsistency rather than panicking.
    pub fn new(
        events: Vec<Event>,
        next_cursor: Option<EventCursor>,
        snapshot_high_watermark: i32,
    ) -> Result<Self, EventLogError> {
        if snapshot_high_watermark < 0 {
            return Err(EventLogError::InvalidRead {
                detail: format!(
                    "page snapshot high-watermark cannot be negative, got {snapshot_high_watermark}"
                ),
            });
        }
        let mut page_session: Option<SessionId> = None;
        let mut previous = 0i32;
        for event in &events {
            let sequence = event.sequence.ok_or_else(|| EventLogError::Corruption {
                detail: format!("event {} in a read page has no sequence", event.id),
            })?;
            if sequence <= 0 {
                return Err(EventLogError::Corruption {
                    detail: format!("event {} has non-positive sequence {sequence}", event.id),
                });
            }
            if sequence <= previous {
                return Err(EventLogError::Corruption {
                    detail: format!(
                        "page sequence {sequence} does not follow {previous} for event {}",
                        event.id
                    ),
                });
            }
            if sequence > snapshot_high_watermark {
                return Err(EventLogError::InvalidRead {
                    detail: format!(
                        "page sequence {sequence} exceeds its snapshot {snapshot_high_watermark}"
                    ),
                });
            }
            match page_session {
                None => page_session = Some(event.session_id),
                Some(session_id) if session_id != event.session_id => {
                    return Err(EventLogError::Corruption {
                        detail: format!(
                            "page mixes sessions {session_id} and {}",
                            event.session_id
                        ),
                    });
                }
                Some(_) => {}
            }
            previous = sequence;
        }
        if let Some(cursor) = &next_cursor {
            if let Some(session_id) = page_session
                && cursor.session_id != session_id
            {
                return Err(EventLogError::CrossSessionCursor {
                    detail: format!(
                        "continuation for session {} cannot follow a page of session {session_id}",
                        cursor.session_id
                    ),
                });
            }
            if cursor.snapshot_high_watermark != Some(snapshot_high_watermark) {
                return Err(EventLogError::IncompatibleCursor {
                    detail: format!(
                        "continuation must stay pinned to snapshot {snapshot_high_watermark}"
                    ),
                });
            }
            if cursor.after_sequence < previous {
                return Err(EventLogError::IncompatibleCursor {
                    detail: format!(
                        "continuation position {} rewinds before the last returned sequence {previous}",
                        cursor.after_sequence
                    ),
                });
            }
            if cursor.after_sequence >= snapshot_high_watermark {
                return Err(EventLogError::IncompatibleCursor {
                    detail: format!(
                        "continuation position {} leaves nothing in snapshot {snapshot_high_watermark}",
                        cursor.after_sequence
                    ),
                });
            }
        }
        Ok(Self {
            events,
            next_cursor,
            snapshot_high_watermark,
        })
    }
}

/// Bounded, ordered reader for canonical session events.
///
/// This trait is part of the public host SPI: an external crate can implement
/// it against its own storage using only public APIs. Implementations answer
/// three request shapes, distinguished through [`EventReadRequest::cursor`]:
///
/// - **Initial snapshot read** — no cursor. Capture the session's current
///   high-watermark, return up to [`EventReadRequest::limit`] events from
///   sequence `1` upward, and report the captured watermark as
///   [`EventPage::snapshot_high_watermark`]. An empty session reports `0`.
/// - **Continuation** — a cursor whose
///   [`EventCursor::snapshot_high_watermark`] is `Some`. Stay pinned to that
///   watermark and return events after [`EventCursor::after_sequence`] up to
///   and including it. Appends committed after the snapshot was captured must
///   remain invisible, so paging neither skips nor duplicates.
/// - **Poll** — a cursor whose snapshot high-watermark is `None`, produced by
///   [`EventCursor::after`]. Capture a fresh high-watermark and return events
///   after the cursor position, which is how callers observe appends made since
///   a completed snapshot.
///
/// A cursor is bound to one session: reject one presented for a different
/// session with [`EventLogError::CrossSessionCursor`], a position that cannot
/// belong to its snapshot with [`EventLogError::IncompatibleCursor`], and a
/// snapshot this reader can no longer serve with
/// [`EventLogError::ExpiredCursor`]. Build results with [`EventPage::new`],
/// which validates those page and cursor invariants for every implementation.
///
/// Sequences increase monotonically per session but need not be contiguous:
/// gaps are expected when a reader projects an append-only physical log into a
/// filtered logical event sequence.
#[async_trait]
pub trait EventReader: Send + Sync {
    /// Read one page. The first page fixes a snapshot high-watermark; continuations
    /// cannot observe concurrent appends and therefore neither skip nor duplicate.
    async fn read_page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError>;
}

/// A coherent canonical event log.
///
/// Implementations explicitly pair their reader and writer and guarantee
/// read-your-accepted-writes, unique monotonic per-session sequences, and the
/// declared durability class. There is deliberately no blanket implementation.
///
/// This trait is part of the public host SPI. An external crate implements
/// [`EventReader`] plus [`EventLog::append`] and supplies the result to
/// composition through [`HostBackends::with_event_log`](crate::HostBackends::with_event_log).
/// The append side owns identity: the log assigns the [`EventId`] and the next
/// per-session sequence, finalizes the request with
/// [`EventRequest::into_event`], and returns that canonical [`Event`]. An
/// accepted append must be visible to the next read of the same session.
/// Ephemeral requests ([`EventRequest::is_ephemeral`]) never reach a durable
/// log; reject them with [`EventLogError::InvalidAppend`].
///
/// The log is append-only. There is no truncate, rewind, or mutation contract.
#[async_trait]
pub trait EventLog: EventReader {
    /// Commit a durable request, assigning its event id and persisted sequence.
    async fn append(&self, request: EventRequest) -> Result<Event, EventLogError>;

    /// Persistence guarantee used for acknowledged appends.
    fn durability(&self) -> EventDurability;
}

/// Non-blocking destination for already-finalized canonical envelopes.
pub trait EventSink: Send + Sync {
    /// Attempt immediate observational delivery without delaying execution.
    fn try_send(&self, event: Event) -> Result<(), EventSinkError>;
}

/// Observational delivery failures. They never change append or turn success.
#[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq)]
pub enum EventSinkError {
    /// The bounded sink cannot accept another envelope now.
    #[error("event sink is full")]
    Full {
        /// Number of envelopes the sink reports dropping.
        dropped: u64,
    },
    /// The sink has no remaining receiver.
    #[error("event sink is closed")]
    Closed,
}

/// Sink used when a host has no live observers.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn try_send(&self, _event: Event) -> Result<(), EventSinkError> {
        Ok(())
    }
}

/// Counts observational failures without making them execution failures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventDeliveryStats {
    /// Envelopes rejected because the sink was full.
    pub full: u64,
    /// Envelopes rejected because the sink was closed.
    pub closed: u64,
}

/// Canonical event router shared by in-process, worker, and facade hosts.
///
/// Durable requests commit to the log before notification. Ephemeral requests
/// use the protocol's [`EventRequest::is_ephemeral`] classification, receive
/// `sequence = None`, and are sink-only. Per-session serialization preserves
/// append acceptance order through the post-commit sink call.
#[derive(Clone)]
pub struct HostEventEmitter {
    log: Arc<dyn EventLog>,
    sink: Arc<dyn EventSink>,
    session_locks: Arc<Mutex<HashMap<SessionId, Weak<Mutex<()>>>>>,
    full: Arc<AtomicU64>,
    closed: Arc<AtomicU64>,
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
        }
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
    async fn emit(&self, request: EventRequest) -> CoreResult<Event> {
        let session_id = request.session_id;
        let lock = self.session_lock(session_id).await;
        let _guard = lock.lock().await;
        let event = if request.is_ephemeral() {
            ephemeral_event(request)
        } else {
            self.log
                .append(request)
                .await
                .map_err(|error| AgentLoopError::store(error.to_string()))?
        };
        self.notify(event.clone());
        Ok(event)
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

#[derive(Default)]
struct EventIndex {
    by_session: HashMap<SessionId, Vec<Event>>,
    by_id: HashMap<EventId, Vec<u8>>,
    by_sequence: HashMap<(SessionId, i32), EventId>,
}

impl EventIndex {
    fn next_sequence(&self, session_id: SessionId) -> Result<i32, EventLogError> {
        self.by_session
            .get(&session_id)
            .and_then(|events| events.last())
            .and_then(|event| event.sequence)
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| EventLogError::InvalidAppend {
                detail: "session sequence exhausted".into(),
            })
    }

    fn insert_existing(&mut self, event: Event) -> Result<bool, EventLogError> {
        let sequence = event.sequence.ok_or_else(|| EventLogError::Corruption {
            detail: format!("durable event {} has no sequence", event.id),
        })?;
        if sequence <= 0 {
            return Err(EventLogError::Corruption {
                detail: format!("event {} has non-positive sequence {sequence}", event.id),
            });
        }
        let canonical = serde_json::to_vec(&event).map_err(|error| EventLogError::Corruption {
            detail: error.to_string(),
        })?;
        if let Some(existing) = self.by_id.get(&event.id) {
            if existing == &canonical {
                return Ok(false);
            }
            return Err(EventLogError::Corruption {
                detail: format!("event id {} has conflicting canonical envelopes", event.id),
            });
        }
        if let Some(existing_id) = self.by_sequence.get(&(event.session_id, sequence)) {
            return Err(EventLogError::Corruption {
                detail: format!(
                    "session {} sequence {sequence} conflicts between {} and {}",
                    event.session_id, existing_id, event.id
                ),
            });
        }
        if let Some(previous) = self
            .by_session
            .get(&event.session_id)
            .and_then(|events| events.last())
            .and_then(|event| event.sequence)
            && sequence <= previous
        {
            return Err(EventLogError::Corruption {
                detail: format!(
                    "session {} sequence {sequence} follows {previous}",
                    event.session_id
                ),
            });
        }
        self.by_id.insert(event.id, canonical);
        self.by_sequence
            .insert((event.session_id, sequence), event.id);
        self.by_session
            .entry(event.session_id)
            .or_default()
            .push(event);
        Ok(true)
    }

    fn page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError> {
        let events = self
            .by_session
            .get(&request.session_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let current_high = events.last().and_then(|event| event.sequence);
        let (after, snapshot) = match request.cursor {
            Some(cursor) => {
                if cursor.session_id != request.session_id {
                    return Err(EventLogError::CrossSessionCursor {
                        detail: "cursor belongs to another session".into(),
                    });
                }
                let snapshot = cursor
                    .snapshot_high_watermark
                    .unwrap_or(current_high.unwrap_or(0));
                if cursor.after_sequence > snapshot {
                    return Err(EventLogError::IncompatibleCursor {
                        detail: "cursor position exceeds its snapshot".into(),
                    });
                }
                if snapshot > current_high.unwrap_or(0) {
                    return Err(EventLogError::ExpiredCursor {
                        detail: "cursor snapshot is not available in this log".into(),
                    });
                }
                (cursor.after_sequence, snapshot)
            }
            None => (0, current_high.unwrap_or(0)),
        };
        if snapshot == 0 {
            return EventPage::new(Vec::new(), None, 0);
        }
        let mut selected = events
            .iter()
            .filter(|event| {
                event
                    .sequence
                    .is_some_and(|sequence| sequence > after && sequence <= snapshot)
            })
            .take(request.limit.get() + 1)
            .cloned()
            .collect::<Vec<_>>();
        let has_more = selected.len() > request.limit.get();
        if has_more {
            selected.pop();
        }
        let next_cursor = has_more
            .then(|| {
                EventCursor::continuation(
                    request.session_id,
                    selected
                        .last()
                        .and_then(|event| event.sequence)
                        .expect("a page with more events returned at least one event"),
                    snapshot,
                )
            })
            .transpose()?;
        EventPage::new(selected, next_cursor, snapshot)
    }
}

/// Volatile coherent event log used by default in-process hosts.
#[derive(Default)]
pub struct InMemoryEventLog {
    index: RwLock<EventIndex>,
}

impl InMemoryEventLog {
    /// Create an empty volatile log.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl EventReader for InMemoryEventLog {
    async fn read_page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError> {
        self.index.read().await.page(request)
    }
}

#[async_trait]
impl EventLog for InMemoryEventLog {
    async fn append(&self, request: EventRequest) -> Result<Event, EventLogError> {
        if request.is_ephemeral() {
            return Err(EventLogError::InvalidAppend {
                detail: format!(
                    "ephemeral event {} must be routed sink-only",
                    request.event_type
                ),
            });
        }
        let mut index = self.index.write().await;
        let sequence = index.next_sequence(request.session_id)?;
        let event = request.into_event(EventId::new(), sequence);
        index.insert_existing(event.clone())?;
        Ok(event)
    }

    fn durability(&self) -> EventDurability {
        EventDurability::Volatile
    }
}

struct JsonlState {
    file: tokio::fs::File,
    committed_len: u64,
    index: EventIndex,
}

/// Crash-durable JSONL log containing complete canonical [`Event`] envelopes.
///
/// An append is acknowledged only after newline-delimited bytes are flushed and
/// `sync_data` succeeds. A torn final record is an uncommitted tail and is
/// truncated on open; any malformed complete line is corruption. The in-memory
/// index is disposable and rebuilt solely from the canonical log.
pub struct JsonlEventLog {
    path: PathBuf,
    state: Mutex<JsonlState>,
}

impl JsonlEventLog {
    /// Open or create a canonical event JSONL file and rebuild its index.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, EventLogError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await?;
        }
        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        let committed_len = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |position| position + 1);
        let mut index = EventIndex::default();
        for (line_index, line) in bytes[..committed_len]
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .enumerate()
        {
            let event: Event =
                serde_json::from_slice(line).map_err(|error| EventLogError::Corruption {
                    detail: format!("line {}: {error}", line_index + 1),
                })?;
            index.insert_existing(event)?;
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.create(true).read(true).append(true);
        #[cfg(unix)]
        {
            // THREAT[TM-FS-014]: canonical envelopes can contain prompts,
            // tool results, and metadata; newly created logs are owner-only.
            options.mode(0o600);
        }
        let file = options.open(&path).await?;
        if bytes.len() != committed_len {
            file.set_len(committed_len as u64).await?;
            file.sync_data().await?;
        }
        Ok(Self {
            path,
            state: Mutex::new(JsonlState {
                file,
                committed_len: committed_len as u64,
                index,
            }),
        })
    }

    /// Backing canonical event-log path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[async_trait]
impl EventReader for JsonlEventLog {
    async fn read_page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError> {
        self.state.lock().await.index.page(request)
    }
}

#[async_trait]
impl EventLog for JsonlEventLog {
    async fn append(&self, request: EventRequest) -> Result<Event, EventLogError> {
        if request.is_ephemeral() {
            return Err(EventLogError::InvalidAppend {
                detail: format!(
                    "ephemeral event {} must be routed sink-only",
                    request.event_type
                ),
            });
        }
        let mut state = self.state.lock().await;
        let sequence = state.index.next_sequence(request.session_id)?;
        let event = request.into_event(EventId::new(), sequence);
        let mut encoded =
            serde_json::to_vec(&event).map_err(|error| EventLogError::InvalidAppend {
                detail: error.to_string(),
            })?;
        encoded.push(b'\n');
        let previous_len = state.committed_len;
        let write_result = async {
            state.file.write_all(&encoded).await?;
            state.file.flush().await?;
            state.file.sync_data().await
        }
        .await;
        if let Err(error) = write_result {
            let _ = state.file.set_len(previous_len).await;
            let _ = state.file.sync_data().await;
            return Err(EventLogError::Backend {
                detail: error.to_string(),
            });
        }
        state.committed_len += encoded.len() as u64;
        state.index.insert_existing(event.clone())?;
        Ok(event)
    }

    fn durability(&self) -> EventDurability {
        EventDurability::CrashDurable
    }
}

#[derive(Clone)]
struct ProjectedMessage {
    event_id: EventId,
    event_type: String,
    sequence: i32,
    tool_name: Option<String>,
    message: Message,
}

/// Read-only message/history projection rebuilt from canonical events.
#[derive(Clone)]
pub struct EventHistory {
    reader: Arc<dyn EventReader>,
}

impl EventHistory {
    /// Project messages from a bounded canonical event reader.
    pub fn new(reader: Arc<dyn EventReader>) -> Self {
        Self { reader }
    }

    /// Underlying canonical reader.
    pub fn event_reader(&self) -> Arc<dyn EventReader> {
        self.reader.clone()
    }

    /// Whether the canonical log contains any event for this session id.
    ///
    /// This reports history presence only. It is deliberately not a session
    /// identity/catalog check: a valid session can have no events, and retained
    /// events can outlive the session record that originally produced them.
    pub async fn has_history(&self, session_id: SessionId) -> Result<bool, EventLogError> {
        let page = self
            .reader
            .read_page(EventReadRequest::new(
                session_id,
                EventReadLimit::new(1).expect("one is a valid event read limit"),
            ))
            .await?;
        Ok(!page.events.is_empty())
    }

    pub(crate) async fn contains_event_type(
        &self,
        session_id: SessionId,
        event_type: &str,
    ) -> Result<bool, EventLogError> {
        let limit = EventReadLimit::default();
        let mut request = EventReadRequest::new(session_id, limit);
        let mut seen = 0usize;
        loop {
            let page = self.reader.read_page(request).await?;
            seen = seen.saturating_add(page.events.len());
            if page
                .events
                .iter()
                .any(|event| event.event_type == event_type)
            {
                return Ok(true);
            }
            if seen > MAX_EVENT_HISTORY_REPLAY {
                return Err(EventLogError::InvalidRead {
                    detail: format!(
                        "event-type replay exceeds the {MAX_EVENT_HISTORY_REPLAY}-event bound"
                    ),
                });
            }
            let Some(cursor) = page.next_cursor else {
                return Ok(false);
            };
            request = EventReadRequest::from_cursor(cursor, limit);
        }
    }

    /// Read one bounded projected-message page without collecting the session.
    ///
    /// Raw event reads stay bounded by the remaining message slots, lifecycle
    /// envelopes are skipped, the first event snapshot is retained across all
    /// internal reads, and no more than [`MAX_EVENT_HISTORY_REPLAY`] envelopes
    /// are examined.
    pub async fn read_page(
        &self,
        request: EventHistoryReadRequest,
    ) -> Result<EventHistoryPage, EventLogError> {
        let message_limit = request.limit.get();
        let first_raw_limit = EventReadLimit::new(message_limit.min(MAX_EVENT_PAGE_SIZE))?;
        let mut raw_request = EventReadRequest::new(request.session_id, first_raw_limit);
        if let Some(cursor) = request.cursor {
            raw_request = raw_request.with_cursor(cursor);
        }
        let mut messages = Vec::with_capacity(message_limit);
        let mut examined = 0usize;
        loop {
            let page = self.reader.read_page(raw_request).await?;
            let snapshot_high_watermark = page.snapshot_high_watermark();
            examined = examined.saturating_add(page.events.len());
            if examined > MAX_EVENT_HISTORY_REPLAY {
                return Err(EventLogError::InvalidRead {
                    detail: format!(
                        "history page examined more than {MAX_EVENT_HISTORY_REPLAY} events"
                    ),
                });
            }
            for event in page.events {
                if let Some(message) = message_from_event(&event) {
                    messages.push(message);
                }
            }
            if messages.len() >= message_limit {
                // A raw continuation can point only to trailing lifecycle
                // envelopes. Probe within the same bounded snapshot so the
                // projected cursor truthfully means another message exists and
                // callers never need an empty terminal page after an exact
                // message boundary.
                let boundary_cursor = page.next_cursor.clone();
                let mut next_cursor = None;
                if let Some(boundary_cursor) = boundary_cursor {
                    let mut probe_request = EventReadRequest::from_cursor(
                        boundary_cursor.clone(),
                        EventReadLimit::default(),
                    );
                    loop {
                        let probe = self.reader.read_page(probe_request).await?;
                        examined = examined.saturating_add(probe.events.len());
                        if examined > MAX_EVENT_HISTORY_REPLAY {
                            return Err(EventLogError::InvalidRead {
                                detail: format!(
                                    "history page examined more than {MAX_EVENT_HISTORY_REPLAY} events"
                                ),
                            });
                        }
                        if probe.events.iter().any(event_projects_message) {
                            next_cursor = Some(boundary_cursor);
                            break;
                        }
                        let Some(cursor) = probe.next_cursor else {
                            break;
                        };
                        probe_request =
                            EventReadRequest::from_cursor(cursor, EventReadLimit::default());
                    }
                }
                return Ok(EventHistoryPage {
                    messages,
                    next_cursor,
                    snapshot_high_watermark,
                });
            }
            if page.next_cursor.is_none() {
                return Ok(EventHistoryPage {
                    messages,
                    next_cursor: None,
                    snapshot_high_watermark,
                });
            }
            let remaining = message_limit - messages.len();
            let raw_limit = EventReadLimit::new(remaining.min(MAX_EVENT_PAGE_SIZE))?;
            raw_request = EventReadRequest::from_cursor(
                page.next_cursor.expect("checked continuation above"),
                raw_limit,
            );
        }
    }

    async fn project(&self, session_id: SessionId) -> Result<Vec<ProjectedMessage>, EventLogError> {
        let limit = EventReadLimit::default();
        let mut request = EventReadRequest::new(session_id, limit);
        let mut projected = Vec::new();
        let mut examined = 0usize;
        loop {
            let page = self.reader.read_page(request).await?;
            examined = examined.saturating_add(page.events.len());
            if examined > MAX_EVENT_HISTORY_REPLAY {
                return Err(EventLogError::InvalidRead {
                    detail: format!(
                        "history replay examined more than {MAX_EVENT_HISTORY_REPLAY} events"
                    ),
                });
            }
            for event in page.events {
                if let Some(message) = message_from_event(&event) {
                    projected.push(ProjectedMessage {
                        event_id: event.id,
                        event_type: event.event_type,
                        sequence: event.sequence.expect("reader returns durable events"),
                        tool_name: match &event.data {
                            EventData::ToolCompleted(data) => Some(data.tool_name.clone()),
                            _ => None,
                        },
                        message,
                    });
                }
            }
            let Some(cursor) = page.next_cursor else {
                break;
            };
            request = EventReadRequest::from_cursor(cursor, limit);
        }
        Ok(projected)
    }

    async fn filtered(&self, query: &MessageQuery) -> Result<Vec<ProjectedMessage>, EventLogError> {
        let mut projected = self.project(query.session_id).await?;
        if let Some(after) = query.after_sequence {
            projected.retain(|item| i64::from(item.sequence) > after);
        }
        for filter in &query.filters {
            match filter {
                MessageFilter::TimeRange { from, to } => projected.retain(|item| {
                    from.is_none_or(|from| item.message.created_at >= from)
                        && to.is_none_or(|to| item.message.created_at <= to)
                }),
                MessageFilter::EventTypes(types) => {
                    projected.retain(|item| types.contains(&item.event_type))
                }
                MessageFilter::ToolName(name) => {
                    projected.retain(|item| item.tool_name.as_ref() == Some(name))
                }
                MessageFilter::Search(search) => {
                    let search = search.to_lowercase();
                    projected.retain(|item| {
                        item.message
                            .text()
                            .is_some_and(|text| text.to_lowercase().contains(&search))
                    });
                }
                MessageFilter::ExcludeIds(ids) => {
                    projected.retain(|item| !ids.contains(&item.event_id))
                }
                MessageFilter::IncludeIds(ids) => {
                    projected.retain(|item| ids.contains(&item.event_id))
                }
                MessageFilter::Custom(predicate) => {
                    projected.retain(|item| predicate(&item.message))
                }
            }
        }
        Ok(projected)
    }
}

fn event_projects_message(event: &Event) -> bool {
    matches!(
        &event.data,
        EventData::InputMessage(_)
            | EventData::OutputMessageCompleted(_)
            | EventData::ToolCompleted(_)
    )
}

#[async_trait]
impl MessageRetriever for EventHistory {
    async fn get(
        &self,
        session_id: SessionId,
        message_id: MessageId,
    ) -> CoreResult<Option<Message>> {
        Ok(self
            .project(session_id)
            .await
            .map_err(core_event_error)?
            .into_iter()
            .find(|item| item.message.id == message_id)
            .map(|item| item.message))
    }

    async fn load(&self, session_id: SessionId) -> CoreResult<Vec<Message>> {
        Ok(self
            .project(session_id)
            .await
            .map_err(core_event_error)?
            .into_iter()
            .map(|item| item.message)
            .collect())
    }

    async fn load_filtered(&self, query: MessageQuery) -> CoreResult<Vec<Message>> {
        let mut messages = self
            .filtered(&query)
            .await
            .map_err(core_event_error)?
            .into_iter()
            .map(|item| item.message)
            .collect::<Vec<_>>();
        let count_before_limit = messages.len();
        query.apply_window_bounds(&mut messages);
        query.prepend_excluded_notice(&mut messages, count_before_limit);
        query.apply_injections(&mut messages);
        Ok(messages)
    }

    async fn load_filtered_history(&self, query: MessageQuery) -> CoreResult<MessageHistory> {
        let source_sequence = self
            .project(query.session_id)
            .await
            .map_err(core_event_error)?
            .last()
            .map(|item| i64::from(item.sequence));
        Ok(MessageHistory {
            messages: self.load_filtered(query).await?,
            source_sequence,
        })
    }

    async fn load_page(
        &self,
        session_id: SessionId,
        offset: usize,
        limit: usize,
    ) -> CoreResult<Vec<Message>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut cursor = None;
        let mut skipped = 0usize;
        let mut messages = Vec::with_capacity(limit.min(MAX_EVENT_HISTORY_PAGE_SIZE));
        loop {
            let requested = if skipped < offset {
                (offset - skipped).min(MAX_EVENT_HISTORY_PAGE_SIZE)
            } else {
                (limit - messages.len()).min(MAX_EVENT_HISTORY_PAGE_SIZE)
            };
            let mut request = EventHistoryReadRequest::new(
                session_id,
                EventHistoryReadLimit::new(requested).map_err(core_event_error)?,
            );
            if let Some(previous) = cursor {
                request = request.with_cursor(previous);
            }
            let page = self.read_page(request).await.map_err(core_event_error)?;
            if skipped < offset {
                skipped = skipped.saturating_add(page.messages.len());
            } else {
                messages.extend(page.messages);
            }
            cursor = page.next_cursor;
            if messages.len() >= limit || cursor.is_none() {
                messages.truncate(limit);
                return Ok(messages);
            }
        }
    }

    async fn count(&self, session_id: SessionId) -> CoreResult<usize> {
        Ok(self
            .project(session_id)
            .await
            .map_err(core_event_error)?
            .len())
    }
}

fn core_event_error(error: EventLogError) -> AgentLoopError {
    AgentLoopError::store(error.to_string())
}

fn message_from_event(event: &Event) -> Option<Message> {
    match &event.data {
        EventData::InputMessage(data) => Some(data.message.clone()),
        EventData::OutputMessageCompleted(OutputMessageCompletedData { message, .. }) => {
            Some(message.clone())
        }
        EventData::ToolCompleted(data) => {
            let mut message = tool_completed_to_message(data.clone());
            message.id = MessageId::from_uuid(event.id.uuid());
            message.created_at = event.ts;
            Some(message)
        }
        // `output.message.replaced` is live/persisted narration only. The later
        // completed envelope supplies the one canonical assistant message.
        _ => None,
    }
}

fn tool_completed_to_message(data: everruns_core::events::ToolCompletedData) -> Message {
    let mut images = Vec::<ToolResultImage>::new();
    let result = data.result.map(|parts| {
        for part in &parts {
            if let ContentPart::Image(image) = part
                && let (Some(base64), Some(media_type)) = (&image.base64, &image.media_type)
            {
                images.push(ToolResultImage {
                    base64: base64.clone(),
                    media_type: media_type.clone(),
                });
            }
        }
        let text_parts = parts
            .iter()
            .filter(|part| matches!(part, ContentPart::Text(_)))
            .collect::<Vec<_>>();
        if text_parts.len() == 1
            && let ContentPart::Text(text) = text_parts[0]
        {
            parse_structured_tool_result_text(&text.text)
        } else if text_parts.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::to_value(text_parts).unwrap_or_default()
        }
    });
    let mut message = if images.is_empty() {
        Message::tool_result(&data.tool_call_id, result, data.error)
    } else {
        Message::tool_result_with_images(&data.tool_call_id, result, images)
    };
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("tool_name".into(), serde_json::json!(data.tool_name));
    if let Some(value) = data.tool_call_fingerprint {
        metadata.insert("tool_call_fingerprint".into(), serde_json::json!(value));
    }
    if let Some(value) = data.tool_result_fingerprint {
        metadata.insert("tool_result_fingerprint".into(), serde_json::json!(value));
    }
    message.metadata = Some(metadata);
    message
}

fn parse_structured_tool_result_text(text: &str) -> serde_json::Value {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return serde_json::Value::String(text.to_string());
    }
    match serde_json::from_str(trimmed) {
        Ok(value @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) => value,
        _ => serde_json::Value::String(text.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::events::{
        EventContext, InputMessageData, OutputMessageDeltaData, SessionStartedData,
    };
    use everruns_core::typed_id::{HarnessId, TurnId};

    struct LifecycleHeavyReader;

    #[async_trait]
    impl EventReader for LifecycleHeavyReader {
        async fn read_page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError> {
            let after = request
                .cursor
                .as_ref()
                .map_or(0, EventCursor::after_sequence);
            let high_watermark = (MAX_EVENT_HISTORY_REPLAY + 1) as i32;
            let end = after
                .saturating_add(request.limit.get() as i32)
                .min(high_watermark);
            let events = ((after + 1)..=end)
                .map(|sequence| {
                    EventRequest::new(
                        request.session_id,
                        EventContext::empty(),
                        OutputMessageDeltaData {
                            turn_id: TurnId::new(),
                            message_id: MessageId::new(),
                            delta: String::new(),
                            accumulated: String::new(),
                            phase: None,
                        },
                    )
                    .into_event(EventId::new(), sequence)
                })
                .collect();
            let next_cursor = (end < high_watermark)
                .then(|| EventCursor::continuation(request.session_id, end, high_watermark))
                .transpose()?;
            EventPage::new(events, next_cursor, high_watermark)
        }
    }

    #[tokio::test]
    async fn full_projection_caps_examined_lifecycle_envelopes() {
        let history = EventHistory::new(Arc::new(LifecycleHeavyReader));
        let error = match history.project(SessionId::new()).await {
            Ok(_) => panic!("lifecycle-heavy replay must be bounded"),
            Err(error) => error,
        };
        assert!(matches!(error, EventLogError::InvalidRead { .. }));
        assert!(error.to_string().contains("examined more than"));
    }

    #[tokio::test]
    async fn exact_message_boundary_ignores_trailing_lifecycle_events() {
        let session_id = SessionId::new();
        let log = Arc::new(InMemoryEventLog::new());
        log.append(EventRequest::new(
            session_id,
            EventContext::empty(),
            InputMessageData::new(Message::user("hello")),
        ))
        .await
        .expect("append input message");
        log.append(EventRequest::new(
            session_id,
            EventContext::empty(),
            OutputMessageCompletedData::new(Message::assistant("hi")),
        ))
        .await
        .expect("append output message");
        log.append(EventRequest::new(
            session_id,
            EventContext::empty(),
            SessionStartedData {
                harness_id: HarnessId::new(),
                agent_id: None,
                model_id: None,
            },
        ))
        .await
        .expect("append lifecycle event");

        let page = EventHistory::new(log)
            .read_page(EventHistoryReadRequest::new(
                session_id,
                EventHistoryReadLimit::new(2).expect("valid history limit"),
            ))
            .await
            .expect("read history");

        assert_eq!(page.messages.len(), 2);
        assert!(page.next_cursor.is_none());
    }

    #[test]
    fn tool_completion_projection_preserves_structured_result_and_fingerprints() {
        let event = EventRequest::new(
            SessionId::new(),
            EventContext::empty(),
            everruns_core::events::ToolCompletedData::success(
                "call_read".into(),
                "read_file".into(),
                vec![ContentPart::text(
                    serde_json::json!({
                        "path": "/workspace/src/lib.rs",
                        "content": "1|fn main() {}"
                    })
                    .to_string(),
                )],
                Some(1),
            )
            .with_fingerprints("sha256:call".into(), "sha256:result".into()),
        )
        .into_event(EventId::new(), 1);
        let message = message_from_event(&event).expect("tool result message");
        let result = message
            .tool_result_content()
            .and_then(|content| content.result.as_ref())
            .expect("projected result");
        assert_eq!(result["path"], "/workspace/src/lib.rs");
        let metadata = message.metadata.expect("tool metadata");
        assert_eq!(metadata["tool_name"], "read_file");
        assert_eq!(metadata["tool_call_fingerprint"], "sha256:call");
        assert_eq!(metadata["tool_result_fingerprint"], "sha256:result");
    }

    #[test]
    fn tool_completion_projection_keeps_scalar_json_as_text() {
        let event = EventRequest::new(
            SessionId::new(),
            EventContext::empty(),
            everruns_core::events::ToolCompletedData::success(
                "call_scalar".into(),
                "custom_tool".into(),
                vec![ContentPart::text("123")],
                Some(1),
            ),
        )
        .into_event(EventId::new(), 1);
        let message = message_from_event(&event).expect("tool result message");
        let result = message
            .tool_result_content()
            .and_then(|content| content.result.as_ref())
            .expect("projected result");
        assert_eq!(result, &serde_json::Value::String("123".into()));
    }
}

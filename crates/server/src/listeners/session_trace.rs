// Keeps the session trace index current from the listener feed.
//
// Decision: buffers each session's trace events as the feed delivers them
// and projects them once the turn ends, so the turn's events are never read
// back from storage (`Database::project_session_trace_events`). A buffer is
// only as complete as this replica's view of the session: the projection
// reads from storage whatever lies between the index's watermark and the
// first buffered event, so a partition that moved, a restart or a failed
// pass costs one extra read, never a missing step. Readers still catch the
// index up before every read, so a turn not yet projected here is shown
// whole.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use everruns_contracts::typed_id::SessionId;
use everruns_core::{Event, EventListener};

use crate::storage::StorageBackend;
use crate::storage::repositories::Database;
use crate::storage::repositories::session_trace_from_events::trace_event;
use crate::storage::repositories::session_trace_projection::{TRACE_EVENT_TYPES, TraceEvent};

/// A buffer untouched this long belongs to a turn whose end will not come
/// here, and is dropped; the next read or the backfill projects it.
const STALE_BUFFER: Duration = Duration::from_secs(60 * 60);
/// How often stale buffers are looked for.
const SWEEP_EVERY: Duration = Duration::from_secs(60);

struct Buffer {
    events: Vec<TraceEvent>,
    touched: Instant,
}

pub struct SessionTraceListener {
    db: Arc<StorageBackend>,
    buffers: parking_lot::Mutex<HashMap<SessionId, Buffer>>,
    last_sweep: parking_lot::Mutex<Instant>,
}

impl SessionTraceListener {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            db,
            buffers: parking_lot::Mutex::new(HashMap::new()),
            last_sweep: parking_lot::Mutex::new(Instant::now()),
        }
    }

    /// Buffer `event`; the session's buffered events when it ends a turn.
    fn buffer(&self, event: &Event) -> Option<Vec<TraceEvent>> {
        self.sweep();
        let trace = trace_event(event)?;
        let mut buffers = self.buffers.lock();
        let buffer = buffers.entry(event.session_id).or_insert_with(|| Buffer {
            events: Vec::new(),
            touched: Instant::now(),
        });
        buffer.touched = Instant::now();
        // A redelivered event is already buffered.
        if buffer
            .events
            .last()
            .is_some_and(|last| last.sequence >= trace.sequence)
        {
            return None;
        }
        buffer.events.push(trace);
        if !Database::ends_trace_turn(&event.event_type) {
            return None;
        }
        buffers.remove(&event.session_id).map(|b| b.events)
    }

    fn sweep(&self) {
        {
            let mut last = self.last_sweep.lock();
            if last.elapsed() < SWEEP_EVERY {
                return;
            }
            *last = Instant::now();
        }
        self.buffers
            .lock()
            .retain(|_, buffer| buffer.touched.elapsed() < STALE_BUFFER);
    }
}

#[async_trait]
impl EventListener for SessionTraceListener {
    async fn on_event(&self, event: &Event) {
        let Some(events) = self.buffer(event) else {
            return;
        };
        if let Err(error) = self
            .db
            .project_session_trace_events(event.session_id.uuid(), events)
            .await
        {
            tracing::debug!(session_id = %event.session_id, %error, "session trace projection deferred");
        }
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(TRACE_EVENT_TYPES.to_vec())
    }

    fn name(&self) -> &'static str {
        "SessionTraceListener"
    }
}

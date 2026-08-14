//! External-consumer acceptance fixture for the canonical event-log SPI.
//!
//! This crate is deliberately outside the Everruns workspace. It implements
//! [`EventReader`] and [`EventLog`] against its own storage using nothing but
//! published public paths from `everruns-host` and `everruns-core`, so it fails
//! to build if the SPI stops being externally implementable.
//!
//! The store is an append-only physical log projected into a *filtered* logical
//! event sequence: entries appended through [`ExternalEventLog::append_hidden`]
//! consume a sequence but are never returned by reads. That is the shape a real
//! external host has when it keeps records the canonical replay must not see,
//! and it exercises the contract that sequences may contain gaps.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use everruns_core::events::{Event, EventRequest};
use everruns_provider::typed_id::{EventId, SessionId};
use everruns_host::{
    EventCursor, EventDurability, EventLog, EventLogError, EventPage, EventReadRequest, EventReader,
};

struct Entry {
    event: Event,
    visible: bool,
}

/// Volatile canonical event log owned entirely by this external crate.
#[derive(Default)]
pub struct ExternalEventLog {
    sessions: Mutex<HashMap<SessionId, Vec<Entry>>>,
}

impl ExternalEventLog {
    /// Create an empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a durable record that consumes a sequence but stays out of the
    /// projected logical event sequence, producing a gap for readers.
    pub fn append_hidden(&self, request: EventRequest) -> Event {
        self.insert(request, false)
    }

    /// Number of physical records stored for a session, visible or not.
    pub fn physical_len(&self, session_id: SessionId) -> usize {
        self.sessions
            .lock()
            .expect("event log mutex")
            .get(&session_id)
            .map_or(0, Vec::len)
    }

    fn insert(&self, request: EventRequest, visible: bool) -> Event {
        let mut sessions = self.sessions.lock().expect("event log mutex");
        let entries = sessions.entry(request.session_id).or_default();
        // The log owns identity: it assigns the event id and the next
        // per-session sequence, then finalizes the request into an `Event`.
        let sequence = entries
            .last()
            .and_then(|entry| entry.event.sequence)
            .unwrap_or(0)
            + 1;
        let event = request.into_event(EventId::new(), sequence);
        entries.push(Entry {
            event: event.clone(),
            visible,
        });
        event
    }
}

#[async_trait]
impl EventReader for ExternalEventLog {
    async fn read_page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError> {
        let session_id = request.session_id();
        let sessions = self.sessions.lock().expect("event log mutex");
        let entries = sessions.get(&session_id).map(Vec::as_slice).unwrap_or(&[]);
        let current_high = entries
            .last()
            .and_then(|entry| entry.event.sequence)
            .unwrap_or(0);

        // Inspecting the request cursor is what distinguishes an initial read,
        // a continuation pinned to an earlier snapshot, and a poll.
        let (after, snapshot) = match request.cursor() {
            None => (0, current_high),
            Some(cursor) => {
                if cursor.session_id() != session_id {
                    return Err(EventLogError::CrossSessionCursor {
                        detail: "cursor belongs to another session".into(),
                    });
                }
                match cursor.snapshot_high_watermark() {
                    // Pinned continuation: stay on the captured snapshot.
                    Some(snapshot) => {
                        if snapshot > current_high {
                            return Err(EventLogError::ExpiredCursor {
                                detail: "cursor snapshot is not available in this log".into(),
                            });
                        }
                        if cursor.after_sequence() > snapshot {
                            return Err(EventLogError::IncompatibleCursor {
                                detail: "cursor position exceeds its snapshot".into(),
                            });
                        }
                        (cursor.after_sequence(), snapshot)
                    }
                    // Poll: capture a fresh snapshot after the cursor position.
                    None => (cursor.after_sequence(), current_high),
                }
            }
        };

        if snapshot == 0 {
            return EventPage::new(Vec::new(), None, 0);
        }

        let limit = request.limit().get();
        let mut selected = entries
            .iter()
            .filter(|entry| entry.visible)
            .filter(|entry| {
                entry
                    .event
                    .sequence
                    .is_some_and(|sequence| sequence > after && sequence <= snapshot)
            })
            .map(|entry| entry.event.clone())
            .take(limit + 1)
            .collect::<Vec<_>>();
        let has_more = selected.len() > limit;
        if has_more {
            selected.pop();
        }
        let next_cursor = has_more
            .then(|| {
                let last = selected
                    .last()
                    .and_then(|event| event.sequence)
                    .expect("a page with more events returned at least one event");
                EventCursor::continuation(session_id, last, snapshot)
            })
            .transpose()?;
        EventPage::new(selected, next_cursor, snapshot)
    }
}

#[async_trait]
impl EventLog for ExternalEventLog {
    async fn append(&self, request: EventRequest) -> Result<Event, EventLogError> {
        if request.is_ephemeral() {
            return Err(EventLogError::InvalidAppend {
                detail: format!(
                    "ephemeral event {} must be routed sink-only",
                    request.event_type
                ),
            });
        }
        Ok(self.insert(request, true))
    }

    fn durability(&self) -> EventDurability {
        EventDurability::Volatile
    }
}

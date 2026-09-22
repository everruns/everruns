//! Cursor constructors for [`EventReader`](crate::EventReader) implementations
//! that live outside this crate.

use everruns_provider::typed_id::SessionId;

use crate::events::{EventCursor, EventLogError};

impl EventCursor {
    /// Continue a pinned snapshot after `after_sequence`.
    ///
    /// The form an out-of-crate [`EventReader`] uses to hand back `next_cursor`
    /// while paging a stable snapshot (see [`EventPage`]).
    pub fn after_snapshot(
        session_id: SessionId,
        after_sequence: i32,
        snapshot_high_watermark: i32,
    ) -> Result<Self, EventLogError> {
        if after_sequence < 0 || snapshot_high_watermark < after_sequence {
            return Err(EventLogError::InvalidRead {
                detail: "cursor position must be within [0, snapshot high-watermark]".into(),
            });
        }
        Ok(Self {
            session_id,
            after_sequence,
            snapshot_high_watermark: Some(snapshot_high_watermark),
        })
    }
}

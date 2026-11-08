//! Reads from a session log's end: lookups that concern the latest turn
//! cost the turn, not the session's history.

use super::*;

/// Read `session_id`'s log from its end, a window at a time, until `enough`
/// holds for the events read so far; return them in sequence order.
///
/// The log has no reverse read, so this pins one snapshot and reads forward
/// within each window, doubling it each time. A lookup that concerns the
/// latest turn costs the turn, not the session's history. At most
/// [`MAX_EVENT_HISTORY_REPLAY`] envelopes are examined.
pub(crate) async fn read_tail_until(
    reader: &dyn EventReader,
    session_id: SessionId,
    enough: impl Fn(&[Event]) -> bool,
) -> Result<Vec<Event>, EventLogError> {
    let high = match reader.last_sequence(session_id).await? {
        Some(high) => high,
        None => reader
            .read_page(EventReadRequest::new(session_id, EventReadLimit::new(1)?))
            .await?
            .snapshot_high_watermark(),
    };
    let mut tail: Vec<Event> = Vec::new();
    let (mut hi, mut window) = (high, 256i32);
    while hi > 0 {
        let lo = hi.saturating_sub(window).max(0);
        let mut chunk = Vec::new();
        let mut request = EventReadRequest::from_cursor(
            EventCursor::continuation(session_id, lo, high)?,
            EventReadLimit::default(),
        );
        'window: loop {
            let page = reader.read_page(request).await?;
            for event in page.events {
                if event.sequence.is_some_and(|sequence| sequence > hi) {
                    break 'window;
                }
                chunk.push(event);
            }
            match page.next_cursor {
                Some(cursor) => {
                    request = EventReadRequest::from_cursor(cursor, EventReadLimit::default())
                }
                None => break,
            }
        }
        chunk.append(&mut tail);
        tail = chunk;
        if tail.len() > MAX_EVENT_HISTORY_REPLAY {
            return Err(EventLogError::InvalidRead {
                detail: format!("tail read examined more than {MAX_EVENT_HISTORY_REPLAY} events"),
            });
        }
        if enough(&tail) {
            break;
        }
        hi = lo;
        window = window.saturating_mul(2);
    }
    Ok(tail)
}

impl EventHistory {
    /// The messages a turn resolves with: those after `checkpoint` (the
    /// newest compaction checkpoint's source), widened to start at the
    /// latest turn's input when the checkpoint is newer than it.
    ///
    /// Decision: a turn reads only its input and what follows it (causal
    /// context), and the completion review sees the history the model sees,
    /// which a checkpoint already replaced. Loading the whole session here
    /// made each turn cost the session's length. A log with no turn started
    /// yet loads whole.
    pub(crate) async fn load_since_checkpoint(
        &self,
        session_id: SessionId,
        checkpoint: i64,
    ) -> CoreResult<Vec<RuntimeMessage>> {
        let latest_input = |events: &[Event]| {
            let input = events.iter().rev().find_map(|event| match &event.data {
                EventData::TurnStarted(data) => Some(data.input_message_id),
                _ => None,
            })?;
            events.iter().find_map(|event| match &event.data {
                EventData::InputMessage(data) if data.message.id == input => event.sequence,
                _ => None,
            })
        };
        let tail = read_tail_until(self.reader.as_ref(), session_id, |events| {
            latest_input(events).is_some()
        })
        .await
        .map_err(core_event_error)?;
        let Some(input_sequence) = latest_input(&tail) else {
            return self.load(session_id).await;
        };
        let after = checkpoint.min(i64::from(input_sequence) - 1);
        Ok(self
            .project_after(session_id, Some(after))
            .await
            .map_err(core_event_error)?
            .into_iter()
            .map(|item| item.message)
            .collect())
    }
}

//! A session's own rules: its history in memory and in segments, its
//! turns and its checkpoints (split from `state.rs`).

use super::*;

impl SessionState {
    pub fn last_sequence(&self) -> Option<i32> {
        self.events
            .last()
            .and_then(|event| event.sequence)
            .or((self.offloaded > 0).then_some(self.offloaded))
    }

    /// The last sequence this replica's segments cover.
    pub fn flushed(&self) -> i32 {
        self.segments.last().map(|s| s.to).unwrap_or(0)
    }

    /// Note that `seg` is stored, keeping `segments` contiguous from
    /// sequence 1. A segment that starts where a recorded one does and
    /// reaches the end replaces that one and those after it (compaction);
    /// one that does not continue the list is ignored.
    pub fn record(&mut self, seg: crate::store::SegRef) {
        let flushed = self.flushed();
        let first = self.segments.iter().position(|s| s.from == seg.from);
        let last = self.segments.iter().position(|s| s.to == seg.to);
        match (first, last) {
            // A compaction: it replaces exactly the run it covers.
            (Some(first), Some(last)) if first < last => {
                self.segments.splice(first..=last, [seg]);
            }
            _ if seg.from == flushed + 1 => self.segments.push(seg),
            _ => {}
        }
    }

    /// The run of segments a compaction rewrites into one: the trailing run
    /// whose older end is at most twice the size of what follows it, or
    /// none while that run is the newest segment alone.
    ///
    /// Decision: tiered, never from sequence 1 (that made every compaction
    /// cost the whole history), and never a forced pair. Forcing at least
    /// two segments merged the newest one, once large, with each later
    /// flush: an O(n) rewrite per flush (2400 times the events written in a
    /// simulation with flushes of 11 to 77 events). Waiting instead until
    /// the small segments after it add up to half its size keeps sizes
    /// geometric: an event is rewritten about log(n) times, and a session
    /// holds about COMPACT_AT segments. The run covers stored segments only;
    /// the unstored tail is a flush's.
    pub fn compaction_range(&self) -> Option<(i32, i32)> {
        let segments = &self.segments;
        let len = |s: &crate::store::SegRef| i64::from(s.to - s.from + 1);
        let last = segments.len().checked_sub(1)?;
        let mut at = last;
        let mut size = len(&segments[at]);
        // The first segment (the session's first flush, with
        // `session.started`) is never merged: reads near the session's start
        // then parse a few events, not half its history.
        let floor = usize::from(segments.first().is_some_and(|s| s.from == 1));
        while at > floor && len(&segments[at - 1]) <= 2 * size {
            at -= 1;
            size += len(&segments[at]);
        }
        (at < last && at >= floor).then(|| (segments[at].from, segments[last].to))
    }

    /// Where the current (or last) turn's events begin: its input or its
    /// `turn.started`, whichever is earlier. A turn resumes from the events
    /// after it; before it, only messages matter.
    pub fn turn_from(&self) -> i32 {
        let mut from = self.last_sequence().unwrap_or(0) + 1;
        if self.turn_start > 0 {
            from = from.min(self.turn_start);
        }
        // Admitted input no turn has consumed yet is read whole too.
        if let Some(first) = self
            .authority
            .pending_inputs
            .iter()
            .find(|seq| **seq > self.authority.consumed)
        {
            from = from.min(*first as i32);
        }
        from
    }

    /// Where an executor's copy of the log begins: the current turn's
    /// events, and the messages after the newest compaction checkpoint
    /// (the engine reads no further back while it can restore one). Older
    /// messages are read on demand.
    pub fn context_from(&self) -> i32 {
        let floor = self.checkpoints.iter().map(|c| c.source_sequence).max();
        match floor {
            Some(floor) => self.turn_from().min(floor.saturating_add(1) as i32),
            None => 1,
        }
    }

    /// Resident events with a sequence above `after`.
    pub fn resident_after(&self, after: i32) -> &[Event] {
        let skip = (after - self.offloaded).max(0) as usize;
        &self.events[skip.min(self.events.len())..]
    }

    /// An epoch holds the session (a turn runs, or its holder is presumed
    /// to until superseded).
    pub fn turn_running(&self) -> bool {
        self.authority.holder.is_some()
    }

    /// Admitted input no turn has consumed yet.
    pub fn queued(&self) -> usize {
        let consumed = self.authority.consumed;
        self.authority
            .pending_inputs
            .iter()
            .filter(|seq| **seq > consumed)
            .count()
    }

    /// Turns with a Framework outcome.
    pub fn turns_completed(&self) -> u64 {
        self.authority.outcomes
    }

    /// How long the last finished turn took, from its `turn.started` to the
    /// event that ended it, when both are still in memory.
    pub fn last_turn_ms(&self) -> Option<u64> {
        let mut events = self.events.iter().rev();
        let end = events.find(|e| {
            matches!(
                e.data,
                EventData::TurnCompleted(_)
                    | EventData::TurnFailed(_)
                    | EventData::TurnCancelled(_)
            )
        })?;
        let start = events.find(|e| matches!(e.data, EventData::TurnStarted(_)))?;
        u64::try_from((end.ts - start.ts).num_milliseconds()).ok()
    }

    /// No turn runs and none waits: safe to unload.
    pub fn idle(&self) -> bool {
        self.authority.holder.is_none()
            && !self.authority.runnable
            && self.authority.open.is_none()
            && self.placement == Placement::Live
    }
}

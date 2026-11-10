//! The journal tail one Engine epoch keeps, so the turn's many reads of a
//! session's recent history (tail lookups, the turn's messages) cost one
//! small read each, or none, instead of a page of history each (data plane
//! v2 step 4 bench: 23 reads returning 960 rows per turn before).
//!
//! Decisions:
//! - It holds one contiguous run of the log, `from..=head`. Stored events
//!   never change, so a cached event is never stale; what the cache cannot
//!   know is what was stored after `head`.
//! - A read pinned to a snapshot at or below `head` is served from the
//!   cache with no round trip. Any other read first asks the control plane
//!   for what follows `head`, so an event another writer stored (an input
//!   admitted mid-turn, a cancel note) is seen on the next read, as before.
//! - It grows from every read and from this epoch's own appends. Two runs
//!   that touch merge; a run that does not touch the cached one replaces it
//!   only when it is newer. An append that does not follow `head` (another
//!   writer came first) is skipped: the next read brings both.
//! - Bounded ([`CACHED_EVENTS`]): past it the oldest events leave, and a
//!   read from before the run goes to the control plane as before. One
//!   turn's tail lookup reads the last 256 events, so the bound covers it
//!   with the turn's own events on top, and an epoch holds only its session.

use std::sync::Mutex;

use crate::core::events::Event;

/// Events one epoch keeps.
pub const CACHED_EVENTS: usize = 512;

/// A contiguous run of a session's log.
#[derive(Default)]
pub struct JournalTail {
    run: Mutex<Option<Run>>,
}

#[derive(Debug)]
struct Run {
    /// Sequence of the first cached event.
    from: i32,
    /// Events `from..=head`, one per sequence.
    events: Vec<Event>,
}

/// A page served from the cache.
#[derive(Debug)]
pub struct CachedPage {
    pub events: Vec<Event>,
    /// The last sequence the page examined.
    pub last: i32,
    /// Events up to the watermark follow `last`.
    pub more: bool,
}

impl JournalTail {
    /// The newest cached sequence, `None` with nothing cached.
    pub fn head(&self) -> Option<i32> {
        let run = self.run.lock().unwrap_or_else(|p| p.into_inner());
        run.as_ref().map(Run::head)
    }

    /// The newest cached sequence when the cache holds the event right after
    /// `after` or `after` is its head; `None` when a read from `after` must
    /// go to the control plane.
    pub fn head_after(&self, after: i32) -> Option<i32> {
        let run = self.run.lock().unwrap_or_else(|p| p.into_inner());
        run.as_ref()
            .filter(|run| after >= run.from - 1 && after <= run.head())
            .map(Run::head)
    }

    /// Record events read after `after`, in sequence order and with none
    /// left out (an unfiltered journal read).
    pub fn record(&self, after: i32, events: &[Event]) {
        let Some(read) = Run::of(after, events) else {
            return;
        };
        let mut guard = self.run.lock().unwrap_or_else(|p| p.into_inner());
        let merged = match guard.take() {
            None => read,
            Some(run) => run.merge(read),
        };
        *guard = Some(merged.trimmed());
    }

    /// This epoch's own appends: kept when they follow the cached head.
    pub fn appended(&self, events: &[Event]) {
        let mut guard = self.run.lock().unwrap_or_else(|p| p.into_inner());
        let Some(run) = guard.as_mut() else {
            return;
        };
        let Some(first) = events.first().and_then(|event| event.sequence) else {
            return;
        };
        if first != run.head() + 1 {
            return;
        }
        if let Some(appended) = Run::of(first - 1, events) {
            let run = guard.take().map(|run| run.merge(appended));
            *guard = run.map(Run::trimmed);
        }
    }

    /// Up to `limit` cached events after `after` and at most `watermark`
    /// that `keep` selects (the limit counts events kept); `None` unless the cache holds all of
    /// `after+1..=watermark`.
    pub fn page(
        &self,
        after: i32,
        watermark: i32,
        limit: usize,
        keep: impl Fn(&Event) -> bool,
    ) -> Option<CachedPage> {
        let run = self.run.lock().unwrap_or_else(|p| p.into_inner());
        let run = run
            .as_ref()
            .filter(|run| after >= run.from - 1 && watermark <= run.head())?;
        let start = usize::try_from(after + 1 - run.from).ok()?;
        let mut page = CachedPage {
            events: Vec::new(),
            last: after,
            more: false,
        };
        for event in run.events.iter().skip(start) {
            let sequence = event.sequence?;
            if sequence > watermark {
                break;
            }
            if page.events.len() == limit {
                page.more = true;
                break;
            }
            page.last = sequence;
            if keep(event) {
                page.events.push(event.clone());
            }
        }
        Some(page)
    }
}

impl Run {
    /// The run `events` make when they follow `after` one sequence each;
    /// `None` when they leave a gap or carry no sequence.
    fn of(after: i32, events: &[Event]) -> Option<Self> {
        if events.is_empty() {
            return None;
        }
        let contiguous = events
            .iter()
            .enumerate()
            .all(|(i, event)| event.sequence == i32::try_from(i).ok().map(|i| after + 1 + i));
        contiguous.then(|| Self {
            from: after + 1,
            events: events.to_vec(),
        })
    }

    fn head(&self) -> i32 {
        self.from - 1 + i32::try_from(self.events.len()).unwrap_or(i32::MAX)
    }

    /// One run covering both when they touch; the newer one otherwise.
    fn merge(self, other: Self) -> Self {
        let (low, high) = if self.from <= other.from {
            (self, other)
        } else {
            (other, self)
        };
        if high.from > low.head() + 1 {
            return if high.head() >= low.head() { high } else { low };
        }
        if high.head() <= low.head() {
            return low;
        }
        let mut low = low;
        let skip = usize::try_from(low.head() + 1 - high.from).unwrap_or(0);
        low.events.extend(high.events.into_iter().skip(skip));
        low
    }

    fn trimmed(mut self) -> Self {
        if self.events.len() > CACHED_EVENTS {
            let drop = self.events.len() - CACHED_EVENTS;
            self.events.drain(..drop);
            self.from += i32::try_from(drop).unwrap_or(i32::MAX);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::events::{EventContext, EventData};
    use everruns_contracts::typed_id::SessionId;

    fn event(session: SessionId, sequence: i32) -> Event {
        let mut event = Event::new(
            session,
            EventContext::empty(),
            EventData::TurnCompleted(Default::default()),
        );
        event.sequence = Some(sequence);
        event
    }

    fn run(session: SessionId, from: i32, to: i32) -> Vec<Event> {
        (from..=to)
            .map(|sequence| event(session, sequence))
            .collect()
    }

    fn sequences(page: &CachedPage) -> Vec<i32> {
        page.events
            .iter()
            .filter_map(|event| event.sequence)
            .collect()
    }

    #[test]
    fn serves_a_snapshot_it_holds_and_nothing_before_it() {
        let session = SessionId::new();
        let tail = JournalTail::default();
        assert_eq!(tail.head_after(0), None);
        tail.record(10, &run(session, 11, 20));
        assert_eq!(tail.head_after(10), Some(20));
        assert_eq!(tail.head_after(20), Some(20));
        assert_eq!(tail.head_after(9), None, "event 10 is not cached");
        assert_eq!(tail.head_after(21), None);

        let page = tail.page(12, 15, 100, |_| true).unwrap();
        assert_eq!(sequences(&page), vec![13, 14, 15]);
        assert_eq!(page.last, 15);
        assert!(!page.more);
        assert!(
            tail.page(12, 21, 100, |_| true).is_none(),
            "a watermark past the head needs the control plane"
        );
        assert!(tail.page(9, 15, 100, |_| true).is_none());
    }

    #[test]
    fn pages_by_events_examined_and_filters() {
        let session = SessionId::new();
        let tail = JournalTail::default();
        tail.record(0, &run(session, 1, 10));
        let page = tail.page(0, 10, 4, |_| true).unwrap();
        assert_eq!(sequences(&page), vec![1, 2, 3, 4]);
        assert!(page.more);
        let page = tail
            .page(0, 10, 4, |event| event.sequence.unwrap() % 2 == 0)
            .unwrap();
        assert_eq!(
            sequences(&page),
            vec![2, 4, 6, 8],
            "the limit counts events kept"
        );
        assert_eq!(page.last, 8);
        assert!(page.more);
        let page = tail.page(8, 10, 4, |_| true).unwrap();
        assert_eq!(sequences(&page), vec![9, 10]);
        assert!(!page.more);
    }

    #[test]
    fn appends_extend_only_from_the_head() {
        let session = SessionId::new();
        let tail = JournalTail::default();
        tail.appended(&run(session, 1, 2));
        assert_eq!(tail.head_after(0), None, "nothing to extend");
        tail.record(0, &run(session, 1, 5));
        tail.appended(&run(session, 6, 7));
        assert_eq!(tail.head_after(0), Some(7));
        // Another writer stored 8: this epoch's 9 waits for the next read.
        tail.appended(&run(session, 9, 9));
        assert_eq!(tail.head_after(0), Some(7));
        tail.record(7, &run(session, 8, 9));
        assert_eq!(tail.head_after(0), Some(9));
    }

    #[test]
    fn reads_merge_when_they_touch_and_newer_wins_otherwise() {
        let session = SessionId::new();
        let tail = JournalTail::default();
        tail.record(20, &run(session, 21, 30));
        // Overlapping and older: extends the front.
        tail.record(14, &run(session, 15, 25));
        assert_eq!(tail.head_after(14), Some(30));
        // Disjoint and older: ignored.
        tail.record(0, &run(session, 1, 5));
        assert_eq!(tail.head_after(14), Some(30));
        // Disjoint and newer: replaces.
        tail.record(40, &run(session, 41, 45));
        assert_eq!(tail.head_after(14), None);
        assert_eq!(tail.head_after(40), Some(45));
        // A read with a gap is not cached.
        let mut gapped = run(session, 46, 47);
        gapped.push(event(session, 49));
        tail.record(45, &gapped);
        assert_eq!(tail.head_after(40), Some(45));
    }

    #[test]
    fn keeps_the_newest_events_past_the_bound() {
        let session = SessionId::new();
        let tail = JournalTail::default();
        let total = i32::try_from(CACHED_EVENTS).unwrap() + 100;
        tail.record(0, &run(session, 1, total));
        assert_eq!(tail.head_after(100), Some(total));
        assert_eq!(tail.head_after(99), None);
    }
}

//! Active conversation lineage over the immutable journal (EVE-1305).
//!
//! Decision: `framework_events` is never rewritten or filtered at write time.
//! Control state names the active branch as canonical sequence ranges plus a
//! `base`: every event appended after `base` extends the active branch. Context
//! and history reads apply that predicate in SQL; raw export does not. Other
//! branches are retained as `branch` objects at the extent they had when they
//! were left, so redo and fork reselect them exactly.
use super::*;
use crate::hosting::spi::session_commands::history::{
    self, HistoryBranch, HistoryLineage, HistoryView, JournalEntry, SequenceRange,
};

/// Active branch, absent until the first selection or a lineage import.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct Lineage {
    pub(super) revision: u64,
    pub(super) branch: String,
    /// Canonical selected ranges, all at or before `base`.
    pub(super) ranges: Vec<SequenceRange>,
    /// Journal head when the branch was selected; later events extend it.
    pub(super) base: i32,
}

/// Active-branch predicate over `seq`, bound to `?3` (ranges JSON or NULL)
/// and `?4` (base). A NULL lineage selects every event.
pub(super) const ACTIVE: &str = "(?3 IS NULL OR seq>?4 OR EXISTS(SELECT 1 FROM json_each(?3) AS r WHERE seq BETWEEN json_extract(r.value,'$[0]') AND json_extract(r.value,'$[1]')))";

/// Parameters for [`ACTIVE`].
pub(super) fn filter(state: &Control) -> Result<(Option<String>, i32)> {
    Ok(match &state.lineage {
        None => (None, 0),
        Some(lineage) => (
            Some(encode(
                &lineage
                    .ranges
                    .iter()
                    .map(|range| [range.first, range.last])
                    .collect::<Vec<_>>(),
            )?),
            lineage.base,
        ),
    })
}

pub(super) fn first_sequence(tx: &Transaction<'_>, session: SessionId) -> Result<Option<i32>> {
    tx.query_row(
        "SELECT min(seq) FROM framework_events WHERE session=?",
        [session.to_string()],
        |row| row.get(0),
    )
    .map_err(error)
}

/// The active branch's identity.
pub(super) fn active_branch(state: &Control) -> String {
    state
        .lineage
        .as_ref()
        .map_or_else(|| history::DEFAULT_BRANCH.into(), |l| l.branch.clone())
}

/// The active branch's current canonical extent.
pub(super) fn active_ranges(
    tx: &Transaction<'_>,
    session: SessionId,
    state: &Control,
) -> Result<Vec<SequenceRange>> {
    let Some(lineage) = &state.lineage else {
        return Ok(
            HistoryLineage::linear(first_sequence(tx, session)?, state.head)
                .active_ranges()
                .to_vec(),
        );
    };
    let mut ranges = lineage.ranges.clone();
    if state.head > lineage.base {
        // Appends after selection are contiguous at base+1..=head.
        match ranges.last_mut() {
            Some(last) if last.last == lineage.base => last.last = state.head,
            _ => ranges.push(SequenceRange::new(lineage.base + 1, state.head)),
        }
    }
    Ok(ranges)
}

/// Every journal position with its turn, for canonical selection. The first
/// terminal of a turn carries the `terminal:` identity.
pub(super) fn journal(tx: &Transaction<'_>, session: SessionId) -> Result<Vec<JournalEntry>> {
    let mut query = tx
        .prepare("SELECT seq,turn_id,coalesce(substr(identity,1,9)='terminal:',0) FROM framework_events WHERE session=? ORDER BY seq")
        .map_err(error)?;
    query
        .query_map([session.to_string()], |row| {
            Ok((
                row.get::<_, i32>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })
        .map_err(error)?
        .map(|row| {
            let (sequence, turn, terminal) = row.map_err(error)?;
            Ok(JournalEntry {
                sequence,
                turn: turn.map(|turn| turn.parse()).transpose().map_err(error)?,
                terminal,
            })
        })
        .collect()
}

/// Journal positions of events about to be imported.
pub(super) fn image_journal(events: &[Event]) -> Vec<JournalEntry> {
    events
        .iter()
        .map(|event| JournalEntry {
            sequence: event.sequence.unwrap_or_default(),
            turn: event.context.turn_id,
            terminal: is_terminal(&event.data),
        })
        .collect()
}

fn retained_count(tx: &Transaction<'_>, session: SessionId) -> Result<usize> {
    tx.query_row(
        "SELECT count(*) FROM framework_objects WHERE session=? AND kind='branch'",
        [session.to_string()],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count as usize)
    .map_err(error)
}

/// Owed work a branch change must not silently discard.
fn obligations(state: &Control) -> bool {
    state.runnable
        || state.wake_at.is_some()
        || !state.finalizing.is_empty()
        || state.inbox > state.consumed
        || state.cancel.is_some()
        || state.resume_generation > state.resume_claimed
}

/// Reducer facts for selecting `branch`, optionally with new `ranges`.
pub(super) fn view(
    tx: &Transaction<'_>,
    session: SessionId,
    state: &Control,
    branch: &str,
    ranges: Option<&[SequenceRange]>,
) -> Result<HistoryView> {
    let active = active_branch(state);
    let retained = if branch == active {
        Some(active_ranges(tx, session, state)?)
    } else {
        object(tx, session, "branch", branch)?
    };
    let requested = match ranges {
        Some(ranges) => Some(history::canonical_selection(
            &journal(tx, session)?,
            ranges,
            state.head,
            true,
        )),
        None => None,
    };
    Ok(HistoryView {
        revision: state.lineage.as_ref().map_or(0, |l| l.revision),
        active,
        retained,
        requested,
        branches: retained_count(tx, session)? + 1,
        obligations: obligations(state),
    })
}

/// Retain the active branch at its extent and activate `branch`.
pub(super) fn select(
    tx: &Transaction<'_>,
    session: SessionId,
    state: &mut Control,
    branch: String,
    ranges: Vec<SequenceRange>,
    revision: u64,
) -> Result<()> {
    let previous = active_branch(state);
    put(
        tx,
        session,
        "branch",
        &previous,
        &active_ranges(tx, session, state)?,
    )?;
    tx.execute(
        "DELETE FROM framework_objects WHERE session=? AND kind='branch' AND key=?",
        params![session.to_string(), branch],
    )
    .map_err(error)?;
    state.lineage = Some(Lineage {
        revision,
        branch,
        ranges,
        base: state.head,
    });
    Ok(())
}

/// The branch catalog, active branch first.
pub(super) fn read(
    tx: &Transaction<'_>,
    session: SessionId,
    state: &Control,
) -> Result<HistoryLineage> {
    let active = active_branch(state);
    let mut branches = vec![HistoryBranch {
        id: active.clone(),
        ranges: active_ranges(tx, session, state)?,
    }];
    let mut query = tx
        .prepare("SELECT key,value FROM framework_objects WHERE session=? AND kind='branch' ORDER BY key LIMIT ?")
        .map_err(error)?;
    let retained = query
        .query_map(
            params![session.to_string(), history::MAX_BRANCHES as i64],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(error)?;
    for row in retained {
        let (id, ranges) = row.map_err(error)?;
        branches.push(HistoryBranch {
            id,
            ranges: decode(ranges)?,
        });
    }
    Ok(HistoryLineage {
        revision: state.lineage.as_ref().map_or(0, |l| l.revision),
        active,
        branches,
    })
}

/// The lineage prefix a checkpoint at `source` summarized: the stored one,
/// or for a checkpoint stored before lineage existed, the whole linear
/// journal through its source.
pub(super) fn summarized(
    stored: Option<&[SequenceRange]>,
    source: i64,
    first: Option<i32>,
) -> Option<Vec<SequenceRange>> {
    if let Some(stored) = stored {
        return Some(stored.to_vec());
    }
    let source = i32::try_from(source).ok()?;
    first
        .filter(|first| *first <= source)
        .map(|first| vec![SequenceRange::new(first, source)])
}

/// Whether a checkpoint at `source` summarizing `prefix` is valid context on
/// `branch`: the branch must contain the source and share the whole prefix.
pub(super) fn covers(branch: &[SequenceRange], source: i64, prefix: &[SequenceRange]) -> bool {
    i32::try_from(source)
        .ok()
        .and_then(|source| history::clip(branch, source))
        .is_some_and(|clipped| clipped == prefix)
}

//! Conversation branch selection over an immutable journal (EVE-1305).
//!
//! Decision: the journal is never rewritten. A branch is a set of journal
//! sequence ranges; the authority keeps which branch is active as control
//! state, separate from raw export/audit reads. Context and history reads see
//! only the active branch, and events appended later extend it. Abandoned
//! branches are retained for redo and fork. A compaction checkpoint records the
//! active lineage prefix it summarized and is eligible only on a branch with
//! the same prefix, so a replacement summary from an abandoned suffix never
//! appears under another branch while ancestor checkpoints stay usable.
//!
//! These helpers are pure so every authority applies the same rules; the
//! session-command reducer owns the quiescence and revision checks.
use everruns_contracts::typed_id::TurnId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Branch every session starts on before any selection.
pub const DEFAULT_BRANCH: &str = "main";
/// Retained branches per session, the active one included.
pub const MAX_BRANCHES: usize = 1024;
/// Ranges per branch; a branch that needs more should be compacted upstream.
pub const MAX_RANGES: usize = 1024;

/// An inclusive range of canonical journal sequences.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SequenceRange {
    /// First included sequence.
    pub first: i32,
    /// Last included sequence.
    pub last: i32,
}

impl SequenceRange {
    /// Range `first..=last`.
    pub fn new(first: i32, last: i32) -> Self {
        Self { first, last }
    }
    fn contains(&self, sequence: i32) -> bool {
        self.first <= sequence && sequence <= self.last
    }
}

/// One retained conversation lineage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryBranch {
    /// Caller-chosen opaque identity, stable across selections.
    pub id: String,
    /// Journal ranges that form this branch's context, in order.
    pub ranges: Vec<SequenceRange>,
}

/// The authority's branch catalog at one revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryLineage {
    /// Incremented by every committed branch change; pass it back as the
    /// expected revision of the next selection.
    pub revision: u64,
    /// Branch new input extends.
    pub active: String,
    /// Every retained branch, the active one first with its current extent.
    pub branches: Vec<HistoryBranch>,
}

impl HistoryLineage {
    /// Linear lineage of a session that never changed branch.
    pub fn linear(first: Option<i32>, head: i32) -> Self {
        Self {
            revision: 0,
            active: DEFAULT_BRANCH.into(),
            branches: vec![HistoryBranch {
                id: DEFAULT_BRANCH.into(),
                ranges: first
                    .filter(|first| *first <= head)
                    .map(|first| vec![SequenceRange::new(first, head)])
                    .unwrap_or_default(),
            }],
        }
    }
    /// A retained branch by identity.
    pub fn branch(&self, id: &str) -> Option<&HistoryBranch> {
        self.branches.iter().find(|branch| branch.id == id)
    }
    /// Ranges of the active branch.
    pub fn active_ranges(&self) -> &[SequenceRange] {
        self.branch(&self.active)
            .map(|branch| branch.ranges.as_slice())
            .unwrap_or_default()
    }
    /// Ranges for rewinding the active branch to just after `through`, or
    /// `None` when `through` is not on it. `0` rewinds to an empty branch.
    pub fn rewind_ranges(&self, through: i32) -> Option<Vec<SequenceRange>> {
        if through == 0 {
            return Some(Vec::new());
        }
        clip(self.active_ranges(), through)
    }
}

/// Reply of a committed or replayed branch selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorySelection {
    /// Lineage revision after the command.
    pub revision: u64,
    /// Branch now active.
    pub branch: String,
    /// `false` when the branch was already active with the same lineage.
    pub changed: bool,
}

/// Facts a branch-capable authority captures for `SelectBranch` in the same
/// transaction that commits it. Authorities without branch support leave
/// [`super::CommandView::history`] empty and the reducer refuses the command.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HistoryView {
    /// Current lineage revision.
    pub revision: u64,
    /// Active branch identity.
    pub active: String,
    /// Retained canonical ranges of the requested branch, when it exists
    /// (the active branch's current extent when it is the active one).
    #[serde(default)]
    pub retained: Option<Vec<SequenceRange>>,
    /// Canonical form of the requested ranges, or the journal rule they break.
    #[serde(default)]
    pub requested: Option<std::result::Result<Vec<SequenceRange>, String>>,
    /// Retained branches, the active one included.
    #[serde(default)]
    pub branches: usize,
    /// Owed work beyond the reducer's own view: runnable or finalizing turns,
    /// unconsumed input, pending cancellation or a retained wakeup.
    #[serde(default)]
    pub obligations: bool,
}

/// Validate an opaque branch identity.
pub fn valid_branch_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}

/// Check range shape: positive, ordered and disjoint, within `head`.
pub fn validate_ranges(ranges: &[SequenceRange], head: i32) -> Result<(), String> {
    if ranges.len() > MAX_RANGES {
        return Err("history branch has too many ranges".into());
    }
    let mut after = 0;
    for range in ranges {
        if range.first <= after || range.last < range.first {
            return Err("history ranges must be positive, ordered and disjoint".into());
        }
        if range.last > head {
            return Err("history range exceeds the canonical journal".into());
        }
        after = range.last;
    }
    Ok(())
}

/// Canonical ranges over `journal` (ascending sequences): snapped to existing
/// events and merged where no journal event lies between them, so two
/// selections of the same events compare equal.
pub fn canonicalize(journal: &[i32], ranges: &[SequenceRange]) -> Vec<SequenceRange> {
    let mut out: Vec<SequenceRange> = Vec::new();
    let mut extending = false;
    let mut cursor = ranges.iter().peekable();
    for &sequence in journal {
        while cursor.peek().is_some_and(|range| range.last < sequence) {
            cursor.next();
        }
        let included = cursor.peek().is_some_and(|range| range.contains(sequence));
        match (included, extending) {
            (true, true) => {
                if let Some(last) = out.last_mut() {
                    last.last = sequence;
                }
            }
            (true, false) => out.push(SequenceRange::new(sequence, sequence)),
            _ => {}
        }
        extending = included;
    }
    out
}

/// One journal position as branch selection sees it.
#[derive(Clone, Copy, Debug)]
pub struct JournalEntry {
    /// Canonical sequence.
    pub sequence: i32,
    /// Turn the event belongs to, if any.
    pub turn: Option<TurnId>,
    /// The event ends its turn.
    pub terminal: bool,
}

/// Canonical selection for a branch: shape-checked, canonicalized and made of
/// whole turns, so a selected turn never loses its tool results or terminal.
/// With `settled`, every selected turn must also have ended: a branch change
/// must not resurrect an open turn the authority no longer tracks.
pub fn canonical_selection(
    journal: &[JournalEntry],
    ranges: &[SequenceRange],
    head: i32,
    settled: bool,
) -> Result<Vec<SequenceRange>, String> {
    validate_ranges(ranges, head)?;
    let sequences: Vec<i32> = journal.iter().map(|entry| entry.sequence).collect();
    let canonical = canonicalize(&sequences, ranges);
    // (events, selected events, ended)
    let mut turns: HashMap<TurnId, (usize, usize, bool)> = HashMap::new();
    for entry in journal {
        if let Some(turn) = entry.turn {
            let state = turns.entry(turn).or_default();
            state.0 += 1;
            state.2 |= entry.terminal;
            if contains(&canonical, entry.sequence) {
                state.1 += 1;
            }
        }
    }
    for (total, selected, ended) in turns.values() {
        if *selected > 0 && selected != total {
            return Err("history branch must select whole turns".into());
        }
        if settled && *selected > 0 && !ended {
            return Err("history branch includes a turn that has not ended".into());
        }
    }
    Ok(canonical)
}

/// Whether `sequence` lies in `ranges`.
pub fn contains(ranges: &[SequenceRange], sequence: i32) -> bool {
    ranges.iter().any(|range| range.contains(sequence))
}

/// The prefix of `ranges` through `through`, or `None` when `through` is not
/// covered. Used both for rewinding and for a checkpoint's lineage prefix.
pub fn clip(ranges: &[SequenceRange], through: i32) -> Option<Vec<SequenceRange>> {
    if !contains(ranges, through) {
        return None;
    }
    Some(
        ranges
            .iter()
            .filter(|range| range.first <= through)
            .map(|range| SequenceRange::new(range.first, range.last.min(through)))
            .collect(),
    )
}

/// Stable digest of a JSON value, for retained command and import identities.
pub fn json_digest(value: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(value.to_string().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Stable digest of a lineage prefix, for authority storage keys.
pub fn lineage_digest(ranges: &[SequenceRange]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for range in ranges {
        hasher.update(range.first.to_be_bytes());
        hasher.update(range.last.to_be_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(first: i32, last: i32) -> SequenceRange {
        SequenceRange::new(first, last)
    }

    #[test]
    fn canonical_ranges_snap_to_events_and_merge_across_empty_gaps() {
        let journal = [1, 2, 4, 5, 7];
        assert_eq!(
            canonicalize(&journal, &[r(1, 2), r(3, 4), r(7, 9)]),
            [r(1, 4), r(7, 7)]
        );
        assert_eq!(
            canonicalize(&journal, &[r(1, 2), r(3, 5), r(6, 9)]),
            [r(1, 7)],
            "sequence gaps without events do not split a range"
        );
        assert_eq!(canonicalize(&journal, &[r(3, 3)]), []);
        assert_eq!(canonicalize(&journal, &[r(1, 7)]), [r(1, 7)]);
    }

    #[test]
    fn selection_refuses_split_turns_and_malformed_ranges() {
        let (a, b) = (TurnId::new(), TurnId::new());
        let entry = |sequence, turn, terminal| JournalEntry {
            sequence,
            turn,
            terminal,
        };
        let journal = [
            entry(1, None, false),
            entry(2, Some(a), false),
            entry(3, Some(a), true),
            entry(4, None, false),
            entry(5, Some(b), false),
            entry(6, Some(b), false),
        ];
        let select =
            |ranges: &[SequenceRange], settled| canonical_selection(&journal, ranges, 6, settled);
        assert_eq!(select(&[r(1, 3)], true).unwrap(), [r(1, 3)]);
        assert_eq!(
            select(&[r(1, 3), r(5, 6)], false).unwrap(),
            [r(1, 3), r(5, 6)]
        );
        assert!(
            select(&[r(1, 3), r(5, 6)], true).is_err(),
            "an open turn cannot be selected by a branch change"
        );
        assert!(select(&[r(1, 2)], false).is_err());
        assert!(select(&[r(3, 4), r(1, 2)], false).is_err());
        assert!(select(&[r(1, 7)], false).is_err());
        assert!(select(&[r(0, 1)], false).is_err());
        assert_eq!(select(&[], true).unwrap(), []);
    }

    #[test]
    fn clip_keeps_ancestors_and_refuses_sequences_off_the_branch() {
        let branch = [r(1, 3), r(7, 9)];
        assert_eq!(clip(&branch, 2).unwrap(), [r(1, 2)]);
        assert_eq!(clip(&branch, 8).unwrap(), [r(1, 3), r(7, 8)]);
        assert!(clip(&branch, 5).is_none());
        assert_ne!(
            lineage_digest(&clip(&branch, 8).unwrap()),
            lineage_digest(&[r(1, 3), r(4, 5)])
        );
    }

    #[test]
    fn rewinding_a_linear_lineage_clips_its_active_branch() {
        let lineage = HistoryLineage::linear(Some(1), 9);
        assert_eq!(lineage.rewind_ranges(4).unwrap(), [r(1, 4)]);
        assert_eq!(lineage.rewind_ranges(0).unwrap(), []);
        assert!(lineage.rewind_ranges(10).is_none());
        assert!(HistoryLineage::linear(None, 0).active_ranges().is_empty());
    }
}

//! Log shipping and restore: the cell's loss window on quorum loss.
//!
//! Data plane v2 design, "Disaster recovery": a cell that loses quorum (two
//! zones, correlated disks) is rebuilt from its last archived snapshot plus
//! the log its leader shipped to the object store since.
//!
//! Decisions:
//! - The leader ships committed (applied) entries, an object per
//!   [`SHIP_EVERY`] holding a contiguous index range
//!   (`cells/<cell>/log/<first>-<last>.json`). A range holds the same
//!   entries whoever ships it, so a new leader that ships an overlapping
//!   range again changes nothing, and restore takes each index once.
//! - A new leader resumes after the highest shipped index in the store; its
//!   own log has everything after it unless a snapshot purged it, in which
//!   case it ships from its first entry and restore starts from an archived
//!   snapshot at or past the gap.
//! - If shipping falls more than [`BEHIND_LIMIT`] behind, the cell stops
//!   admitting new work ([`Shipper::behind`]) until it catches up, so the
//!   loss window is a bound and not a hope.
//! - Restore never guesses: a hole in the shipped log after the snapshot is
//!   an error, not a cell with some entries missing.
//!
//! Not here yet (design, plan step 9): incarnations for a rebuilt cell,
//! shipping before an at-most-once tool runs, and retention tied to restore
//! points.

use std::time::{Duration, Instant};

use openraft::{EntryPayload, RaftLogReader, StoredMembership};
use parking_lot::Mutex;

use crate::state::{CellId, CellState};
use crate::store::Store;
use crate::types::Entry;
use crate::wal::Wal;

/// How often the leader ships what committed since the last object.
pub const SHIP_EVERY: Duration = Duration::from_secs(1);

/// Unshipped work older than this stops admission.
pub const BEHIND_LIMIT: Duration = Duration::from_secs(5);

/// Entries per shipped object, at most; a backlog ships in several.
const MAX_ENTRIES: usize = 4096;

/// Ships one cell's committed log from its leader.
pub struct Shipper {
    store: Store,
    cell: CellId,
    wal: Wal,
    state: Mutex<Progress>,
}

#[derive(Default)]
struct Progress {
    /// Highest shipped index; `None` until read from the store (after a
    /// leadership change, the store is the truth).
    shipped: Option<u64>,
    /// When the oldest entry not shipped yet was first seen.
    pending_since: Option<Instant>,
}

impl Shipper {
    pub fn new(store: Store, cell: CellId, wal: Wal) -> Self {
        Self {
            store,
            cell,
            wal,
            state: Mutex::default(),
        }
    }

    /// This node stopped leading: the next leader term resumes from the
    /// store.
    pub fn reset(&self) {
        *self.state.lock() = Progress::default();
    }

    /// Highest index known shipped by this node.
    pub fn shipped(&self) -> Option<u64> {
        self.state.lock().shipped
    }

    /// Whether unshipped entries have waited longer than [`BEHIND_LIMIT`].
    pub fn behind(&self) -> bool {
        self.state
            .lock()
            .pending_since
            .is_some_and(|since| since.elapsed() > BEHIND_LIMIT)
    }

    /// Note that entries up to `applied` exist; call on every tick.
    pub fn observe(&self, applied: u64) {
        let mut state = self.state.lock();
        if state.shipped.is_none_or(|shipped| applied > shipped) {
            state.pending_since.get_or_insert_with(Instant::now);
        }
    }

    /// Ship every entry applied up to `applied` that is not shipped yet.
    /// Returns the highest shipped index.
    pub async fn ship(&self, applied: u64) -> anyhow::Result<u64> {
        let resume = self.state.lock().shipped;
        let mut shipped = match resume {
            Some(shipped) => shipped,
            None => self
                .store
                .shipped(self.cell)
                .await?
                .iter()
                .map(|(_, last)| *last)
                .max()
                .unwrap_or(0),
        };
        while shipped < applied {
            let mut wal = self.wal.clone();
            let entries: Vec<Entry> = wal
                .try_get_log_entries(shipped + 1..=applied)
                .await
                .map_err(|error| anyhow::anyhow!("read log: {error}"))?
                .into_iter()
                .take(MAX_ENTRIES)
                .collect();
            let (Some(first), Some(last)) = (entries.first(), entries.last()) else {
                break;
            };
            let (first, last) = (first.log_id.index, last.log_id.index);
            if first != shipped + 1 {
                tracing::warn!(
                    cell = self.cell,
                    from = shipped + 1,
                    first,
                    "log before {first} was purged into a snapshot before it shipped"
                );
            }
            let bytes = serde_json::to_vec(&entries)?;
            self.store.put_log(self.cell, first, last, bytes).await?;
            shipped = last;
        }
        let mut state = self.state.lock();
        state.shipped = Some(shipped);
        if shipped >= applied {
            state.pending_since = None;
        }
        Ok(shipped)
    }
}

/// What a restore found.
#[derive(Debug)]
pub struct Restored {
    pub state: CellState,
    /// Index of the archived snapshot it started from (0: none).
    pub snapshot: u64,
    /// Index of the last entry applied.
    pub last: u64,
}

/// Rebuild `cell`'s state from the object store alone: the newest archived
/// snapshot, then every shipped entry after it, in order.
pub async fn restore(store: &Store, cell: CellId) -> anyhow::Result<Restored> {
    let mut state = CellState::default();
    for bytes in store.snapshots(cell).await? {
        let copy = crate::sm::decode_archived(&bytes)?;
        if index(&copy) >= index(&state) {
            state = copy;
        }
    }
    let snapshot = index(&state);
    let mut last = snapshot;
    for (first, end) in store.shipped(cell).await? {
        if end <= last {
            continue;
        }
        if first > last + 1 {
            anyhow::bail!(
                "shipped log of cell {cell} has a hole: {} to {}",
                last + 1,
                first - 1
            );
        }
        let entries: Vec<Entry> = serde_json::from_slice(&store.get_log(cell, first, end).await?)?;
        let mut changes = Vec::new();
        for entry in entries {
            if entry.log_id.index <= last {
                continue;
            }
            state.last_applied = Some(entry.log_id);
            match entry.payload {
                EntryPayload::Blank => {}
                EntryPayload::Normal(command) => {
                    state.apply(command, &mut changes);
                }
                EntryPayload::Membership(membership) => {
                    state.membership = StoredMembership::new(Some(entry.log_id), membership);
                }
            }
            last = entry.log_id.index;
            changes.clear();
        }
    }
    Ok(Restored {
        state,
        snapshot,
        last,
    })
}

fn index(state: &CellState) -> u64 {
    state.last_applied.map_or(0, |id| id.index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_hole_in_the_shipped_log_fails_the_restore() {
        let store = Store::open("memory:///").unwrap();
        store.put_log(7, 5, 6, b"[]".to_vec()).await.unwrap();
        let error = restore(&store, 7).await.unwrap_err();
        assert!(error.to_string().contains("hole: 1 to 4"), "{error}");
    }

    #[tokio::test]
    async fn an_empty_store_restores_an_empty_cell() {
        let store = Store::open("memory:///").unwrap();
        let restored = restore(&store, 7).await.unwrap();
        assert_eq!((restored.snapshot, restored.last), (0, 0));
        assert!(restored.state.sessions.is_empty());
    }
}

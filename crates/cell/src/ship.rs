//! Log shipping: the cell's loss window on quorum loss.
//!
//! Data plane v2 design, "Disaster recovery": a cell that loses quorum (two
//! zones, correlated disks) is rebuilt from its last archived snapshot plus
//! the log its leader shipped to the object store since (`restore.rs`).
//!
//! Decisions:
//! - The leader ships committed (applied) entries, an object per
//!   [`SHIP_EVERY`] holding a contiguous index range of the cell's current
//!   incarnation (`restore_points.rs`). A range holds the same entries
//!   whoever ships it, so a new leader that ships an overlapping range
//!   again changes nothing, and restore takes each index once.
//! - A new leader resumes after the highest shipped index in the store; its
//!   own log has everything after it unless a snapshot purged it, in which
//!   case it ships from its first entry and restore starts from an archived
//!   snapshot at or past the gap.
//! - If shipping falls more than [`BEHIND_LIMIT`] behind, the cell stops
//!   admitting new work ([`Shipper::behind`]) until it catches up, so the
//!   loss window is a bound and not a hope.
//! - Some writes are shipped before they are answered ([`ships_first`]):
//!   the start of a tool call that is not safe to repeat, a session's
//!   create, and every step of a move. A rebuild then knows every call
//!   that started (it reports a lost outcome as unknown instead of running
//!   the call again), never loses a session someone was told exists, and
//!   never finds a session in two cells. What a rebuild can lose is the
//!   last [`SHIP_EVERY`] or so of turn progress, never an effect or an
//!   identity. The cost is one object write on those paths only.
//! - Shipping is one at a time per node: the loop and the writes that
//!   ship first share one lock, so progress is never read half-updated.

use std::time::{Duration, Instant};

use openraft::RaftLogReader;
use parking_lot::Mutex;

use crate::state::{CellId, Command};
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
    busy: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Progress {
    /// The incarnation `shipped` counts in.
    inc: u64,
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
            busy: tokio::sync::Mutex::new(()),
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

    /// Ship every entry through `applied` of incarnation `inc` that is not
    /// shipped yet. Returns the highest shipped index.
    pub async fn ship(&self, applied: u64, inc: u64) -> anyhow::Result<u64> {
        let _one = self.busy.lock().await;
        let resume = {
            let mut state = self.state.lock();
            if state.inc != inc {
                *state = Progress {
                    inc,
                    ..Progress::default()
                };
            }
            state.shipped
        };
        let mut shipped = match resume {
            Some(shipped) => shipped,
            None => self
                .store
                .shipped(self.cell, inc)
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
            self.store
                .put_log(self.cell, inc, first, last, bytes)
                .await?;
            shipped = last;
        }
        let mut state = self.state.lock();
        state.shipped = Some(shipped);
        if shipped >= applied {
            state.pending_since = None;
        }
        Ok(shipped)
    }

    /// Ship through `index` now, unless it already is.
    pub async fn flush(&self, index: u64, inc: u64) -> anyhow::Result<()> {
        {
            let state = self.state.lock();
            if state.inc == inc && state.shipped.is_some_and(|shipped| shipped >= index) {
                return Ok(());
            }
        }
        self.ship(index, inc).await.map(|_| ())
    }
}

/// Whether `command` is answered only once it is shipped (see the module
/// decisions).
pub fn ships_first(command: &Command) -> bool {
    use everruns::hosting::spi::session_commands::SessionCommand;
    use everruns_contracts::runtime::events::EventData;
    use everruns_contracts::tool_types::SideEffectClass;
    match command {
        Command::Batch { commands } | Command::Chain { commands } => {
            commands.iter().any(ships_first)
        }
        Command::Append { request, .. } => match &request.data {
            // An unknown tool is treated as the default class: not safe to
            // repeat.
            EventData::ToolStarted(started) => matches!(
                started.side_effect_class,
                None | Some(SideEffectClass::AtMostOnce)
            ),
            _ => false,
        },
        Command::Host { command, .. } => matches!(command, SessionCommand::Create { .. }),
        Command::Place { .. }
        | Command::Freeze { .. }
        | Command::Import { .. }
        | Command::Activate { .. }
        | Command::Forward { .. }
        | Command::Drop { .. }
        | Command::Thaw { .. }
        | Command::Revert { .. } => true,
        _ => false,
    }
}

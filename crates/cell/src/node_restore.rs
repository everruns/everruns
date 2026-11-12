//! The leader's upkeep of its cell's restore points (`restore_points.rs`):
//! a snapshot at least every [`RESTORE_POINT_EVERY`] and before a drain,
//! answering ship-first writes, and collecting what the restore window no
//! longer needs.
//!
//! Decisions:
//! - Snapshots by time as well as by log length. A quiet cell may go hours
//!   without reaching `snapshot_every` entries, and a point-in-time
//!   restore then replays everything since its last snapshot; a snapshot
//!   each quarter hour bounds that replay and gives thinning a copy in
//!   every hour of the window.
//! - Only a leader collects, each cell its own snapshots and log; the
//!   directory cell's leader also collects retired segments, which belong
//!   to no one cell. Deleting is idempotent, so two leaders overlapping
//!   during an election do no harm.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::node::Node;

/// The longest a leader lets its cell go without a snapshot.
pub const RESTORE_POINT_EVERY: Duration = Duration::from_secs(15 * 60);

/// How often a leader collects old restore points.
const COLLECT_EVERY: Duration = Duration::from_secs(60 * 60);

impl Node {
    pub(crate) async fn restore_points_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut collected: Option<Instant> = None;
        let mut drain_snapshot = false;
        loop {
            tick.tick().await;
            if !self.is_leader() {
                continue;
            }
            let draining = self
                .gateway
                .get()
                .is_some_and(|g| g.draining().contains(&self.cfg.cell));
            let snapshot_at = self
                .raft
                .metrics()
                .borrow()
                .snapshot
                .map_or(0, |id| id.index);
            let applied = *self.shared.applied.borrow();
            let due = self.shared.last_snapshot_age() >= RESTORE_POINT_EVERY;
            // One snapshot as a drain starts: the restore point closest to
            // the cell's sessions leaving.
            let before_drain = draining && !drain_snapshot;
            drain_snapshot = draining;
            if applied > snapshot_at && (due || before_drain) {
                let _ = self.raft.trigger().snapshot().await;
            }
            if collected.is_none_or(|at| at.elapsed() >= COLLECT_EVERY) {
                collected = Some(Instant::now());
                let segments = self
                    .gateway
                    .get()
                    .is_some_and(|g| g.directory_cell().ok() == Some(self.cfg.cell));
                let window = crate::restore_points::restore_window();
                match self
                    .shared
                    .store
                    .collect(self.cfg.cell, segments, window, crate::node::now_ms())
                    .await
                {
                    Ok(done) => {
                        tracing::info!(cell = self.cfg.cell, ?done, "restore points collected")
                    }
                    Err(error) => {
                        tracing::warn!(cell = self.cfg.cell, %error, "collecting restore points failed")
                    }
                }
            }
        }
    }

    /// Ship the log through `index` before answering a write that must not
    /// be lost (`ship::ships_first`), retrying until `deadline`.
    pub(crate) async fn ship_first(&self, index: u64, deadline: Instant) -> anyhow::Result<()> {
        let started = Instant::now();
        let mut backoff = Duration::from_millis(20);
        loop {
            let inc = self.shared.state.read().incarnation;
            let applied = &self.shared.applied;
            match self.shipper.flush(index, inc, || *applied.borrow()).await {
                Ok(()) => {
                    self.prom.ship_first_seconds.observe(&[], started.elapsed());
                    return Ok(());
                }
                Err(error) if Instant::now() >= deadline => {
                    return Err(error.context("ship before answering"));
                }
                Err(error) => tracing::debug!(%error, "ship-first retry"),
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_millis(500));
        }
    }
}

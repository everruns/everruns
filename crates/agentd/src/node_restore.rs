//! The leader's upkeep of its shard's restore points (`restore_points.rs`):
//! a snapshot at least every [`RESTORE_POINT_EVERY`] and before a drain,
//! sessions' history stored as segments,
//! shipping the log each round, answering ship-first writes, and collecting what the restore window no
//! longer needs.
//!
//! Decisions:
//! - Snapshots by time as well as by log length. A quiet shard may go hours
//!   without reaching `snapshot_every` entries, and a point-in-time
//!   restore then replays everything since its last snapshot; a snapshot
//!   each quarter hour bounds that replay and gives thinning a copy in
//!   every hour of the window.
//! - Only a leader collects, each shard its own snapshots and log; the
//!   directory shard's leader also collects retired segments, which belong
//!   to no one shard. Deleting is idempotent, so two leaders overlapping
//!   during an election do no harm.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;

use crate::node::Node;

/// The longest a leader lets its shard go without a snapshot.
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
                .is_some_and(|g| g.draining().contains(&self.cfg.shard));
            let snapshot_at = self
                .raft
                .metrics()
                .borrow()
                .snapshot
                .map_or(0, |id| id.index);
            let applied = *self.shared.applied.borrow();
            let due = self.shared.last_snapshot_age() >= RESTORE_POINT_EVERY;
            // One snapshot as a drain starts: the restore point closest to
            // the shard's sessions leaving.
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
                    .is_some_and(|g| g.directory_shard().ok() == Some(self.cfg.shard));
                let window = crate::restore_points::restore_window();
                match self
                    .shared
                    .store
                    .collect(self.cfg.shard, segments, window, crate::node::now_ms())
                    .await
                {
                    Ok(done) => {
                        tracing::info!(shard = self.cfg.shard, ?done, "restore points collected")
                    }
                    Err(error) => {
                        tracing::warn!(shard = self.cfg.shard, %error, "collecting restore points failed")
                    }
                }
            }
        }
    }

    /// Leader only: store sessions' unstored events in the object store and
    /// record the segments with `Stored`, so every replica can unload them.
    /// Legacy segments a compaction replaced are marked retired; replaced
    /// pack entries need nothing (`packs.rs` sweeps unnamed packs).
    pub(crate) async fn store_events(&self, idle_for: Duration) {
        let stored = self.shared.flush(512, idle_for, self.cfg.node_id).await;
        if stored.is_empty() {
            return;
        }
        let commands = stored
            .iter()
            .map(|f| crate::state::Command::Stored {
                session: f.session,
                seg: f.seg,
            })
            .collect();
        if let Err(error) = self
            .propose(crate::state::Command::Batch { commands })
            .await
        {
            tracing::debug!(%error, "recording stored segments failed");
            return;
        }
        // Decision: retire only what the applied state no longer names. A
        // compaction the state ignored (the session moved away, or changed
        // under the pass) leaves its old segments in use, and a retired
        // segment is deleted a window later.
        let retire: Vec<_> = {
            let state = self.shared.state.read();
            stored
                .into_iter()
                .filter(|f| f.compact)
                .filter_map(|f| {
                    let s = state.sessions.get(&f.session)?;
                    if !s.segments.contains(&f.seg) {
                        return None;
                    }
                    let gone: Vec<_> = f
                        .replaced
                        .into_iter()
                        .filter(|old| old.pack.is_none() && !s.segments.contains(old))
                        .collect();
                    (!gone.is_empty()).then_some((f.session, gone))
                })
                .collect()
        };
        if retire.is_empty() {
            return;
        }
        let store = self.shared.store.clone();
        tokio::spawn(async move {
            futures::stream::iter(retire)
                .for_each_concurrent(16, |(session, gone)| {
                    let store = store.clone();
                    async move {
                        if let Err(error) = store.retire_segments(session, &gone).await {
                            tracing::debug!(%session, %error, "retiring replaced segments failed");
                        }
                    }
                })
                .await;
        });
    }

    /// While this node leads: ship the committed log to the object store
    /// (see `ship`).
    pub(crate) async fn ship_loop(self: Arc<Self>) {
        let mut tick = tokio::time::interval(crate::ship::SHIP_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let leading = self.is_leader();
            self.shared.set_leading(leading);
            if !leading {
                self.shipper.reset();
                continue;
            }
            let applied = *self.shared.applied.borrow();
            let inc = self.shared.state.read().incarnation;
            self.shipper.observe(applied);
            if !self.shipper.due(applied, inc) {
                continue;
            }
            let waiting = self.shipper.waiting_since();
            let shipped = self.shipper.ship(applied, inc).await;
            if let (Ok(_), Some(since)) = (&shipped, waiting) {
                // The wait is noticed at a tick, so the oldest entry may
                // have been applied up to one tick earlier: count it.
                let lag = since.elapsed() + crate::ship::SHIP_TICK;
                self.prom.ship_lag_seconds.observe(&[], lag);
                self.prom.ship_lag_window.observe(lag);
            }
            if let Err(error) = shipped {
                tracing::warn!(shard = self.cfg.shard, %error, "log shipping failed; retrying");
            }
        }
    }

    /// Make the write at `index` survive the loss of every replica before it
    /// is answered (`ship::ships_first`): shipped to the object store,
    /// retrying until `deadline`. Writers waiting together share one object
    /// (`Shipper::flush`).
    ///
    /// Decision (2026-11-25): the store, not every voter's disk. Phase 2
    /// answered these writes once all three voters held them on disk, and
    /// shipped them only with the log every 10 s. The disaster drill then
    /// lost every disk of a shard and with them 687 sessions' `tool.started`
    /// records, so a resent message ran those tools again, and session
    /// creates the shard had answered. The 75k goal holds that such writes
    /// reach the store (U9) and that a whole lost shard finishes every
    /// session (E2), so they wait for the store again, as at step 9.
    pub(crate) async fn ship_first(&self, index: u64, deadline: Instant) -> anyhow::Result<()> {
        let started = Instant::now();
        let mut backoff = Duration::from_millis(20);
        loop {
            let inc = self.shared.state.read().incarnation;
            let applied = &self.shared.applied;
            match self.shipper.flush(index, inc, || *applied.borrow()).await {
                Ok(()) => {
                    self.prom.ship_first_total.inc(&[("path", "store")]);
                    self.prom.ship_first_seconds.observe(&[], started.elapsed());
                    self.prom.ship_first_window.observe(started.elapsed());
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

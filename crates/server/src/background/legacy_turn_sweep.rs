//! TEMPORARY rolling-deploy bridge for the session actor cutover
//! (actor-based design, step 6c). Step 7 deletes this module with the
//! per-step turn workflow.
//!
//! Decisions:
//! - The cutover migration (`*_session_actor_cutover.sql`) hands the turn tasks in flight at migration time to
//!   session actors. During a rolling deploy, replicas of the previous
//!   release keep running for a while: an old server can still start a turn
//!   as a `process_input` task, and an old worker that finishes a step
//!   enqueues the next one. Nothing in this release runs those, so the
//!   session would stall. Every few seconds this sweep converts them the way
//!   the migration did (`Database::convert_legacy_turn_tasks`): the session's
//!   actor is marked runnable and the old tasks and their `turn_workflow` are
//!   cancelled, in one statement, idempotently, on every server replica.
//! - Old workers cannot pick up what this release creates: they claim only
//!   durable activities by type, and turns of this release are `session_actors`
//!   rows that only the new lease RPCs hand out. The reverse direction is the
//!   one that needs bridging, and this sweep is that bridge.
//! - Workers hear about the actors it marks: the actor table's trigger
//!   notifies under PostgreSQL LISTEN; under NATS this sweep publishes the
//!   same `session_actor` payload.

use std::sync::Arc;
use std::time::Duration;

use crate::background::supervised_task::{RestartPolicy, TaskSupervisor};
use crate::live_updates::task_notifications::TaskBroadcaster;
use crate::storage::{LegacyTurnSweep, StorageBackend};

/// How often the sweep runs.
pub const LEGACY_TURN_SWEEP_INTERVAL: Duration = Duration::from_secs(5);
/// Most old tasks one pass converts; a full pass runs again at once.
pub const LEGACY_TURN_SWEEP_BATCH: i64 = 200;
/// A claimed old task whose worker went silent this long is converted too.
/// Longer than the stale-task reaper's reclaim window, so a live old worker
/// finishing its step is never raced.
pub const LEGACY_TURN_STALE_CLAIM: Duration = Duration::from_secs(120);

/// Convert pending old-path turn tasks until none are left (at most a few
/// batches per call). Returns the total.
pub async fn sweep_legacy_turn_tasks(
    db: &StorageBackend,
    broadcaster: Option<&TaskBroadcaster>,
) -> anyhow::Result<LegacyTurnSweep> {
    let mut total = LegacyTurnSweep::default();
    for _ in 0..10 {
        let pass = db
            .convert_legacy_turn_tasks(LEGACY_TURN_SWEEP_BATCH, LEGACY_TURN_STALE_CLAIM)
            .await?;
        total.retired_tasks += pass.retired_tasks;
        total.marked_actors += pass.marked_actors;
        if pass.retired_tasks < LEGACY_TURN_SWEEP_BATCH {
            break;
        }
    }
    if total.marked_actors > 0 {
        tracing::info!(
            retired_tasks = total.retired_tasks,
            marked_actors = total.marked_actors,
            "Handed old-path turn tasks to session actors (rolling deploy)"
        );
        if let Some(broadcaster) = broadcaster {
            broadcaster
                .notify_task_available(crate::turns::SESSION_ACTOR_NOTIFICATION)
                .await;
        }
    }
    Ok(total)
}

/// Run [`sweep_legacy_turn_tasks`] every [`LEGACY_TURN_SWEEP_INTERVAL`] under
/// the supervisor.
pub fn spawn_legacy_turn_sweep(
    supervisor: &mut TaskSupervisor,
    db: Arc<StorageBackend>,
    broadcaster: Option<Arc<TaskBroadcaster>>,
) {
    supervisor.spawn(
        "legacy_turn_sweep",
        RestartPolicy::always_after(Duration::from_secs(5)),
        move || {
            let db = db.clone();
            let broadcaster = broadcaster.clone();
            async move {
                let mut tick = tokio::time::interval(LEGACY_TURN_SWEEP_INTERVAL);
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tick.tick().await;
                    if let Err(error) = sweep_legacy_turn_tasks(&db, broadcaster.as_deref()).await {
                        tracing::warn!(error = %error, "Old-path turn task sweep failed");
                    }
                }
            }
        },
    );
}

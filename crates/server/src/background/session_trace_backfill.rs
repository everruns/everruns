// Session trace backfill: projects sessions whose trace index is behind.
//
// Decision: the write path projects new events as they land, so this job only
// finds sessions written before the index existed, or whose write-path pass was
// skipped or failed. It takes the most recently written sessions first, so the
// sessions people open next are indexed first, and spends a bounded number of
// events per run so a large backlog drains over several runs instead of one
// long one. A trace read catches its own session up regardless.

use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, info, warn};

use crate::background::cluster_jobs::ClusterJob;
use crate::storage::{StorageBackend, TRACE_PASS_BUDGET, TraceCatchUp};

/// Durable schedule name of the session trace backfill job.
pub const SESSION_TRACE_BACKFILL_SCHEDULE: &str = "session-trace-backfill";
/// Activity type the session trace backfill schedule enqueues.
pub const SESSION_TRACE_BACKFILL_ACTIVITY: &str = "session_trace_backfill";

/// Sessions considered per run.
const SESSIONS_PER_RUN: i64 = 50;
/// Projection passes per run, across all sessions.
const PASSES_PER_RUN: usize = 20;

/// The backfill as a cluster-once job, once a minute.
pub fn backfill_job(db: Arc<StorageBackend>) -> ClusterJob {
    ClusterJob::every(
        SESSION_TRACE_BACKFILL_SCHEDULE,
        SESSION_TRACE_BACKFILL_ACTIVITY,
        "Projects sessions whose trace index is behind their events.",
        Duration::from_secs(60),
        move || {
            let db = db.clone();
            Box::pin(async move {
                match run_backfill(&db).await {
                    Ok(0) => debug!("session trace index up to date"),
                    Ok(passes) => info!(passes, "session trace backfill ran"),
                    Err(error) => warn!(%error, "session trace backfill failed"),
                }
            })
        },
    )
}

/// One backfill run. Returns the number of projection passes it made.
pub async fn run_backfill(db: &StorageBackend) -> anyhow::Result<usize> {
    let database = db.database();
    let mut passes = 0;
    for session_id in database.sessions_behind_trace(SESSIONS_PER_RUN).await? {
        loop {
            if passes >= PASSES_PER_RUN {
                return Ok(passes);
            }
            passes += 1;
            match database
                .catch_up_session_trace(session_id, TRACE_PASS_BUDGET, false)
                .await?
            {
                TraceCatchUp::Partial { .. } => continue,
                TraceCatchUp::UpToDate { .. } | TraceCatchUp::Busy => break,
            }
        }
    }
    Ok(passes)
}

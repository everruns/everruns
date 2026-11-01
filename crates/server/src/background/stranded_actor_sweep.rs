// Stranded actor sweep: the backstop for a session left `active` with nothing
// to run it.
//
// Decision: a session's status says `active` from its turn's start until a
// holder writes `session.idled`, and only a leased or runnable actor ever
// writes that. Every path that ends a lease without the idle used to rely on
// the next lease to owe it, so an actor released (or withdrawn) without it
// left the session `active` forever: no lease would ever come (production,
// 2026-10-29: a poisoned lease released a closed, un-idled turn, and the
// session stayed `active` for days). This cluster-once job finds such
// sessions and marks their actors resumable, so a worker folds the log and
// finishes what the log still owes: an open turn resumes, a closed one is
// idled (`Database::recover_stranded_session_actors`).
//
// Decision: `GRACE` keeps the sweep off sessions whose state is only between
// two writes: a cancel withdraws the actor before it idles the session, and
// intake commits the runnable mark with the input. A driven turn always holds
// a lease, so a long turn is never a candidate however long it runs.

use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, warn};

use crate::background::cluster_jobs::ClusterJob;
use crate::storage::StorageBackend;

/// Durable schedule name of the stranded actor sweep.
pub const STRANDED_ACTOR_SWEEP_SCHEDULE: &str = "stranded-actor-sweep";
/// Activity type the stranded actor sweep schedule enqueues.
pub const STRANDED_ACTOR_SWEEP_ACTIVITY: &str = "stranded_actor_sweep";

const SWEEP_PERIOD: Duration = Duration::from_secs(60);
/// How long a session and its actor must be untouched before they count as
/// stranded.
const GRACE: Duration = Duration::from_secs(300);
/// Most actors re-marked per run; the next run picks up the rest.
const SWEEP_BATCH: i64 = 100;

/// The sweep as a cluster-once job, once a minute.
pub fn sweep_job(db: Arc<StorageBackend>) -> ClusterJob {
    ClusterJob::every(
        STRANDED_ACTOR_SWEEP_SCHEDULE,
        STRANDED_ACTOR_SWEEP_ACTIVITY,
        "Resumes sessions left active with no actor leased or runnable, so their turns finish or idle.",
        SWEEP_PERIOD,
        move || {
            let db = db.clone();
            Box::pin(async move {
                match db.recover_stranded_session_actors(GRACE, SWEEP_BATCH).await {
                    Ok(sessions) if sessions.is_empty() => debug!("no stranded session actors"),
                    Ok(sessions) => {
                        for session_id in &sessions {
                            warn!(
                                session_id = %session_id,
                                "Session was active with no actor leased or runnable; marked its actor resumable"
                            );
                        }
                    }
                    Err(error) => warn!(%error, "Stranded actor sweep failed"),
                }
            })
        },
    )
}

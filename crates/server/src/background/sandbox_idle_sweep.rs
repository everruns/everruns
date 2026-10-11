// Sandbox idle sweep: the durable backstop for idle pause.
//
// Decision: idle pause is armed as an in-process timer when a session goes
// idle (`SessionSandboxService::schedule_idle_pause`). That timer lives in one
// server's memory, so a restart, a replica change, or a failed pause leaves the
// sandbox running with nothing to retry it, and a paid provider sandbox keeps
// running until someone notices. This cluster-once job pauses every managed
// sandbox that is still running past its idle limit while its session is not
// running a turn. It waits one sweep period past the limit so the timer, when
// it survives, still acts first.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tracing::{info, warn};
use uuid::Uuid;

use crate::background::cluster_jobs::ClusterJob;
use crate::domains::session_sandbox::SessionSandboxService;

/// Durable schedule name of the Sandbox idle sweep.
pub const SANDBOX_IDLE_SWEEP_SCHEDULE: &str = "sandbox-idle-sweep";
/// Activity type the Sandbox idle sweep schedule enqueues.
pub const SANDBOX_IDLE_SWEEP_ACTIVITY: &str = "sandbox_idle_sweep";

const SWEEP_PERIOD: Duration = Duration::from_secs(60);
/// Most sandboxes paused per run; the next run picks up the rest.
const SWEEP_BATCH: i64 = 50;

/// The sweep as a cluster-once job. Disabled without managed Sandbox lifecycle
/// (no encryption configured), since then nothing can talk to providers.
pub fn idle_sweep_job(pool: PgPool, service: Option<Arc<SessionSandboxService>>) -> ClusterJob {
    let Some(service) = service else {
        return ClusterJob::disabled(SANDBOX_IDLE_SWEEP_SCHEDULE, SANDBOX_IDLE_SWEEP_ACTIVITY);
    };
    ClusterJob::every(
        SANDBOX_IDLE_SWEEP_SCHEDULE,
        SANDBOX_IDLE_SWEEP_ACTIVITY,
        "Pauses managed Sandboxes still running past their idle limit.",
        SWEEP_PERIOD,
        move || {
            let pool = pool.clone();
            let service = service.clone();
            Box::pin(async move { sweep(&pool, &service).await })
        },
    )
}

async fn sweep(pool: &PgPool, service: &SessionSandboxService) {
    let sessions = match overdue_idle_sandbox_sessions(pool, SWEEP_PERIOD, SWEEP_BATCH).await {
        Ok(sessions) => sessions,
        Err(e) => {
            warn!(error = %e, "Sandbox idle sweep query failed");
            return;
        }
    };
    for session_id in sessions {
        let session_id = everruns_contracts::typed_id::SessionId::from_uuid(session_id);
        match service.pause_if_idle_pause_enabled(session_id).await {
            Ok(true) => info!(session_id = %session_id, "Paused idle Sandbox missed by its timer"),
            Ok(false) => {}
            Err(e) => {
                warn!(session_id = %session_id, error = %e, "Sandbox idle sweep pause failed")
            }
        }
    }
}

/// Sessions whose managed sandbox is running, has had no activity for longer
/// than its idle limit plus `grace`, and whose session is not running a turn.
/// Sandboxes whose lifecycle says `keep_running` are left alone.
pub async fn overdue_idle_sandbox_sessions(
    pool: &PgPool,
    grace: Duration,
    limit: i64,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT s.session_id
        FROM sandboxes s
        JOIN sessions sess ON sess.id = s.session_id
        WHERE s.deleted_at IS NULL
          AND s.observed_state = 'ready'
          AND s.desired_state = 'ready'
          AND COALESCE(s.profile_snapshot->'target'->>'kind', '') NOT IN ('vfs', 'host')
          AND COALESCE(s.profile_snapshot->'lifecycle'->>'idle_action', '') <> 'keep_running'
          AND sess.status IN ('idle', 'started', 'paused')
          AND COALESCE(s.last_activity_at, s.updated_at)
              < now() - make_interval(secs => COALESCE(
                    (s.profile_snapshot->'lifecycle'->>'idle_after_seconds')::double precision,
                    180) + $1)
        ORDER BY s.last_activity_at NULLS FIRST
        LIMIT $2
        "#,
    )
    .bind(grace.as_secs_f64())
    .bind(limit)
    .fetch_all(pool)
    .await
}

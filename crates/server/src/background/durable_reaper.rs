// Server side of durable stale-task reaping.
//
// Decision: reclaim, dead-task terminalization and seal bookkeeping are engine
// logic and live in `everruns_durable::maintenance`. The queue carries only
// activities that are not turns (turns run on session actor leases, whose
// expiry is their recovery), so a dead or sealed task has no session to
// surface to: the handler only reports reap passes that fail.

use std::sync::Arc;
use std::time::Duration;

use everruns_core::{ErrorReport, ErrorScope, SharedErrorReporter};
use everruns_durable::{
    PostgresWorkflowEventStore, ReapHandler, ReaperConfig, StaleTaskReaper, StoreError,
};
use sqlx::PgPool;

use crate::background::supervised_task::{RestartPolicy, TaskSupervisor};

/// Reports failed reap passes to the error reporter.
pub struct ReapErrorReporter {
    error_reporter: SharedErrorReporter,
}

impl ReapErrorReporter {
    pub fn new(error_reporter: SharedErrorReporter) -> Self {
        Self { error_reporter }
    }
}

#[async_trait::async_trait]
impl ReapHandler for ReapErrorReporter {
    fn reap_failed(&self, error: &StoreError) {
        // Detached so a slow vendor reporter cannot stall the reap loop during
        // an upstream outage.
        //
        // A datastore outage fails every pass, every 10 s; the report is
        // bounded like the reaper's own log line (EVE-1328), carrying the
        // count of passes it stands in for.
        let throttle = &everruns_core::report_throttle::REPEATED_FAILURES;
        let Some(suppressed) = throttle.admit("server.stale_task_reclaim", "") else {
            return;
        };
        let reporter = self.error_reporter.clone();
        let message = error.to_string();
        tokio::spawn(async move {
            reporter
                .report(
                    ErrorReport::error("server.stale_task_reclaim", message).with_scope(
                        ErrorScope::new()
                            .with_component("stale_task_reclaim")
                            .with_extra("suppressed_since_last_report", suppressed.to_string()),
                    ),
                )
                .await;
        });
    }
}

/// Run the stale-task reaper on `pool` under the supervisor: every 10 s,
/// reclaiming claims whose heartbeat is older than 30 s.
pub fn spawn_stale_task_reaper(
    supervisor: &mut TaskSupervisor,
    pool: PgPool,
    handler: Arc<ReapErrorReporter>,
) {
    supervisor.spawn(
        "stale_task_reclaim",
        RestartPolicy::always_after(Duration::from_secs(5)),
        move || {
            let reaper = StaleTaskReaper::new(
                Arc::new(PostgresWorkflowEventStore::new(pool.clone())),
                ReaperConfig::default(),
                handler.clone(),
            );
            async move { reaper.run().await }
        },
    );
}

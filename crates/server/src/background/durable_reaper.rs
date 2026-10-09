// Server side of durable stale-task reaping.
//
// Decision: reclaim, dead-task terminalization and seal bookkeeping are engine
// logic and live in `everruns_durable::maintenance`. What a dead or sealed
// task means for an agent turn (fail the turn, idle the session, emit
// `turn.sealed`) is server domain, and plugs in here as a `ReapHandler`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_core::{ErrorReport, ErrorScope, SharedErrorReporter};
use everruns_durable::{
    DeadTaskInfo, PostgresWorkflowEventStore, ReapHandler, ReaperConfig, SealedTaskInfo,
    StaleTaskReaper, StoreError,
};
use sqlx::PgPool;

use crate::background::supervised_task::{RestartPolicy, TaskSupervisor};
use crate::domains::sessions::SessionService;
use crate::services::EventService;

/// Fails the session's turn when its durable task dies or is sealed.
pub struct TurnReapHandler {
    event_service: Arc<EventService>,
    session_service: Arc<SessionService>,
    error_reporter: SharedErrorReporter,
}

impl TurnReapHandler {
    pub fn new(
        event_service: Arc<EventService>,
        session_service: Arc<SessionService>,
        error_reporter: SharedErrorReporter,
    ) -> Self {
        Self {
            event_service,
            session_service,
            error_reporter,
        }
    }
}

#[async_trait]
impl ReapHandler for TurnReapHandler {
    async fn workflow_failed(&self, dead: &DeadTaskInfo, error: &str) {
        crate::background::durable_failure::handle_failed_task(
            &self.event_service,
            &self.session_service,
            dead,
            error,
        )
        .await;
    }

    async fn task_sealed(&self, sealed: &SealedTaskInfo) {
        crate::background::durable_seal::handle_sealed_task(
            &self.event_service,
            &self.session_service,
            sealed,
        )
        .await;
    }

    fn reap_failed(&self, error: &StoreError) {
        // Detached so a slow vendor reporter cannot stall the reap loop during
        // an upstream outage.
        let reporter = self.error_reporter.clone();
        let message = error.to_string();
        tokio::spawn(async move {
            reporter
                .report(
                    ErrorReport::error("server.stale_task_reclaim", message)
                        .with_scope(ErrorScope::new().with_component("stale_task_reclaim")),
                )
                .await;
        });
    }
}

/// Run the stale-task reaper on `pool` under the supervisor: every 10 s,
/// reclaiming claims whose heartbeat is older than 30 s, and resuming turn
/// runs left with no task a minute after their last step completed (a
/// worker that handed off in separate writes and died in between).
pub fn spawn_stale_task_reaper(
    supervisor: &mut TaskSupervisor,
    pool: PgPool,
    handler: Arc<TurnReapHandler>,
) {
    supervisor.spawn(
        "stale_task_reclaim",
        RestartPolicy::always_after(Duration::from_secs(5)),
        move || {
            let reaper = StaleTaskReaper::new(
                Arc::new(PostgresWorkflowEventStore::new(pool.clone())),
                ReaperConfig {
                    stranded_workflow_type: Some(everruns_worker::durable_turn::TURN_WORKFLOW_TYPE),
                    ..ReaperConfig::default()
                },
                handler.clone(),
            );
            async move { reaper.run().await }
        },
    );
}

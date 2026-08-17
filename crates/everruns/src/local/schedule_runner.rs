use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::StreamExt;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use super::{LocalError, LocalResult, LocalScheduleStore, LocalSessionRunner};

/// Runtime settings for [`LocalScheduleRunner`].
#[derive(Clone, Debug)]
pub struct LocalScheduleRunnerConfig {
    /// How often the SQLite store is checked for due schedules.
    pub poll_interval: Duration,
    /// How long an unfinished claim remains exclusive before another runner may recover it.
    /// Failed deliveries also wait this long before becoming claimable again.
    pub claim_timeout: Duration,
    /// Maximum schedules claimed per poll.
    pub batch_size: usize,
}

const MAX_BATCH_SIZE: usize = 1_000;

impl Default for LocalScheduleRunnerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(1),
            claim_timeout: Duration::from_secs(30),
            batch_size: 32,
        }
    }
}

/// Explicitly started in-process executor for schedules in a [`LocalScheduleStore`].
pub struct LocalScheduleRunner {
    store: LocalScheduleStore,
    session_runner: Arc<dyn LocalSessionRunner>,
    config: LocalScheduleRunnerConfig,
}

impl LocalScheduleRunner {
    /// Create a runner using the default polling, claim, and batch limits.
    pub fn new(store: LocalScheduleStore, session_runner: Arc<dyn LocalSessionRunner>) -> Self {
        Self {
            store,
            session_runner,
            config: LocalScheduleRunnerConfig::default(),
        }
    }

    /// Replace the polling, claim, and batch configuration.
    pub fn with_config(mut self, config: LocalScheduleRunnerConfig) -> Self {
        self.config = config;
        self
    }

    /// Spawn the polling task. Retain the returned handle for the host lifetime.
    pub fn start(self) -> LocalResult<LocalScheduleRunnerHandle> {
        if self.config.poll_interval.is_zero() {
            return Err(LocalError::Config(
                "schedule poll interval must be positive".into(),
            ));
        }
        if self.config.claim_timeout.is_zero() {
            return Err(LocalError::Config(
                "schedule claim timeout must be positive".into(),
            ));
        }
        if self.config.batch_size == 0 {
            return Err(LocalError::Config(
                "schedule batch size must be positive".into(),
            ));
        }
        if self.config.batch_size > MAX_BATCH_SIZE {
            return Err(LocalError::Config(format!(
                "schedule batch size must not exceed {MAX_BATCH_SIZE}"
            )));
        }
        let runner_id = uuid::Uuid::now_v7().to_string();
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let join = tokio::spawn(async move {
            tracing::info!(
                runner_id,
                poll_interval_ms = self.config.poll_interval.as_millis(),
                claim_timeout_ms = self.config.claim_timeout.as_millis(),
                batch_size = self.config.batch_size,
                "Local schedule runner started"
            );
            let mut interval = tokio::time::interval(self.config.poll_interval);
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            break;
                        }
                    }
                    _ = interval.tick() => {
                        if let Err(error) = self.poll_once(&runner_id).await {
                            tracing::error!(runner_id, error = %error, "Local schedule poll failed");
                        }
                    }
                }
            }
            tracing::info!(runner_id, "Local schedule runner stopped");
        });
        Ok(LocalScheduleRunnerHandle {
            shutdown_tx,
            join: Some(join),
        })
    }

    async fn poll_once(&self, runner_id: &str) -> everruns_provider::error::Result<()> {
        let routable_session_ids = self.session_runner.routable_session_ids().await?;
        let claimed = self.store.claim_due(
            runner_id,
            Utc::now(),
            self.config.claim_timeout,
            self.config.batch_size,
            routable_session_ids.as_deref(),
        )?;
        if !claimed.is_empty() {
            tracing::debug!(
                runner_id,
                count = claimed.len(),
                "Claimed due local schedules"
            );
        }
        futures::stream::iter(claimed)
            .for_each_concurrent(self.config.batch_size, |claim| async move {
                self.process_claim(claim, runner_id).await;
            })
            .await;
        Ok(())
    }

    async fn process_claim(&self, claim: super::schedule_store::ClaimedSchedule, runner_id: &str) {
        let schedule = &claim.schedule;
        tracing::debug!(
            runner_id,
            schedule_id = %schedule.id,
            session_id = %schedule.session_id,
            "Delivering local schedule"
        );
        let delivery = self.deliver_with_heartbeat(&claim, runner_id).await;
        match delivery {
            Ok(()) => match self.store.complete_delivery(&claim, runner_id, Utc::now()) {
                Ok(()) => tracing::info!(
                    runner_id,
                    schedule_id = %schedule.id,
                    session_id = %schedule.session_id,
                    "Local schedule delivered"
                ),
                Err(error) => tracing::error!(
                    runner_id,
                    schedule_id = %schedule.id,
                    session_id = %schedule.session_id,
                    error = %error,
                    "Local schedule was delivered but its claim could not be completed"
                ),
            },
            Err(error) => {
                let error_text = bounded_error(&error.to_string());
                if let Err(release_error) =
                    self.store
                        .fail_delivery(&claim, runner_id, Utc::now(), &error_text)
                {
                    tracing::error!(
                        runner_id,
                        schedule_id = %schedule.id,
                        session_id = %schedule.session_id,
                        error = %release_error,
                        "Local schedule failed and its claim could not be released"
                    );
                }
                tracing::warn!(
                    runner_id,
                    schedule_id = %schedule.id,
                    session_id = %schedule.session_id,
                    error = %error,
                    "Local schedule delivery failed; occurrence retained for retry"
                );
            }
        }
    }

    async fn deliver_with_heartbeat(
        &self,
        claim: &super::schedule_store::ClaimedSchedule,
        runner_id: &str,
    ) -> everruns_provider::error::Result<()> {
        let schedule = &claim.schedule;
        let delivery = self
            .session_runner
            .send_message(schedule.session_id, &schedule.description);
        tokio::pin!(delivery);
        let heartbeat_interval = (self.config.claim_timeout / 3).max(Duration::from_millis(1));
        let mut heartbeat = tokio::time::interval(heartbeat_interval);
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
        heartbeat.tick().await;
        loop {
            tokio::select! {
                result = &mut delivery => return result,
                _ = heartbeat.tick() => {
                    if !self.store.renew_claim(claim, runner_id, Utc::now())? {
                        return Err(everruns_provider::error::AgentLoopError::store(format!(
                            "local schedule claim {} was lost during delivery",
                            claim.claim_id
                        )));
                    }
                }
            }
        }
    }
}

fn bounded_error(error: &str) -> String {
    const MAX_ERROR_BYTES: usize = 4_096;
    if error.len() <= MAX_ERROR_BYTES {
        return error.to_string();
    }
    let mut end = MAX_ERROR_BYTES;
    while !error.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &error[..end])
}

/// Owned lifecycle handle for a running local schedule executor.
pub struct LocalScheduleRunnerHandle {
    shutdown_tx: watch::Sender<bool>,
    join: Option<JoinHandle<()>>,
}

impl LocalScheduleRunnerHandle {
    /// Stop claiming new schedules and wait for the current delivery to finish.
    pub async fn shutdown(mut self) -> LocalResult<()> {
        let _ = self.shutdown_tx.send(true);
        if let Some(join) = self.join.take() {
            join.await.map_err(|error| {
                LocalError::Other(format!("local schedule runner task failed: {error}"))
            })?;
        }
        Ok(())
    }

    /// Stop immediately. Unfinished claims become eligible after `claim_timeout`.
    pub fn abort(mut self) {
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}

impl Drop for LocalScheduleRunnerHandle {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
    }
}

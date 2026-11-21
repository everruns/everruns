//! The session services an activation on a cell gets (data plane v2 step
//! 11.4): the schedule store the `session_schedule` tools use. Every write
//! is a [`Command::Platform`] on the session's cell, so it is replicated
//! with the session and applied there with its limits (`platform.rs`).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime::session_schedule::{
    BackgroundLimits, ScheduleBounds, ScheduleLimitError, SessionSchedule,
    resolve_agent_schedule_bounds, validate_cron_min_interval,
};
use everruns_contracts::runtime::session_services::SessionScheduleStore;
use everruns_contracts::typed_id::{EventId, PrincipalId, ScheduleId, SessionId};
use everruns_core::host::HostBackends;

use crate::hosting::Port;
use crate::platform::{Op, ScheduleChange, next_cron};
use crate::state::Command;

/// Give an activation's runtime the cell's session services.
pub(crate) fn attach(backends: HostBackends, port: Port) -> HostBackends {
    let schedules: Arc<dyn SessionScheduleStore> = Arc::new(CellSchedules { port });
    backends.with_schedule_store_factory(Arc::new(move |_org| schedules.clone()))
}

/// Schedules in the session's cell state.
pub(crate) struct CellSchedules {
    port: Port,
}

/// A new schedule, as the local host builds it: recurring from its next cron
/// time, one-shot at its time.
pub fn build(
    session: SessionId,
    description: String,
    cron_expression: Option<String>,
    scheduled_at: Option<DateTime<Utc>>,
    timezone: String,
    now: DateTime<Utc>,
) -> std::result::Result<SessionSchedule, String> {
    if cron_expression.is_none() && scheduled_at.is_none() {
        return Err("a schedule needs a cron_expression or a scheduled_at time".to_owned());
    }
    let mut schedule = SessionSchedule {
        id: ScheduleId::new(),
        session_id: session,
        // The store does not learn who asked; as on the local host.
        owner_principal_id: PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        owner: None,
        effective_owner: None,
        description,
        schedule_type: SessionSchedule::derive_type(&cron_expression),
        cron_expression,
        scheduled_at,
        timezone,
        enabled: true,
        next_trigger_at: scheduled_at,
        last_triggered_at: None,
        trigger_count: 0,
        created_at: now,
        updated_at: now,
        deleted_at: None,
        ends_at: None,
        max_runs: None,
        stop_when: None,
        stop_reason: None,
        budget: None,
    };
    if schedule.cron_expression.is_some() {
        schedule.next_trigger_at = Some(next_cron(&schedule, now)?);
    }
    Ok(schedule)
}

impl CellSchedules {
    async fn propose(&self, session: SessionId, op: Op) -> Result<serde_json::Value> {
        self.port
            .propose(
                session,
                Command::Platform {
                    session,
                    action: op,
                    now_ms: crate::node::now_ms(),
                    seed: EventId::new(),
                },
            )
            .await
    }

    async fn create(&self, schedule: SessionSchedule) -> Result<SessionSchedule> {
        let session = schedule.session_id;
        let value = self
            .propose(
                session,
                Op::CreateSchedule {
                    schedule: Box::new(schedule),
                },
            )
            .await?;
        serde_json::from_value(value).map_err(|e| AgentLoopError::store(e.to_string()))
    }
}

#[async_trait]
impl SessionScheduleStore for CellSchedules {
    async fn create_schedule(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<DateTime<Utc>>,
        timezone: String,
    ) -> Result<SessionSchedule> {
        let schedule = build(
            session_id,
            description,
            cron_expression,
            scheduled_at,
            timezone,
            Utc::now(),
        )
        .map_err(AgentLoopError::config)?;
        self.create(schedule).await
    }

    async fn create_schedule_enforcing_limits(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<DateTime<Utc>>,
        timezone: String,
        bounds: ScheduleBounds,
    ) -> std::result::Result<SessionSchedule, ScheduleLimitError> {
        if let Some(cron) = cron_expression.as_deref() {
            validate_cron_min_interval(cron).map_err(ScheduleLimitError::Rejected)?;
        }
        let now = Utc::now();
        let bounds = resolve_agent_schedule_bounds(
            &bounds,
            &BackgroundLimits::default(),
            cron_expression.is_some(),
            now,
        )
        .map_err(ScheduleLimitError::Rejected)?;
        let mut schedule = build(
            session_id,
            description,
            cron_expression,
            scheduled_at,
            timezone,
            now,
        )
        .map_err(ScheduleLimitError::Rejected)?;
        schedule.ends_at = bounds.ends_at;
        schedule.max_runs = bounds.max_runs;
        schedule.stop_when = bounds.stop_when;
        self.create(schedule).await.map_err(|error| match error {
            AgentLoopError::Configuration(reason) => ScheduleLimitError::Rejected(reason),
            error => ScheduleLimitError::Store(error),
        })
    }

    async fn cancel_schedule(
        &self,
        session_id: SessionId,
        schedule_id: ScheduleId,
    ) -> Result<SessionSchedule> {
        let value = self
            .propose(
                session_id,
                Op::UpdateSchedule {
                    id: schedule_id,
                    change: ScheduleChange {
                        enabled: Some(false),
                        ..ScheduleChange::default()
                    },
                },
            )
            .await
            .map_err(|error| match error {
                AgentLoopError::Configuration(reason) => AgentLoopError::tool(reason),
                error => error,
            })?;
        serde_json::from_value(value).map_err(|e| AgentLoopError::store(e.to_string()))
    }

    async fn list_schedules(&self, session_id: SessionId) -> Result<Vec<SessionSchedule>> {
        self.port
            .read(session_id, |s| {
                s.platform
                    .schedules
                    .values()
                    .filter(|schedule| schedule.deleted_at.is_none())
                    .cloned()
                    .collect()
            })
            .await
    }

    async fn count_active_schedules(&self, session_id: SessionId) -> Result<u32> {
        self.port
            .read(session_id, |s| {
                s.platform
                    .schedules
                    .values()
                    .filter(|schedule| schedule.enabled && schedule.deleted_at.is_none())
                    .count() as u32
            })
            .await
    }

    /// An org's sessions spread over cells, so a cell cannot count them;
    /// the per-session cap still holds (see `platform.rs`).
    async fn count_active_org_schedules(&self) -> Result<u32> {
        Ok(0)
    }
}

// Database-backed SessionScheduleStore implementation
//
// Adapts the storage backend to the core SessionScheduleStore trait,
// allowing scheduling tools to create/cancel/list schedules.

use crate::kernel_imports::{
    everruns_provider::error::AgentLoopError,
    everruns_provider::error::Result,
    everruns_provider::error::StoreResultExt,
    everruns_provider::typed_id::ScheduleId,
    everruns_provider::typed_id::SessionId,
    session_schedule::{
        DEFAULT_MAX_SCHEDULES_PER_ORG, DEFAULT_MIN_INTERVAL_SECONDS,
        MAX_ACTIVE_SCHEDULES_PER_SESSION, ScheduleLimitError, SessionSchedule,
        validate_cron_min_interval_with, validate_schedule_create_limits,
    },
    session_services::SessionScheduleStore,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::backend::StorageBackend;
use super::models::{CreateSessionScheduleRow, SessionScheduleRow, UpdateSessionScheduleRow};
use crate::domains::session_schedules::compute_next_trigger;

use std::sync::Arc;

fn schedule_limits() -> (i64, i64) {
    let min_interval = std::env::var("SESSION_SCHEDULE_MIN_INTERVAL_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MIN_INTERVAL_SECONDS);
    let max_per_org = std::env::var("RESOURCE_LIMIT_MAX_SESSION_SCHEDULES_PER_ORG")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_SCHEDULES_PER_ORG);
    (min_interval, max_per_org)
}

/// Convert a storage row to the domain type.
pub fn row_to_domain(row: &SessionScheduleRow) -> SessionSchedule {
    SessionSchedule {
        id: row.id,
        session_id: row.session_id,
        owner_principal_id: row.owner_principal_id,
        resolved_owner_user_id: row.resolved_owner_user_id,
        owner: None,
        effective_owner: None,
        description: row.description.clone(),
        schedule_type: SessionSchedule::derive_type(&row.cron_expression),
        cron_expression: row.cron_expression.clone(),
        scheduled_at: row.scheduled_at,
        timezone: row.timezone.clone(),
        enabled: row.enabled,
        next_trigger_at: row.next_trigger_at,
        last_triggered_at: row.last_triggered_at,
        trigger_count: row.trigger_count as u32,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Database-backed session schedule store.
///
/// Used by scheduling tools (create_schedule, cancel_schedule, list_schedules).
/// Requires org_id for multitenancy — callers must provide the correct org_id.
#[derive(Clone)]
pub struct DbSessionScheduleStore {
    db: Arc<StorageBackend>,
    org_id: i64,
}

impl DbSessionScheduleStore {
    pub fn new(db: Arc<StorageBackend>, org_id: i64) -> Self {
        Self { db, org_id }
    }
}

#[async_trait]
impl SessionScheduleStore for DbSessionScheduleStore {
    async fn create_schedule(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<DateTime<Utc>>,
        timezone: String,
    ) -> Result<SessionSchedule> {
        let next_trigger =
            compute_next_trigger(cron_expression.as_deref(), scheduled_at, &timezone)
                .map_err(|e| AgentLoopError::tool(e.to_string()))?;
        let session = self
            .db
            .get_session(self.org_id, session_id)
            .await
            .store_err()?
            .ok_or_else(|| AgentLoopError::tool("Session not found".to_string()))?;

        let row = self
            .db
            .create_session_schedule(CreateSessionScheduleRow {
                org_id: self.org_id,
                session_id,
                owner_principal_id: session.owner_principal_id,
                resolved_owner_user_id: session.resolved_owner_user_id,
                description,
                cron_expression,
                scheduled_at,
                timezone,
                next_trigger_at: next_trigger,
            })
            .await
            .store_err()?;

        Ok(row_to_domain(&row))
    }

    async fn create_schedule_enforcing_limits(
        &self,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<DateTime<Utc>>,
        timezone: String,
    ) -> std::result::Result<SessionSchedule, ScheduleLimitError> {
        if let Some(cron) = cron_expression.as_deref() {
            validate_cron_min_interval_with(cron, schedule_limits().0)
                .map_err(ScheduleLimitError::Rejected)?;
        }

        let next_trigger =
            compute_next_trigger(cron_expression.as_deref(), scheduled_at, &timezone)
                .map_err(|e| ScheduleLimitError::Store(AgentLoopError::tool(e.to_string())))?;
        let session = self
            .db
            .get_session(self.org_id, session_id)
            .await
            .store_err()
            .map_err(ScheduleLimitError::Store)?
            .ok_or_else(|| ScheduleLimitError::Store(AgentLoopError::tool("Session not found")))?;

        let max_per_org = schedule_limits().1;
        let row = self
            .db
            .create_session_schedule_with_limits(
                CreateSessionScheduleRow {
                    org_id: self.org_id,
                    session_id,
                    owner_principal_id: session.owner_principal_id,
                    resolved_owner_user_id: session.resolved_owner_user_id,
                    description,
                    cron_expression,
                    scheduled_at,
                    timezone,
                    next_trigger_at: next_trigger,
                },
                MAX_ACTIVE_SCHEDULES_PER_SESSION,
                max_per_org,
            )
            .await
            .store_err()
            .map_err(ScheduleLimitError::Store)?;

        match row {
            Some(row) => Ok(row_to_domain(&row)),
            None => validate_schedule_create_limits(self, session_id, None).await.and_then(|()| {
                Err(ScheduleLimitError::Rejected(format!(
                    "Maximum {max_per_org} active schedules per org reached. Cancel an existing schedule first."
                )))
            }),
        }
    }

    async fn cancel_schedule(
        &self,
        _session_id: SessionId,
        schedule_id: ScheduleId,
    ) -> Result<SessionSchedule> {
        let row = self
            .db
            .update_session_schedule(
                self.org_id,
                schedule_id,
                UpdateSessionScheduleRow {
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .await
            .store_err()?
            .ok_or_else(|| AgentLoopError::tool("Schedule not found".to_string()))?;

        Ok(row_to_domain(&row))
    }

    async fn list_schedules(&self, session_id: SessionId) -> Result<Vec<SessionSchedule>> {
        let rows = self
            .db
            .list_session_schedules(self.org_id, session_id)
            .await
            .store_err()?;

        Ok(rows.iter().map(row_to_domain).collect())
    }

    async fn count_active_schedules(&self, session_id: SessionId) -> Result<u32> {
        self.db
            .count_active_session_schedules(session_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))
    }

    async fn count_active_org_schedules(&self) -> Result<u32> {
        self.db
            .count_active_org_session_schedules(self.org_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))
    }
}

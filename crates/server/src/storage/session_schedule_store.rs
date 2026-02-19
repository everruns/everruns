// Database-backed SessionScheduleStore implementation
//
// Adapts the storage backend to the core SessionScheduleStore trait,
// allowing scheduling tools to create/cancel/list schedules.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_core::{
    AgentLoopError, Result, ScheduleId, SessionId, session_schedule::SessionSchedule,
    traits::SessionScheduleStore,
};

use super::backend::StorageBackend;
use super::models::{CreateSessionScheduleRow, SessionScheduleRow, UpdateSessionScheduleRow};
use crate::services::session_schedule::compute_next_trigger;

use std::sync::Arc;

/// Convert a storage row to the domain type.
pub fn row_to_domain(row: &SessionScheduleRow) -> SessionSchedule {
    SessionSchedule {
        id: row.id,
        session_id: row.session_id,
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
/// Requires org_id for multitenancy — uses DEFAULT_ORG_ID for now (worker context).
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

        let row = self
            .db
            .create_session_schedule(CreateSessionScheduleRow {
                org_id: self.org_id,
                session_id,
                description,
                cron_expression,
                scheduled_at,
                timezone,
                next_trigger_at: next_trigger,
            })
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        Ok(row_to_domain(&row))
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
            .map_err(|e| AgentLoopError::store(e.to_string()))?
            .ok_or_else(|| AgentLoopError::tool("Schedule not found".to_string()))?;

        Ok(row_to_domain(&row))
    }

    async fn list_schedules(&self, session_id: SessionId) -> Result<Vec<SessionSchedule>> {
        let rows = self
            .db
            .list_session_schedules(self.org_id, session_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        Ok(rows.iter().map(row_to_domain).collect())
    }

    async fn count_active_schedules(&self, session_id: SessionId) -> Result<u32> {
        self.db
            .count_active_session_schedules(session_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))
    }
}

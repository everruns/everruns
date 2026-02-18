// Session schedule service — business logic for session-scoped schedules.
//
// Handles cron parsing, next-trigger computation, and schedule lifecycle.

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use everruns_core::{
    ScheduleId, SessionId,
    session_schedule::{MAX_ACTIVE_SCHEDULES_PER_SESSION, SessionSchedule},
};
use std::sync::Arc;

use crate::storage::backend::StorageBackend;
use crate::storage::models::{CreateSessionScheduleRow, UpdateSessionScheduleRow};
use crate::storage::session_schedule_store::row_to_domain;

/// Compute the next trigger time for a schedule.
///
/// For cron: parses the expression and finds the next occurrence after now.
/// For one-shot: returns the scheduled_at time if it's in the future.
pub fn compute_next_trigger(
    cron_expression: Option<&str>,
    scheduled_at: Option<DateTime<Utc>>,
    _timezone: &str,
) -> Result<Option<DateTime<Utc>>> {
    if let Some(cron_expr) = cron_expression {
        let schedule = cron::Schedule::from_str(cron_expr)
            .map_err(|e| anyhow!("Invalid cron expression '{}': {}", cron_expr, e))?;

        let next = schedule.upcoming(Utc).next();
        Ok(next)
    } else if let Some(at) = scheduled_at {
        if at > Utc::now() {
            Ok(Some(at))
        } else {
            Ok(None) // Already past — will not trigger
        }
    } else {
        Ok(None)
    }
}

use std::str::FromStr;

/// Service for managing session schedules.
pub struct SessionScheduleService {
    db: Arc<StorageBackend>,
}

impl SessionScheduleService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    pub async fn create(
        &self,
        org_id: i64,
        session_id: SessionId,
        description: String,
        cron_expression: Option<String>,
        scheduled_at: Option<DateTime<Utc>>,
        timezone: String,
    ) -> Result<SessionSchedule> {
        // Validate max active
        let active = self.db.count_active_session_schedules(session_id).await?;
        if active >= MAX_ACTIVE_SCHEDULES_PER_SESSION {
            return Err(anyhow!(
                "Maximum {} active schedules per session",
                MAX_ACTIVE_SCHEDULES_PER_SESSION
            ));
        }

        let next_trigger =
            compute_next_trigger(cron_expression.as_deref(), scheduled_at, &timezone)
                .context("Failed to compute next trigger")?;

        let row = self
            .db
            .create_session_schedule(CreateSessionScheduleRow {
                org_id,
                session_id,
                description,
                cron_expression,
                scheduled_at,
                timezone,
                next_trigger_at: next_trigger,
            })
            .await?;

        Ok(row_to_domain(&row))
    }

    pub async fn get(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<Option<SessionSchedule>> {
        let row = self.db.get_session_schedule(org_id, schedule_id).await?;
        Ok(row.as_ref().map(row_to_domain))
    }

    pub async fn list(&self, org_id: i64, session_id: SessionId) -> Result<Vec<SessionSchedule>> {
        let rows = self.db.list_session_schedules(org_id, session_id).await?;
        Ok(rows.iter().map(row_to_domain).collect())
    }

    pub async fn update_enabled(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
        enabled: bool,
    ) -> Result<Option<SessionSchedule>> {
        // If re-enabling, recompute next trigger
        let mut input = UpdateSessionScheduleRow {
            enabled: Some(enabled),
            ..Default::default()
        };

        if enabled && let Some(row) = self.db.get_session_schedule(org_id, schedule_id).await? {
            let next = compute_next_trigger(
                row.cron_expression.as_deref(),
                row.scheduled_at,
                &row.timezone,
            )?;
            input.next_trigger_at = Some(next);
        }

        let row = self
            .db
            .update_session_schedule(org_id, schedule_id, input)
            .await?;
        Ok(row.as_ref().map(row_to_domain))
    }

    pub async fn delete(&self, org_id: i64, schedule_id: ScheduleId) -> Result<bool> {
        self.db.delete_session_schedule(org_id, schedule_id).await
    }

    /// Mark a schedule as triggered and compute next run.
    /// For one-shot: disables after trigger.
    /// For recurring: computes next trigger from cron.
    pub async fn mark_triggered(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<Option<SessionSchedule>> {
        let row = self.db.get_session_schedule(org_id, schedule_id).await?;
        let Some(row) = row else { return Ok(None) };

        let is_recurring = row.cron_expression.is_some();
        let next_trigger = if is_recurring {
            compute_next_trigger(row.cron_expression.as_deref(), None, &row.timezone)?
        } else {
            None // One-shot: no next trigger
        };

        let input = UpdateSessionScheduleRow {
            enabled: if is_recurring { None } else { Some(false) },
            next_trigger_at: Some(next_trigger),
            last_triggered_at: Some(Utc::now()),
            trigger_count_increment: true,
        };

        let updated = self
            .db
            .update_session_schedule(org_id, schedule_id, input)
            .await?;
        Ok(updated.as_ref().map(row_to_domain))
    }
}

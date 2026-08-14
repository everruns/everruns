// Session schedule service — business logic for session-scoped schedules.
//
// Handles cron parsing, next-trigger computation, and schedule lifecycle.

use crate::kernel_imports::{
    everruns_provider::typed_id::ScheduleId,
    everruns_provider::typed_id::SessionId,
    session_schedule::{MAX_ACTIVE_SCHEDULES_PER_SESSION, SessionSchedule},
};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use everruns_durable::UpdateField;
use std::sync::Arc;

use crate::services::PrincipalService;
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
    principal_service: PrincipalService,
}

impl SessionScheduleService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            principal_service: PrincipalService::new(db.clone()),
            db,
        }
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
        let session = self
            .db
            .get_session(org_id, session_id)
            .await?
            .ok_or_else(|| anyhow!("Session not found"))?;

        let row = self
            .db
            .create_session_schedule(CreateSessionScheduleRow {
                org_id,
                session_id,
                owner_principal_id: session.owner_principal_id,
                resolved_owner_user_id: session.resolved_owner_user_id,
                description,
                cron_expression,
                scheduled_at,
                timezone,
                next_trigger_at: next_trigger,
            })
            .await?;

        let mut schedule = row_to_domain(&row);
        self.hydrate_ownership(org_id, &mut schedule).await?;
        Ok(schedule)
    }

    pub async fn get(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<Option<SessionSchedule>> {
        let row = self.db.get_session_schedule(org_id, schedule_id).await?;
        match row {
            Some(row) => {
                let mut schedule = row_to_domain(&row);
                self.hydrate_ownership(org_id, &mut schedule).await?;
                Ok(Some(schedule))
            }
            None => Ok(None),
        }
    }

    pub async fn list(&self, org_id: i64, session_id: SessionId) -> Result<Vec<SessionSchedule>> {
        let rows = self.db.list_session_schedules(org_id, session_id).await?;
        let mut schedules = Vec::with_capacity(rows.len());
        for row in rows {
            let mut schedule = row_to_domain(&row);
            self.hydrate_ownership(org_id, &mut schedule).await?;
            schedules.push(schedule);
        }
        Ok(schedules)
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
            input.next_trigger_at = UpdateField::from_option(next);
        }

        let row = self
            .db
            .update_session_schedule(org_id, schedule_id, input)
            .await?;
        match row {
            Some(row) => {
                let mut schedule = row_to_domain(&row);
                self.hydrate_ownership(org_id, &mut schedule).await?;
                Ok(Some(schedule))
            }
            None => Ok(None),
        }
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
            next_trigger_at: UpdateField::from_option(next_trigger),
            last_triggered_at: Some(Utc::now()),
            trigger_count_increment: true,
        };

        let updated = self
            .db
            .update_session_schedule(org_id, schedule_id, input)
            .await?;
        match updated {
            Some(row) => {
                let mut schedule = row_to_domain(&row);
                self.hydrate_ownership(org_id, &mut schedule).await?;
                Ok(Some(schedule))
            }
            None => Ok(None),
        }
    }

    async fn hydrate_ownership(&self, org_id: i64, schedule: &mut SessionSchedule) -> Result<()> {
        schedule.owner = self
            .principal_service
            .get_summary(org_id, schedule.owner_principal_id)
            .await?;
        schedule.effective_owner = self
            .principal_service
            .effective_owner_summary(org_id, schedule.resolved_owner_user_id)
            .await?;
        Ok(())
    }
}

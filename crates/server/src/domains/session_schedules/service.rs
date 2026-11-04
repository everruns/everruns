// Session schedule service — business logic for session-scoped schedules.
//
// Handles cron parsing, next-trigger computation, and schedule lifecycle.

use crate::kernel_imports::{
    contracts::typed_id::ScheduleId,
    contracts::typed_id::SessionId,
    session_schedule::{
        MAX_ACTIVE_SCHEDULES_PER_SESSION, ScheduleBudget, SessionSchedule, parse_session_cron,
        stop_reason,
    },
};
use crate::storage::UpdateField;
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use std::sync::Arc;

use crate::domains::users::PrincipalService;
use crate::storage::backend::StorageBackend;
use crate::storage::runtime::session_schedule::row_to_domain;
use crate::storage::{CreateSessionScheduleRow, UpdateSessionScheduleRow};

/// Compute the next trigger time for a schedule.
///
/// For cron: accepts the standard 5-field form the `create_schedule` tool
/// documents, read through `parse_session_cron` (shared with the interval
/// gate and the facade's local schedule store, so hosted and embedded
/// schedules agree on what a cron means), evaluates it in the schedule's IANA
/// timezone, and finds the next occurrence after now. Every error is a caller
/// input error: an invalid cron or timezone.
/// For one-shot: returns the scheduled_at time if it's in the future.
pub fn compute_next_trigger(
    cron_expression: Option<&str>,
    scheduled_at: Option<DateTime<Utc>>,
    timezone: &str,
) -> Result<Option<DateTime<Utc>>> {
    if let Some(cron_expr) = cron_expression {
        next_cron_occurrence(cron_expr, timezone, Utc::now())
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

fn next_cron_occurrence(
    cron_expr: &str,
    timezone: &str,
    after: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>> {
    let schedule = parse_session_cron(cron_expr).map_err(|error| anyhow!(error))?;
    let tz = if timezone.trim().is_empty() {
        chrono_tz::UTC
    } else {
        chrono_tz::Tz::from_str(timezone)
            .map_err(|e| anyhow!("Invalid IANA timezone '{timezone}': {e}"))?
    };
    // The ambiguous wall-time iterator may first return the other offset's
    // already-past UTC instant. A recurring trigger must strictly advance.
    Ok(schedule
        .after(&after.with_timezone(&tz))
        .find(|next| next.with_timezone(&Utc) > after)
        .map(|next| next.with_timezone(&Utc)))
}

use std::str::FromStr;

/// What a person changes on a schedule. `Some(None)` clears an end.
#[derive(Debug, Clone, Default)]
pub struct ScheduleChange {
    pub enabled: Option<bool>,
    pub ends_at: Option<Option<DateTime<Utc>>>,
    pub max_runs: Option<Option<u32>>,
    pub budget_usd: Option<f64>,
}

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
                ends_at: None,
                max_runs: None,
                stop_when: None,
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
            Some(row) => Ok(Some(self.to_domain(org_id, &row).await?)),
            None => Ok(None),
        }
    }

    /// Schedules for the session view, including ones that were deleted.
    /// The agent's own list omits deleted rows.
    pub async fn list(&self, org_id: i64, session_id: SessionId) -> Result<Vec<SessionSchedule>> {
        let rows = self
            .db
            .list_session_schedule_history(org_id, session_id)
            .await?;
        let mut schedules = Vec::with_capacity(rows.len());
        for row in rows {
            let mut schedule = row_to_domain(&row);
            self.hydrate_ownership(org_id, &mut schedule).await?;
            schedules.push(schedule);
        }
        self.hydrate_budgets(org_id, &mut schedules).await?;
        Ok(schedules)
    }

    /// A person's change to a schedule: pause or resume it, and extend its
    /// end, run count or budget. Unlike an agent, a person is not held to the
    /// org's agent limits. Resuming clears why it stopped. Errors in
    /// `Err(Ok(message))` are the caller's input; `Err(Err(_))` is a failure.
    pub async fn update(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
        change: ScheduleChange,
    ) -> std::result::Result<Option<SessionSchedule>, std::result::Result<String, anyhow::Error>>
    {
        let Some(row) = self
            .db
            .get_session_schedule(org_id, schedule_id)
            .await
            .map_err(Err)?
        else {
            return Ok(None);
        };
        let now = Utc::now();
        if change.ends_at.flatten().is_some_and(|at| at <= now) {
            return Err(Ok("ends_at must be in the future".to_string()));
        }
        if change.max_runs.flatten().is_some_and(|runs| runs < 1) {
            return Err(Ok("max_runs must be at least 1".to_string()));
        }
        if let Some(limit) = change.budget_usd
            && (!limit.is_finite() || limit <= 0.0)
        {
            return Err(Ok("budget_usd must be a positive amount in USD".to_string()));
        }

        let mut input = UpdateSessionScheduleRow {
            enabled: change.enabled,
            ends_at: change
                .ends_at
                .map_or(UpdateField::Unchanged, UpdateField::from_option),
            max_runs: change.max_runs.map_or(UpdateField::Unchanged, |runs| {
                UpdateField::from_option(runs.map(|runs| runs.min(i32::MAX as u32) as i32))
            }),
            ..Default::default()
        };
        if change.enabled == Some(true) {
            // A person resuming a schedule starts its repeated-outcome count
            // over, so a runaway stop does not trip again on the next run.
            self.db
                .reset_schedule_outcomes(schedule_id.uuid())
                .await
                .map_err(Err)?;
            input.stop_reason = UpdateField::Clear;
            input.next_trigger_at = UpdateField::from_option(
                compute_next_trigger(
                    row.cron_expression.as_deref(),
                    row.scheduled_at,
                    &row.timezone,
                )
                .map_err(Err)?,
            );
        }
        if let Some(limit) = change.budget_usd {
            self.db
                .set_schedule_budget_limit(org_id, &row.public_id, limit)
                .await
                .map_err(Err)?;
        }
        let updated = self
            .db
            .update_session_schedule(org_id, schedule_id, input)
            .await
            .map_err(Err)?;
        match updated {
            Some(row) => Ok(Some(self.to_domain(org_id, &row).await.map_err(Err)?)),
            None => Ok(None),
        }
    }

    /// Stop a schedule on its own: it reached its end or its budget.
    pub async fn stop(&self, org_id: i64, schedule_id: ScheduleId, reason: &str) -> Result<()> {
        self.db
            .update_session_schedule(
                org_id,
                schedule_id,
                UpdateSessionScheduleRow {
                    enabled: Some(false),
                    next_trigger_at: UpdateField::Clear,
                    stop_reason: UpdateField::Set(reason.to_string()),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }

    /// Skip this fire without counting it, because the org's unattended spend
    /// for today is used up. A recurring schedule moves to its next time; a
    /// one-shot waits for the daily cap to reset at midnight UTC.
    pub async fn defer(&self, org_id: i64, schedule_id: ScheduleId) -> Result<()> {
        let Some(row) = self.db.get_session_schedule(org_id, schedule_id).await? else {
            return Ok(());
        };
        let next = match row.cron_expression.as_deref() {
            Some(cron) => next_cron_occurrence(cron, &row.timezone, Utc::now())?,
            None => (Utc::now() + chrono::Duration::days(1))
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .map(|midnight| midnight.and_utc()),
        };
        self.db
            .update_session_schedule(
                org_id,
                schedule_id,
                UpdateSessionScheduleRow {
                    next_trigger_at: UpdateField::from_option(next),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }

    pub async fn delete(&self, org_id: i64, schedule_id: ScheduleId) -> Result<bool> {
        self.db.delete_session_schedule(org_id, schedule_id).await
    }

    /// Mark a schedule as triggered and compute next run.
    /// For one-shot: disables after trigger.
    /// For recurring: computes next trigger from cron, and disables the
    /// schedule with `stop_reason = ended` when this was its last run (its
    /// `max_runs` reached, or its next time past `ends_at`).
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
        let runs_after = row.trigger_count + 1;
        let ended = is_recurring
            && (row.max_runs.is_some_and(|max| runs_after >= max)
                || matches!((row.ends_at, next_trigger), (Some(end), Some(next)) if next > end));

        let input = UpdateSessionScheduleRow {
            enabled: if !is_recurring || ended {
                Some(false)
            } else {
                None
            },
            next_trigger_at: if ended {
                UpdateField::Clear
            } else {
                UpdateField::from_option(next_trigger)
            },
            last_triggered_at: Some(Utc::now()),
            trigger_count_increment: true,
            stop_reason: if ended {
                UpdateField::Set(stop_reason::ENDED.to_string())
            } else {
                UpdateField::Unchanged
            },
            ..Default::default()
        };

        let updated = self
            .db
            .update_session_schedule(org_id, schedule_id, input)
            .await?;
        match updated {
            Some(row) => Ok(Some(self.to_domain(org_id, &row).await?)),
            None => Ok(None),
        }
    }

    async fn to_domain(
        &self,
        org_id: i64,
        row: &crate::storage::SessionScheduleRow,
    ) -> Result<SessionSchedule> {
        let mut schedules = vec![row_to_domain(row)];
        self.hydrate_ownership(org_id, &mut schedules[0]).await?;
        self.hydrate_budgets(org_id, &mut schedules).await?;
        Ok(schedules.remove(0))
    }

    /// Fill each schedule's budget from its `session_schedule` budget row.
    async fn hydrate_budgets(&self, org_id: i64, schedules: &mut [SessionSchedule]) -> Result<()> {
        let ids: Vec<String> = schedules.iter().map(|s| s.id.to_string()).collect();
        let budgets = self.db.schedule_budgets(org_id, &ids).await?;
        for schedule in schedules {
            let id = schedule.id.to_string();
            schedule.budget = budgets
                .iter()
                .find(|budget| budget.subject_id == id)
                .map(|budget| ScheduleBudget {
                    limit_usd: budget.limit,
                    spent_usd: (budget.limit - budget.balance).max(0.0),
                    status: budget.status.clone(),
                });
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recurring_cron_stays_in_future_during_repeated_hour() {
        for timestamp in ["2026-11-01T06:02:07Z", "2026-11-01T07:02:07Z"] {
            let after = DateTime::parse_from_rfc3339(timestamp)
                .unwrap()
                .with_timezone(&Utc);
            assert_eq!(
                next_cron_occurrence("*/1 * * * * *", "America/Chicago", after).unwrap(),
                Some(after + chrono::Duration::seconds(1))
            );
        }
    }
    use chrono::TimeZone;

    #[test]
    fn five_field_cron_is_accepted() {
        let after = Utc.with_ymd_and_hms(2026, 10, 11, 23, 0, 0).unwrap();
        let next = next_cron_occurrence("0 7 * * *", "UTC", after).unwrap();
        assert_eq!(
            next,
            Some(Utc.with_ymd_and_hms(2026, 10, 12, 7, 0, 0).unwrap())
        );
    }

    #[test]
    fn cron_is_evaluated_in_the_schedule_timezone() {
        // 07:00 in Chicago is 12:00 UTC while daylight saving time is on.
        let after = Utc.with_ymd_and_hms(2026, 10, 11, 23, 0, 0).unwrap();
        let next = next_cron_occurrence("0 7 * * *", "America/Chicago", after).unwrap();
        assert_eq!(
            next,
            Some(Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap())
        );
        // Sunday 18:00 Chicago (cron weekday 0) after the DST switch is 00:00 UTC Monday.
        let after = Utc.with_ymd_and_hms(2026, 11, 2, 0, 0, 0).unwrap();
        let next = next_cron_occurrence("0 18 * * 0", "America/Chicago", after).unwrap();
        assert_eq!(
            next,
            Some(Utc.with_ymd_and_hms(2026, 11, 9, 0, 0, 0).unwrap())
        );
    }

    #[test]
    fn five_field_weekdays_count_from_sunday() {
        // Monday 2026-10-12 09:00 UTC is the first weekday run after Sunday.
        let after = Utc.with_ymd_and_hms(2026, 10, 11, 23, 0, 0).unwrap();
        assert_eq!(
            next_cron_occurrence("0 9 * * 1-5", "UTC", after).unwrap(),
            Some(Utc.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap())
        );
    }

    #[test]
    fn six_and_seven_field_cron_still_work() {
        let after = Utc.with_ymd_and_hms(2026, 10, 11, 23, 0, 0).unwrap();
        assert!(
            next_cron_occurrence("0 0 7 * * *", "UTC", after)
                .unwrap()
                .is_some()
        );
        assert!(
            next_cron_occurrence("0 0 7 * * * *", "UTC", after)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn canonical_parser_preserves_leap_day_calendar() {
        let after = Utc.with_ymd_and_hms(2027, 3, 1, 0, 0, 0).unwrap();
        assert_eq!(
            next_cron_occurrence("0 9 29 2 *", "UTC", after).unwrap(),
            Some(Utc.with_ymd_and_hms(2028, 2, 29, 9, 0, 0).unwrap())
        );
    }

    #[test]
    fn bad_cron_and_timezone_are_rejected() {
        let after = Utc.with_ymd_and_hms(2026, 10, 11, 23, 0, 0).unwrap();
        assert!(next_cron_occurrence("every day", "UTC", after).is_err());
        assert!(next_cron_occurrence("0 7 * * *", "Mars/Olympus", after).is_err());
    }
}

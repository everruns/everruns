//! Declarative schedules: make a named schedule match a definition.
//!
//! Decision: a host that owns a recurring job (a cleanup sweep, a sync) states
//! it as a [`ScheduleSpec`] and calls [`ensure_schedule`] on every start-up,
//! instead of hand-writing find-compare-update-create for each job. The
//! schedule's unique name is the key. A drifted row (edited by hand, left by an
//! older release, disabled) is put back to the spec; a matching row is left
//! untouched so its `next_trigger_at` survives restarts. Replicas starting at
//! once race on the unique name, and the loser reports
//! [`EnsureOutcome::AlreadyExists`].
//!
//! [`DurableScheduler`](super::DurableScheduler) then fires the schedule once
//! per cluster per trigger, whatever the number of replicas.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tracing::info;
use uuid::Uuid;

use super::SchedulerError;
use crate::persistence::{
    CreateScheduleRow, Pagination, ScheduleFilter, ScheduleRow, ScheduleTargetType, Schedules,
    StoreError, UpdateSchedule,
};
use everruns_db::UpdateField;

/// When a schedule fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cadence {
    /// A 7-field cron expression (`sec min hour day month weekday year`).
    Cron(String),
    /// A fixed period, stored as `@every <secs>s`. Each trigger is one period
    /// after the previous trigger was handled.
    Every(Duration),
}

impl Cadence {
    /// The `cron_expression` column value for this cadence.
    pub fn expression(&self) -> String {
        match self {
            Self::Cron(expr) => expr.clone(),
            Self::Every(period) => format!("{EVERY_PREFIX}{}s", period.as_secs()),
        }
    }
}

/// Prefix of an interval cadence in the `cron_expression` column.
pub(super) const EVERY_PREFIX: &str = "@every ";

/// Parse an `@every <secs>s` expression. `None` when `expr` is not one.
pub(super) fn parse_every(expr: &str) -> Option<Result<Duration, SchedulerError>> {
    let rest = expr.trim().strip_prefix(EVERY_PREFIX)?;
    let secs = rest
        .trim()
        .strip_suffix('s')
        .and_then(|n| n.parse::<u64>().ok())
        .filter(|secs| *secs > 0);
    Some(secs.map(Duration::from_secs).ok_or_else(|| {
        SchedulerError::CronError(format!(
            "invalid interval '{expr}': expected '@every <seconds>s' with seconds > 0"
        ))
    }))
}

/// The definition a named schedule is kept at.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleSpec {
    /// Unique schedule name, the key [`ensure_schedule`] matches on.
    pub name: String,
    pub description: Option<String>,
    pub cadence: Cadence,
    pub timezone: String,
    pub target_type: ScheduleTargetType,
    /// Activity type (or workflow type) the schedule starts.
    pub target_name: String,
    pub target_input: Value,
    pub max_concurrent: Option<u32>,
    pub catch_up_missed: bool,
    pub max_catch_up: Option<u32>,
    pub retry_policy: Option<Value>,
}

impl ScheduleSpec {
    /// A schedule that enqueues the standalone activity `activity_type`, one at
    /// a time, in UTC, skipping missed triggers instead of catching up.
    pub fn activity(
        name: impl Into<String>,
        activity_type: impl Into<String>,
        cadence: Cadence,
        input: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: None,
            cadence,
            timezone: "UTC".to_string(),
            target_type: ScheduleTargetType::Activity,
            target_name: activity_type.into(),
            target_input: input,
            max_concurrent: Some(1),
            catch_up_missed: false,
            max_catch_up: Some(1),
            retry_policy: None,
        }
    }

    /// Set the description shown in schedule listings.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Whether `row` already matches this spec (and is enabled).
    pub fn matches(&self, row: &ScheduleRow) -> bool {
        row.description == self.description
            && row.cron_expression == self.cadence.expression()
            && row.timezone == self.timezone
            && row.target_type == self.target_type
            && row.target_name == self.target_name
            && row.target_input == self.target_input
            && row.enabled
            && row.max_concurrent == self.max_concurrent
            && row.catch_up_missed == self.catch_up_missed
            && row.max_catch_up == self.max_catch_up
            && row.retry_policy == self.retry_policy
    }
}

/// What [`ensure_schedule`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnsureOutcome {
    /// No schedule had the name; one was created.
    Created(Uuid),
    /// The schedule had drifted and was reset to the spec.
    Updated(Uuid),
    /// The schedule already matched.
    Unchanged(Uuid),
    /// Another replica created it between our lookup and our insert.
    AlreadyExists,
}

/// Find a schedule by its unique name.
pub async fn find_schedule<S: Schedules + ?Sized>(
    store: &S,
    name: &str,
) -> Result<Option<ScheduleRow>, StoreError> {
    const PAGE: u32 = 500;
    let mut offset = 0;
    loop {
        let page = store
            .list_schedules(
                ScheduleFilter::default(),
                Pagination {
                    offset,
                    limit: PAGE,
                },
            )
            .await?;
        let len = page.len() as u32;
        if let Some(found) = page.into_iter().find(|row| row.name == name) {
            return Ok(Some(found));
        }
        if len < PAGE {
            return Ok(None);
        }
        offset += PAGE;
    }
}

/// Make the schedule named `spec.name` exist, enabled, with exactly `spec`.
pub async fn ensure_schedule<S: Schedules + ?Sized>(
    store: &S,
    spec: &ScheduleSpec,
) -> Result<EnsureOutcome, SchedulerError> {
    let expression = spec.cadence.expression();
    if let Some(existing) = find_schedule(store, &spec.name).await? {
        if spec.matches(&existing) {
            info!(schedule = %spec.name, schedule_id = %existing.id, "Durable schedule already configured");
            return Ok(EnsureOutcome::Unchanged(existing.id));
        }
        store
            .update_schedule(
                existing.id,
                UpdateSchedule {
                    description: spec
                        .description
                        .clone()
                        .map_or(UpdateField::Clear, UpdateField::Set),
                    cron_expression: Some(expression.clone()),
                    timezone: Some(spec.timezone.clone()),
                    target_type: Some(spec.target_type),
                    target_name: Some(spec.target_name.clone()),
                    target_input: Some(spec.target_input.clone()),
                    enabled: Some(true),
                    max_concurrent: spec
                        .max_concurrent
                        .map_or(UpdateField::Clear, UpdateField::Set),
                    catch_up_missed: Some(spec.catch_up_missed),
                    max_catch_up: spec
                        .max_catch_up
                        .map_or(UpdateField::Clear, UpdateField::Set),
                    retry_policy: spec
                        .retry_policy
                        .clone()
                        .map_or(UpdateField::Clear, UpdateField::Set),
                    next_trigger_at: UpdateField::Set(next_after(&expression, Utc::now())?),
                    ..Default::default()
                },
            )
            .await?;
        info!(schedule = %spec.name, schedule_id = %existing.id, "Updated durable schedule");
        return Ok(EnsureOutcome::Updated(existing.id));
    }

    match store
        .create_schedule(CreateScheduleRow {
            name: spec.name.clone(),
            description: spec.description.clone(),
            cron_expression: expression.clone(),
            timezone: spec.timezone.clone(),
            target_type: spec.target_type,
            target_name: spec.target_name.clone(),
            target_input: spec.target_input.clone(),
            enabled: true,
            max_concurrent: spec.max_concurrent,
            catch_up_missed: spec.catch_up_missed,
            max_catch_up: spec.max_catch_up,
            retry_policy: spec.retry_policy.clone(),
            next_trigger_at: Some(next_after(&expression, Utc::now())?),
        })
        .await
    {
        Ok(id) => {
            info!(schedule = %spec.name, schedule_id = %id, "Created durable schedule");
            Ok(EnsureOutcome::Created(id))
        }
        Err(StoreError::Database(error)) if is_unique_violation(&error) => {
            info!(schedule = %spec.name, "Durable schedule already exists");
            Ok(EnsureOutcome::AlreadyExists)
        }
        Err(error) => Err(error.into()),
    }
}

/// Disable the schedule named `name`, if there is one and it is enabled.
/// Returns whether a schedule was disabled.
///
/// A host calls this for a job switched off by configuration, so a schedule
/// created while it was on stops firing.
pub async fn disable_schedule<S: Schedules + ?Sized>(
    store: &S,
    name: &str,
) -> Result<bool, StoreError> {
    match find_schedule(store, name).await? {
        Some(row) if row.enabled => {
            store
                .update_schedule(
                    row.id,
                    UpdateSchedule {
                        enabled: Some(false),
                        ..Default::default()
                    },
                )
                .await?;
            info!(schedule = %name, schedule_id = %row.id, "Disabled durable schedule");
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn is_unique_violation(error: &str) -> bool {
    error.contains("duplicate") || error.contains("unique")
}

/// When a schedule with `expression` next fires after `now`.
pub(super) fn next_after(
    expression: &str,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, SchedulerError> {
    if let Some(period) = parse_every(expression) {
        let period = chrono::Duration::from_std(period?)
            .map_err(|e| SchedulerError::CronError(e.to_string()))?;
        return Ok(now + period);
    }
    use std::str::FromStr;
    cron::Schedule::from_str(expression)
        .map_err(|e| SchedulerError::CronError(e.to_string()))?
        .after(&now)
        .next()
        .ok_or_else(|| SchedulerError::CronError("no upcoming occurrence".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::InMemoryWorkflowEventStore;
    use serde_json::json;

    fn spec() -> ScheduleSpec {
        ScheduleSpec::activity(
            "nightly-sweep",
            "sweep",
            Cadence::Every(Duration::from_secs(90)),
            json!({ "batch": 10 }),
        )
        .with_description("Sweeps things")
    }

    async fn named(store: &InMemoryWorkflowEventStore, name: &str) -> Vec<ScheduleRow> {
        store
            .list_schedules(
                ScheduleFilter::default(),
                Pagination {
                    offset: 0,
                    limit: 100,
                },
            )
            .await
            .unwrap()
            .into_iter()
            .filter(|row| row.name == name)
            .collect()
    }

    #[test]
    fn every_round_trips_and_rejects_garbage() {
        let every = Cadence::Every(Duration::from_secs(3600)).expression();
        assert_eq!(every, "@every 3600s");
        assert_eq!(
            parse_every(&every).unwrap().unwrap(),
            Duration::from_secs(3600)
        );
        assert!(
            parse_every("0 * * * * * *").is_none(),
            "cron is not an interval"
        );
        assert!(parse_every("@every 0s").unwrap().is_err());
        assert!(parse_every("@every soon").unwrap().is_err());
    }

    #[test]
    fn next_after_handles_both_cadences() {
        let now = Utc::now();
        assert_eq!(
            next_after("@every 90s", now).unwrap(),
            now + chrono::Duration::seconds(90)
        );
        let cron_next = next_after("0 * * * * * *", now).unwrap();
        assert!(cron_next > now && cron_next <= now + chrono::Duration::seconds(60));
        assert!(next_after("not cron", now).is_err());
    }

    #[tokio::test]
    async fn creates_once_then_leaves_a_matching_schedule_alone() {
        let store = InMemoryWorkflowEventStore::new();
        let created = ensure_schedule(&store, &spec()).await.unwrap();
        let EnsureOutcome::Created(id) = created else {
            panic!("expected create, got {created:?}");
        };
        let first = store.get_schedule(id).await.unwrap();

        assert_eq!(
            ensure_schedule(&store, &spec()).await.unwrap(),
            EnsureOutcome::Unchanged(id)
        );
        let rows = named(&store, "nightly-sweep").await;
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert!(spec().matches(row));
        assert_eq!(row.cron_expression, "@every 90s");
        assert_eq!(row.max_concurrent, Some(1));
        assert!(!row.catch_up_missed);
        assert_eq!(
            row.next_trigger_at, first.next_trigger_at,
            "an unchanged schedule keeps its next trigger"
        );
    }

    #[tokio::test]
    async fn resets_a_drifted_schedule() {
        let store = InMemoryWorkflowEventStore::new();
        store
            .create_schedule(CreateScheduleRow {
                name: "nightly-sweep".into(),
                description: Some("outdated".into()),
                cron_expression: "*/5 * * * *".into(),
                timezone: "America/Chicago".into(),
                target_type: ScheduleTargetType::Workflow,
                target_name: "wrong".into(),
                target_input: json!({ "batch": 1 }),
                enabled: false,
                max_concurrent: Some(3),
                catch_up_missed: true,
                max_catch_up: Some(9),
                retry_policy: Some(json!({})),
                next_trigger_at: None,
            })
            .await
            .unwrap();

        let outcome = ensure_schedule(&store, &spec()).await.unwrap();
        assert!(matches!(outcome, EnsureOutcome::Updated(_)));
        let rows = named(&store, "nightly-sweep").await;
        assert_eq!(rows.len(), 1);
        assert!(spec().matches(&rows[0]), "row: {:?}", rows[0]);
        assert!(rows[0].next_trigger_at.is_some());
    }

    #[tokio::test]
    async fn a_concurrent_create_is_not_an_error() {
        let store = InMemoryWorkflowEventStore::new();
        let other = spec();
        store
            .create_schedule(CreateScheduleRow {
                name: other.name.clone(),
                description: None,
                cron_expression: other.cadence.expression(),
                timezone: other.timezone.clone(),
                target_type: other.target_type,
                target_name: other.target_name.clone(),
                target_input: other.target_input.clone(),
                enabled: true,
                max_concurrent: None,
                catch_up_missed: false,
                max_catch_up: None,
                retry_policy: None,
                next_trigger_at: None,
            })
            .await
            .unwrap();
        // The store refuses a second row under the same name.
        let duplicate = store
            .create_schedule(CreateScheduleRow {
                name: other.name.clone(),
                description: None,
                cron_expression: "@every 1s".into(),
                timezone: "UTC".into(),
                target_type: ScheduleTargetType::Activity,
                target_name: "x".into(),
                target_input: json!({}),
                enabled: true,
                max_concurrent: None,
                catch_up_missed: false,
                max_catch_up: None,
                retry_policy: None,
                next_trigger_at: None,
            })
            .await;
        assert!(matches!(duplicate, Err(StoreError::Database(e)) if is_unique_violation(&e)));
    }

    #[tokio::test]
    async fn disable_stops_an_enabled_schedule_only() {
        let store = InMemoryWorkflowEventStore::new();
        assert!(!disable_schedule(&store, "nightly-sweep").await.unwrap());

        ensure_schedule(&store, &spec()).await.unwrap();
        assert!(disable_schedule(&store, "nightly-sweep").await.unwrap());
        assert!(!disable_schedule(&store, "nightly-sweep").await.unwrap());
        assert!(!named(&store, "nightly-sweep").await[0].enabled);

        // Ensuring again turns it back on.
        assert!(matches!(
            ensure_schedule(&store, &spec()).await.unwrap(),
            EnsureOutcome::Updated(_)
        ));
        assert!(named(&store, "nightly-sweep").await[0].enabled);
    }
}

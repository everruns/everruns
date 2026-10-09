use super::queries as q;
use super::types::{
    CreateScheduleRequest, ListExecutionsQuery, ListSchedulesQuery, ScheduleExecutionResponse,
    ScheduleExecutionsListResponse, ScheduleResponse, ScheduleStatsResponse, SchedulesListResponse,
    TriggerResponse, UpdateScheduleRequest,
};
use crate::domains::common::*;
use crate::storage::UpdateField;
use chrono::Utc;
use everruns_durable::{
    CreateScheduleRow, Pagination, ScheduleExecutionFilter, ScheduleFilter, UpdateSchedule,
};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct CreateSchedule(pub CreateScheduleRequest);

impl CommandSchema for CreateSchedule {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateScheduleRequest>()
    }
}

#[command(
    name = "create_schedule",
    category = "schedules",
    description = "Create a new durable scheduled task with a cron expression.",
    method = "POST",
    path = "/v1/durable/schedules",
    policy = super::SCHEDULE_MANAGE,
    cli = CliRoute::new(&["durable", "schedules"], "create").with_examples(&[CliExample::new("Run a session workflow every night", "everruns durable schedules create --name nightly-triage --cron-expression '0 2 * * *' --timezone UTC --target '{\"type\":\"workflow\",\"name\":\"session.run\",\"input\":{\"session_id\":\"session_01h9\"}}' --reason 'Nightly triage'")]),
)]
impl Command for CreateSchedule {
    type Output = ScheduleResponse;

    async fn execute(self, ctx: &Ctx) -> Result<ScheduleResponse, CommandError> {
        q::ensure_platform_user(ctx)?;
        let store = q::store(ctx)?;
        let mut req = self.0;
        let target_type = q::parse_target_type(&req.target.target_type)?;
        req.cron_expression = q::normalize_cron_expression(&req.cron_expression)?;
        let next_trigger_at = if req.enabled {
            q::calculate_next_trigger(&req.cron_expression)?
        } else {
            None
        };

        let schedule_id = store
            .create_schedule(CreateScheduleRow {
                name: req.name,
                description: req.description,
                cron_expression: req.cron_expression,
                timezone: req.timezone,
                target_type,
                target_name: req.target.name,
                target_input: req.target.input,
                enabled: req.enabled,
                max_concurrent: req.max_concurrent,
                catch_up_missed: req.catch_up_missed,
                max_catch_up: req.max_catch_up,
                retry_policy: req.retry_policy,
                next_trigger_at,
            })
            .await
            .map_err(q::map_store_error)?;

        let schedule = store
            .get_schedule(schedule_id)
            .await
            .map_err(q::map_store_error)?;
        Ok(schedule.into())
    }
}

#[derive(Debug, Default, Deserialize, serde::Serialize)]
pub struct ListSchedules {
    // Bashkit's MCP flag parser forwards bools as JSON strings ("true"/"false"),
    // so the lenient deserializer is required to accept `--enabled true`.
    /// Filter by enabled status.
    #[serde(default, deserialize_with = "deserialize_opt_bool_lenient")]
    pub enabled: Option<bool>,
    /// Filter by target type ("workflow" or "activity").
    pub target_type: Option<String>,
    /// Pagination offset.
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    pub offset: Option<u32>,
    /// Pagination limit.
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    pub limit: Option<u32>,
}

impl CommandSchema for ListSchedules {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<ListSchedulesQuery>()
    }
}

#[command(
    name = "list_schedules",
    category = "schedules",
    description = "List durable scheduled tasks.",
    method = "GET",
    path = "/v1/durable/schedules",
    policy = super::SCHEDULE_VIEW,
    cli = CliRoute::new(&["durable", "schedules"], "list").with_examples(&[CliExample::new("Find schedules that are still enabled", "everruns durable schedules list --enabled true")]),
)]
impl Command for ListSchedules {
    type Output = SchedulesListResponse;

    async fn execute(self, ctx: &Ctx) -> Result<SchedulesListResponse, CommandError> {
        q::ensure_platform_user(ctx)?;
        let store = q::store(ctx)?;
        let filter = ScheduleFilter {
            enabled: self.enabled,
            target_type: q::optional_target_type(self.target_type.as_deref())?,
        };
        let pagination = Pagination {
            offset: self.offset.unwrap_or(0),
            limit: self.limit.unwrap_or(100),
        };
        let schedules = store
            .list_schedules(filter.clone(), pagination)
            .await
            .map_err(q::map_store_error)?;
        let total = store
            .count_schedules(filter)
            .await
            .map_err(q::map_store_error)?;

        Ok(SchedulesListResponse {
            data: schedules.into_iter().map(ScheduleResponse::from).collect(),
            total,
        })
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetSchedule {
    /// Schedule's prefixed public identifier.
    pub schedule_id: uuid::Uuid,
}

#[command(
    name = "get_schedule",
    category = "schedules",
    description = "Get a durable schedule.",
    method = "GET",
    path = "/v1/durable/schedules/{schedule_id}",
    policy = super::SCHEDULE_VIEW,
    cli = CliRoute::new(&["durable", "schedules"], "get").with_args(&[CliArg::new("schedule_id").at(1)]).with_examples(&[CliExample::new("Check a schedule's cron expression, target and next run", "everruns durable schedules get sched_01h9")]),
    positional = "schedule_id",
)]
impl Command for GetSchedule {
    type Output = ScheduleResponse;

    async fn execute(self, ctx: &Ctx) -> Result<ScheduleResponse, CommandError> {
        q::ensure_platform_user(ctx)?;
        let schedule = q::store(ctx)?
            .get_schedule(self.schedule_id)
            .await
            .map_err(q::map_store_error)?;
        Ok(schedule.into())
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateScheduleCmd {
    /// Schedule's prefixed public identifier.
    pub schedule_id: uuid::Uuid,
    #[serde(flatten)]
    pub req: UpdateScheduleRequest,
}

#[command(
    name = "update_schedule",
    category = "schedules",
    description = "Update a durable schedule.",
    method = "PATCH",
    path = "/v1/durable/schedules/{schedule_id}",
    policy = super::SCHEDULE_MANAGE,
    cli = CliRoute::new(&["durable", "schedules"], "update").with_examples(&[CliExample::new("Move a schedule to a different time", "everruns durable schedules update --schedule-id sched_01h9 --cron-expression '0 3 * * *' --reason 'Avoid the 02:00 backup window'")]),
)]
impl Command for UpdateScheduleCmd {
    type Output = ScheduleResponse;

    async fn execute(self, ctx: &Ctx) -> Result<ScheduleResponse, CommandError> {
        q::ensure_platform_user(ctx)?;
        let store = q::store(ctx)?;
        let mut req = self.req;
        req.cron_expression = req
            .cron_expression
            .map(|cron| q::normalize_cron_expression(&cron))
            .transpose()?;
        let target_type = q::optional_target_type(
            req.target
                .as_ref()
                .map(|target| target.target_type.as_str()),
        )?;

        store
            .update_schedule(
                self.schedule_id,
                UpdateSchedule {
                    name: None,
                    description: req
                        .description
                        .map_or(UpdateField::Unchanged, UpdateField::Set),
                    cron_expression: req.cron_expression,
                    timezone: req.timezone,
                    target_type,
                    target_name: req.target.as_ref().map(|target| target.name.clone()),
                    target_input: req.target.map(|target| target.input),
                    enabled: req.enabled,
                    max_concurrent: req
                        .max_concurrent
                        .map_or(UpdateField::Unchanged, UpdateField::Set),
                    catch_up_missed: req.catch_up_missed,
                    max_catch_up: req
                        .max_catch_up
                        .map_or(UpdateField::Unchanged, UpdateField::Set),
                    retry_policy: req
                        .retry_policy
                        .map_or(UpdateField::Unchanged, UpdateField::Set),
                    next_trigger_at: UpdateField::Unchanged,
                },
            )
            .await
            .map_err(q::map_store_error)?;

        let schedule = store
            .get_schedule(self.schedule_id)
            .await
            .map_err(q::map_store_error)?;
        Ok(schedule.into())
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteSchedule {
    /// Schedule's prefixed public identifier.
    pub schedule_id: uuid::Uuid,
}

#[command(
    name = "delete_schedule",
    category = "schedules",
    description = "Delete a durable schedule.",
    method = "DELETE",
    path = "/v1/durable/schedules/{schedule_id}",
    policy = super::SCHEDULE_MANAGE,
    cli = CliRoute::new(&["durable", "schedules"], "delete").with_examples(&[CliExample::new("Remove a schedule you no longer need", "everruns durable schedules delete --schedule-id sched_01h9 --reason 'Replaced by a webhook trigger'")]),
)]
impl Command for DeleteSchedule {
    type Output = bool;

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        q::ensure_platform_user(ctx)?;
        q::store(ctx)?
            .delete_schedule(self.schedule_id)
            .await
            .map_err(q::map_store_error)?;
        Ok(true)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct PauseSchedule {
    /// Schedule's prefixed public identifier.
    pub schedule_id: uuid::Uuid,
}

#[command(
    name = "pause_schedule",
    category = "schedules",
    description = "Pause a durable schedule.",
    method = "POST",
    path = "/v1/durable/schedules/{schedule_id}/pause",
    policy = super::SCHEDULE_MANAGE,
    cli = CliRoute::new(&["durable", "schedules"], "pause").with_examples(&[CliExample::new("Stop a schedule from firing while you investigate", "everruns durable schedules pause --schedule-id sched_01h9 --reason 'Investigate repeated failures'")]),
)]
impl Command for PauseSchedule {
    type Output = ScheduleResponse;

    async fn execute(self, ctx: &Ctx) -> Result<ScheduleResponse, CommandError> {
        q::ensure_platform_user(ctx)?;
        let store = q::store(ctx)?;
        store
            .update_schedule(
                self.schedule_id,
                UpdateSchedule {
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .await
            .map_err(q::map_store_error)?;
        let _ = store
            .update_next_trigger(
                self.schedule_id,
                Utc::now() + chrono::Duration::days(365 * 100),
            )
            .await;
        let schedule = store
            .get_schedule(self.schedule_id)
            .await
            .map_err(q::map_store_error)?;
        Ok(schedule.into())
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ResumeSchedule {
    /// Schedule's prefixed public identifier.
    pub schedule_id: uuid::Uuid,
}

#[command(
    name = "resume_schedule",
    category = "schedules",
    description = "Resume a durable schedule.",
    method = "POST",
    path = "/v1/durable/schedules/{schedule_id}/resume",
    policy = super::SCHEDULE_MANAGE,
    cli = CliRoute::new(&["durable", "schedules"], "resume").with_examples(&[CliExample::new("Turn a paused schedule back on", "everruns durable schedules resume --schedule-id sched_01h9 --reason 'Failures fixed'")]),
)]
impl Command for ResumeSchedule {
    type Output = ScheduleResponse;

    async fn execute(self, ctx: &Ctx) -> Result<ScheduleResponse, CommandError> {
        q::ensure_platform_user(ctx)?;
        let store = q::store(ctx)?;
        let current = store
            .get_schedule(self.schedule_id)
            .await
            .map_err(q::map_store_error)?;
        let next_trigger = q::calculate_next_trigger(&current.cron_expression)?;

        store
            .update_schedule(
                self.schedule_id,
                UpdateSchedule {
                    enabled: Some(true),
                    ..Default::default()
                },
            )
            .await
            .map_err(q::map_store_error)?;
        if let Some(next) = next_trigger {
            let _ = store.update_next_trigger(self.schedule_id, next).await;
        }
        let schedule = store
            .get_schedule(self.schedule_id)
            .await
            .map_err(q::map_store_error)?;
        Ok(schedule.into())
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct TriggerSchedule {
    /// Schedule's prefixed public identifier.
    pub schedule_id: uuid::Uuid,
}

#[command(
    name = "trigger_schedule",
    category = "schedules",
    description = "Manually trigger a durable schedule.",
    method = "POST",
    path = "/v1/durable/schedules/{schedule_id}/trigger",
    policy = super::SCHEDULE_MANAGE,
    cli = CliRoute::new(&["durable", "schedules"], "trigger").with_examples(&[CliExample::new("Run a schedule once now to test its target", "everruns durable schedules trigger --schedule-id sched_01h9 --reason 'Verify the target before the next run'")]),
)]
impl Command for TriggerSchedule {
    type Output = TriggerResponse;

    async fn execute(self, ctx: &Ctx) -> Result<TriggerResponse, CommandError> {
        q::ensure_platform_user(ctx)?;
        let store = q::store(ctx)?;
        let _ = store
            .get_schedule(self.schedule_id)
            .await
            .map_err(q::map_store_error)?;
        let execution_id = store
            .create_schedule_execution(self.schedule_id, Utc::now())
            .await
            .map_err(q::map_store_error)?;
        Ok(TriggerResponse { execution_id })
    }
}

#[derive(Debug, Default, Deserialize, serde::Serialize)]
pub struct ListScheduleExecutions {
    /// Schedule's prefixed public identifier.
    pub schedule_id: uuid::Uuid,
    pub status: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    pub offset: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    pub limit: Option<u32>,
}

impl CommandSchema for ListScheduleExecutions {
    fn param_schema() -> serde_json::Value {
        let mut schema = delegated_param_schema::<ListExecutionsQuery>();
        if let Some(properties) = schema
            .get_mut("properties")
            .and_then(|value| value.as_object_mut())
        {
            properties.insert(
                "schedule_id".to_string(),
                serde_json::json!({
                    "type": "string",
                    "format": "uuid",
                    "description": "Schedule ID",
                }),
            );
        }
        if let Some(required) = schema
            .get_mut("required")
            .and_then(|value| value.as_array_mut())
        {
            required.push(serde_json::Value::String("schedule_id".to_string()));
        } else if let Some(obj) = schema.as_object_mut() {
            obj.insert(
                "required".to_string(),
                serde_json::Value::Array(vec![serde_json::Value::String(
                    "schedule_id".to_string(),
                )]),
            );
        }
        schema
    }
}

#[command(
    name = "list_schedule_executions",
    category = "schedules",
    description = "List executions for a durable schedule.",
    method = "GET",
    path = "/v1/durable/schedules/{schedule_id}/executions",
    policy = super::SCHEDULE_VIEW,
    cli = CliRoute::new(&["durable", "schedules", "executions"], "list").with_examples(&[CliExample::new("Find recent failed runs of a schedule", "everruns durable schedules executions list --schedule-id sched_01h9 --status failed")]),
)]
impl Command for ListScheduleExecutions {
    type Output = ScheduleExecutionsListResponse;

    async fn execute(self, ctx: &Ctx) -> Result<ScheduleExecutionsListResponse, CommandError> {
        q::ensure_platform_user(ctx)?;
        let store = q::store(ctx)?;
        let _ = store
            .get_schedule(self.schedule_id)
            .await
            .map_err(q::map_store_error)?;
        let filter = ScheduleExecutionFilter {
            schedule_id: Some(self.schedule_id),
            status: self
                .status
                .as_deref()
                .map(q::parse_execution_status)
                .transpose()?,
        };
        let pagination = Pagination {
            offset: self.offset.unwrap_or(0),
            limit: self.limit.unwrap_or(100),
        };
        let executions = store
            .list_schedule_executions(filter, pagination)
            .await
            .map_err(q::map_store_error)?;
        let total = executions.len();
        Ok(ScheduleExecutionsListResponse {
            data: executions
                .into_iter()
                .map(ScheduleExecutionResponse::from)
                .collect(),
            total,
        })
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetExecution {
    /// Schedule execution's identifier.
    pub execution_id: uuid::Uuid,
}

#[command(
    name = "get_execution",
    category = "schedules",
    description = "Get a durable schedule execution.",
    method = "GET",
    path = "/v1/durable/executions/{execution_id}",
    policy = super::SCHEDULE_VIEW,
    cli = CliRoute::new(&["durable", "executions"], "get").with_args(&[CliArg::new("execution_id").at(1)]).with_examples(&[CliExample::new("Inspect one run of a schedule, including its error if it failed", "everruns durable executions get exec_01h9")]),
    positional = "execution_id",
)]
impl Command for GetExecution {
    type Output = ScheduleExecutionResponse;

    async fn execute(self, ctx: &Ctx) -> Result<ScheduleExecutionResponse, CommandError> {
        q::ensure_platform_user(ctx)?;
        let execution = q::store(ctx)?
            .get_schedule_execution(self.execution_id)
            .await
            .map_err(q::map_store_error)?;
        Ok(execution.into())
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetScheduleStats {
    /// Schedule's prefixed public identifier.
    pub schedule_id: uuid::Uuid,
}

#[command(
    name = "get_schedule_stats",
    category = "schedules",
    description = "Get durable schedule statistics.",
    method = "GET",
    path = "/v1/durable/schedules/{schedule_id}/stats",
    policy = super::SCHEDULE_VIEW,
    cli = CliRoute::new(&["durable", "schedules", "stats"], "get").with_examples(&[CliExample::new("See a schedule's success and failure counts", "everruns durable schedules stats get --schedule-id sched_01h9")]),
)]
impl Command for GetScheduleStats {
    type Output = ScheduleStatsResponse;

    async fn execute(self, ctx: &Ctx) -> Result<ScheduleStatsResponse, CommandError> {
        q::ensure_platform_user(ctx)?;
        let store = q::store(ctx)?;
        let _ = store
            .get_schedule(self.schedule_id)
            .await
            .map_err(q::map_store_error)?;
        let stats = store
            .get_schedule_stats(self.schedule_id)
            .await
            .map_err(q::map_store_error)?;
        Ok(stats.into())
    }
}

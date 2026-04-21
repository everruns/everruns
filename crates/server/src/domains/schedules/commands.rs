use super::queries as q;
use super::types::{
    CreateScheduleRequest, ListExecutionsQuery, ListSchedulesQuery, ScheduleExecutionResponse,
    ScheduleExecutionsListResponse, ScheduleResponse, ScheduleStatsResponse, SchedulesListResponse,
    TriggerResponse, UpdateScheduleRequest,
};
use crate::domains::common::*;
use chrono::Utc;
use everruns_durable::{
    CreateScheduleRow, Pagination, ScheduleExecutionFilter, ScheduleFilter, UpdateField,
    UpdateSchedule,
};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize)]
pub struct CreateSchedule(pub CreateScheduleRequest);

impl CommandSchema for CreateSchedule {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateScheduleRequest>()
    }
}

impl Command for CreateSchedule {
    type Output = ScheduleResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_schedule",
            category: "schedules",
            description: "Create a new durable scheduled task with a cron expression.",
            method: "POST",
            path: "/v1/durable/schedules",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<ScheduleResponse, CommandError> {
        q::ensure_platform_user(ctx)?;
        let store = q::store(ctx)?;
        let req = self.0;
        let target_type = q::parse_target_type(&req.target.target_type)?;
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

inventory::submit! { CommandDescriptor::of::<CreateSchedule>() }

#[derive(Debug, Default, Deserialize)]
pub struct ListSchedules {
    pub enabled: Option<bool>,
    pub target_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    pub offset: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    pub limit: Option<u32>,
}

impl CommandSchema for ListSchedules {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<ListSchedulesQuery>()
    }
}

impl Command for ListSchedules {
    type Output = SchedulesListResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_schedules",
            category: "schedules",
            description: "List durable scheduled tasks.",
            method: "GET",
            path: "/v1/durable/schedules",
        }
    }

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

inventory::submit! { CommandDescriptor::of::<ListSchedules>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetSchedule {
    pub schedule_id: uuid::Uuid,
}

impl Command for GetSchedule {
    type Output = ScheduleResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_schedule",
            category: "schedules",
            description: "Get a durable schedule.",
            method: "GET",
            path: "/v1/durable/schedules/{schedule_id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("schedule_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<ScheduleResponse, CommandError> {
        q::ensure_platform_user(ctx)?;
        let schedule = q::store(ctx)?
            .get_schedule(self.schedule_id)
            .await
            .map_err(q::map_store_error)?;
        Ok(schedule.into())
    }
}

inventory::submit! { CommandDescriptor::of::<GetSchedule>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateScheduleCmd {
    pub schedule_id: uuid::Uuid,
    #[serde(flatten)]
    pub req: UpdateScheduleRequest,
}

impl Command for UpdateScheduleCmd {
    type Output = ScheduleResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_schedule",
            category: "schedules",
            description: "Update a durable schedule.",
            method: "PATCH",
            path: "/v1/durable/schedules/{schedule_id}",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<ScheduleResponse, CommandError> {
        q::ensure_platform_user(ctx)?;
        let store = q::store(ctx)?;
        let req = self.req;
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

inventory::submit! { CommandDescriptor::of::<UpdateScheduleCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteSchedule {
    pub schedule_id: uuid::Uuid,
}

impl Command for DeleteSchedule {
    type Output = bool;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_schedule",
            category: "schedules",
            description: "Delete a durable schedule.",
            method: "DELETE",
            path: "/v1/durable/schedules/{schedule_id}",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        q::ensure_platform_user(ctx)?;
        q::store(ctx)?
            .delete_schedule(self.schedule_id)
            .await
            .map_err(q::map_store_error)?;
        Ok(true)
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteSchedule>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct PauseSchedule {
    pub schedule_id: uuid::Uuid,
}

impl Command for PauseSchedule {
    type Output = ScheduleResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "pause_schedule",
            category: "schedules",
            description: "Pause a durable schedule.",
            method: "POST",
            path: "/v1/durable/schedules/{schedule_id}/pause",
        }
    }

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

inventory::submit! { CommandDescriptor::of::<PauseSchedule>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ResumeSchedule {
    pub schedule_id: uuid::Uuid,
}

impl Command for ResumeSchedule {
    type Output = ScheduleResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "resume_schedule",
            category: "schedules",
            description: "Resume a durable schedule.",
            method: "POST",
            path: "/v1/durable/schedules/{schedule_id}/resume",
        }
    }

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

inventory::submit! { CommandDescriptor::of::<ResumeSchedule>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct TriggerSchedule {
    pub schedule_id: uuid::Uuid,
}

impl Command for TriggerSchedule {
    type Output = TriggerResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "trigger_schedule",
            category: "schedules",
            description: "Manually trigger a durable schedule.",
            method: "POST",
            path: "/v1/durable/schedules/{schedule_id}/trigger",
        }
    }

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

inventory::submit! { CommandDescriptor::of::<TriggerSchedule>() }

#[derive(Debug, Default, Deserialize)]
pub struct ListScheduleExecutions {
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

impl Command for ListScheduleExecutions {
    type Output = ScheduleExecutionsListResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_schedule_executions",
            category: "schedules",
            description: "List executions for a durable schedule.",
            method: "GET",
            path: "/v1/durable/schedules/{schedule_id}/executions",
        }
    }

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

inventory::submit! { CommandDescriptor::of::<ListScheduleExecutions>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetExecution {
    pub execution_id: uuid::Uuid,
}

impl Command for GetExecution {
    type Output = ScheduleExecutionResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_execution",
            category: "schedules",
            description: "Get a durable schedule execution.",
            method: "GET",
            path: "/v1/durable/executions/{execution_id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("execution_id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<ScheduleExecutionResponse, CommandError> {
        q::ensure_platform_user(ctx)?;
        let execution = q::store(ctx)?
            .get_schedule_execution(self.execution_id)
            .await
            .map_err(q::map_store_error)?;
        Ok(execution.into())
    }
}

inventory::submit! { CommandDescriptor::of::<GetExecution>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetScheduleStats {
    pub schedule_id: uuid::Uuid,
}

impl Command for GetScheduleStats {
    type Output = ScheduleStatsResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_schedule_stats",
            category: "schedules",
            description: "Get durable schedule statistics.",
            method: "GET",
            path: "/v1/durable/schedules/{schedule_id}/stats",
        }
    }

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

inventory::submit! { CommandDescriptor::of::<GetScheduleStats>() }

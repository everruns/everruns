// Durable Scheduled Tasks API routes
//
// Decision: All endpoints require explicit platform-user auth.
//   Matches the durable control-plane contract for global/system surfaces.
//
// Provides CRUD operations and management for scheduled tasks:
// - Schedule creation, listing, updates, and deletion
// - Pause/resume functionality
// - Manual triggering
// - Execution history

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
};
use chrono::{DateTime, Utc};
use everruns_durable::{
    ScheduleExecutionRow, ScheduleExecutionStatus, ScheduleRow, ScheduleStats, ScheduleTargetType,
    WorkflowEventStore,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::common::{ApiResult, ErrorResponse, impl_auth_state};
use crate::auth::{AuthState, PlatformUser, rate_limit::OrgRateLimiter};
use crate::domains::common::{Command, Ctx};
use crate::domains::schedules::{
    CreateSchedule, DeleteSchedule, GetExecution, GetSchedule, GetScheduleStats,
    ListScheduleExecutions, ListSchedules, PauseSchedule, ResumeSchedule, TriggerSchedule,
    UpdateScheduleCmd,
};
use crate::storage::StorageBackend;
use everruns_core::{Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole};

/// App state for schedule routes
/// All schedule endpoints require platform-user auth.
#[derive(Clone)]
pub struct ScheduleAppState {
    db: Arc<StorageBackend>,
    store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    auth: AuthState,
    org_rate_limiter: OrgRateLimiter,
}

impl_auth_state!(ScheduleAppState);

impl ScheduleAppState {
    /// Create new state with an optional workflow event store and auth state
    pub fn new(
        db: Arc<StorageBackend>,
        store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            store,
            auth,
            org_rate_limiter: OrgRateLimiter::default(),
        }
    }

    pub fn with_org_rate_limiter(mut self, limiter: OrgRateLimiter) -> Self {
        self.org_rate_limiter = limiter;
        self
    }

    /// Get the store, returning an error response if not available
    fn get_store(
        &self,
    ) -> Result<&Arc<dyn WorkflowEventStore + Send + Sync>, (StatusCode, Json<ErrorResponse>)> {
        self.store.as_ref().ok_or_else(|| {
            ErrorResponse::new("Durable execution store not available".to_string())
                .into_response(StatusCode::SERVICE_UNAVAILABLE)
        })
    }

    fn ctx(&self, auth: &PlatformUser) -> Result<Ctx, (StatusCode, Json<ErrorResponse>)> {
        let caller = Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            user_id: Some(auth.0.id),
            role: OrgRole::Owner,
            is_platform_user: auth.0.is_platform_user,
            is_internal: false,
        };
        Ok(Ctx::minimal(
            caller,
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_workflow_store(Some(self.get_store()?.clone())))
    }
}

/// Create schedule routes
pub fn routes(state: ScheduleAppState) -> Router {
    Router::new()
        // CRUD
        .route("/v1/durable/schedules", post(create_schedule))
        .route("/v1/durable/schedules", get(list_schedules))
        .route("/v1/durable/schedules/{schedule_id}", get(get_schedule))
        .route(
            "/v1/durable/schedules/{schedule_id}",
            patch(update_schedule),
        )
        .route(
            "/v1/durable/schedules/{schedule_id}",
            delete(delete_schedule),
        )
        // Actions
        .route(
            "/v1/durable/schedules/{schedule_id}/pause",
            post(pause_schedule),
        )
        .route(
            "/v1/durable/schedules/{schedule_id}/resume",
            post(resume_schedule),
        )
        .route(
            "/v1/durable/schedules/{schedule_id}/trigger",
            post(trigger_schedule),
        )
        // Executions
        .route(
            "/v1/durable/schedules/{schedule_id}/executions",
            get(list_schedule_executions),
        )
        .route("/v1/durable/executions/{execution_id}", get(get_execution))
        // Stats
        .route(
            "/v1/durable/schedules/{schedule_id}/stats",
            get(get_schedule_stats),
        )
        .with_state(state)
}

// ============================================================================
// Request types
// ============================================================================

/// Target for a schedule - either a workflow or activity
#[derive(Debug, Deserialize, ToSchema)]
#[schema(example = json!({"type": "workflow", "name": "session.run", "input": {"session_id": "session_01933b5a00007000800000000000001"}}))]
pub struct ScheduleTarget {
    /// Target type: "workflow" or "activity"
    #[serde(rename = "type")]
    #[schema(example = "workflow")]
    pub target_type: String,
    /// Workflow type name or activity type name
    #[schema(example = "session.run")]
    pub name: String,
    /// Input JSON for the workflow/activity
    #[serde(default)]
    #[schema(example = json!({"session_id": "session_01933b5a00007000800000000000001"}))]
    pub input: serde_json::Value,
}

/// Create schedule request
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateScheduleRequest {
    /// Unique name for the schedule
    #[schema(example = "nightly-triage")]
    pub name: String,
    /// Optional description
    #[schema(example = "Fires the support-triage agent every night at 02:00 UTC")]
    pub description: Option<String>,
    /// Cron expression (5-field or 7-field). Standard `min hour day-of-month month day-of-week` form.
    #[schema(example = "0 2 * * *")]
    pub cron_expression: String,
    /// Timezone (default: UTC). IANA name (e.g. `UTC`, `America/New_York`).
    #[serde(default = "default_timezone")]
    #[schema(example = "UTC")]
    pub timezone: String,
    /// Target to trigger. Variant shape is defined on `ScheduleTarget`.
    pub target: ScheduleTarget,
    /// Whether schedule is enabled (default: true)
    #[serde(default = "default_enabled")]
    #[schema(example = true)]
    pub enabled: bool,
    /// Max concurrent executions. Omit for no limit beyond the worker pool's concurrency.
    #[schema(example = 1)]
    pub max_concurrent: Option<u32>,
    /// Whether to catch up missed triggers (default: false)
    #[serde(default)]
    #[schema(example = false)]
    pub catch_up_missed: bool,
    /// Max catch-up executions when `catch_up_missed` is true. Older missed fires are dropped.
    #[schema(example = 3)]
    pub max_catch_up: Option<u32>,
    /// Retry policy for failed executions (provider-specific JSON; see the durable engine's `RetryPolicy`).
    /// Example: `{"max_attempts": 3, "initial_backoff_secs": 30, "backoff_multiplier": 2.0}`.
    pub retry_policy: Option<serde_json::Value>,
}

fn default_timezone() -> String {
    "UTC".to_string()
}

fn default_enabled() -> bool {
    true
}

/// Update schedule request
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateScheduleRequest {
    /// New description
    #[schema(example = "Fires the support-triage agent every weekday at 08:00 UTC")]
    pub description: Option<String>,
    /// New cron expression. Standard `min hour day-of-month month day-of-week` form.
    #[schema(example = "0 8 * * 1-5")]
    pub cron_expression: Option<String>,
    /// New timezone (IANA name).
    #[schema(example = "America/New_York")]
    pub timezone: Option<String>,
    /// New target. Variant shape is defined on `ScheduleTarget`.
    pub target: Option<ScheduleTarget>,
    /// Enable/disable
    #[schema(example = true)]
    pub enabled: Option<bool>,
    /// Max concurrent executions
    #[schema(example = 1)]
    pub max_concurrent: Option<u32>,
    /// Catch up missed triggers
    #[schema(example = true)]
    pub catch_up_missed: Option<bool>,
    /// Max catch-up executions
    #[schema(example = 3)]
    pub max_catch_up: Option<u32>,
    /// Retry policy (provider-specific JSON; see the durable engine's `RetryPolicy`).
    /// Example: `{"max_attempts": 3, "initial_backoff_secs": 30, "backoff_multiplier": 2.0}`.
    pub retry_policy: Option<serde_json::Value>,
}

// ============================================================================
// Response types
// ============================================================================

/// Schedule response
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleResponse {
    /// UUID of the schedule.
    #[schema(example = "01933b5a-0000-7000-8000-000000000001")]
    pub id: Uuid,
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "nightly-triage")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Fires the support-triage agent every night at 02:00 UTC")]
    pub description: Option<String>,
    /// Cron expression in the standard `min hour day-of-month month day-of-week` form (6 fields with seconds optional). Evaluated in `timezone`.
    #[schema(example = "0 2 * * *")]
    pub cron_expression: String,
    /// IANA timezone name used to interpret `cron_expression` (e.g. `UTC`, `America/New_York`).
    #[schema(example = "UTC")]
    pub timezone: String,
    /// What the schedule invokes when it fires (a session, an agent, an app channel, etc.).
    pub target: ScheduleTargetResponse,
    /// When `false`, the schedule is paused — kept in storage but never fires until re-enabled.
    #[schema(example = true)]
    pub enabled: bool,
    /// Maximum number of overlapping executions allowed. `None` means no limit beyond the worker pool's concurrency.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 1)]
    pub max_concurrent: Option<u32>,
    /// When `true`, missed fires (while the scheduler was down, paused, or unreachable) are queued and run after recovery, subject to `max_catch_up`.
    #[schema(example = false)]
    pub catch_up_missed: bool,
    /// Maximum number of missed fires to replay when `catch_up_missed` is `true`. Older missed fires are dropped. `None` means no cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 3)]
    pub max_catch_up: Option<u32>,
    /// Optional retry policy for failed runs (provider-specific JSON; see the durable engine's `RetryPolicy`).
    /// Example: `{"max_attempts": 3, "initial_backoff_secs": 30, "backoff_multiplier": 2.0}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<serde_json::Value>,
    /// Timestamp of the most recent fire (RFC 3339). `None` if never triggered.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "2026-05-25T02:00:00Z")]
    pub last_triggered_at: Option<DateTime<Utc>>,
    /// Timestamp of the next scheduled fire (RFC 3339). `None` when the schedule is disabled or the cron expression has no upcoming match.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "2026-05-26T02:00:00Z")]
    pub next_trigger_at: Option<DateTime<Utc>>,
    /// Timestamp when this resource was created (RFC 3339).
    #[schema(example = "2026-05-01T12:00:00Z")]
    pub created_at: DateTime<Utc>,
    /// Timestamp when this resource was last updated (RFC 3339).
    #[schema(example = "2026-05-20T12:00:00Z")]
    pub updated_at: DateTime<Utc>,
}

/// Schedule target response
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleTargetResponse {
    /// Target type discriminator (`workflow` or `activity`).
    #[serde(rename = "type")]
    #[schema(example = "workflow")]
    pub target_type: String,
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "session.run")]
    pub name: String,
    /// Input JSON payload passed to the workflow/activity on each fire.
    /// Example: `{"session_id": "session_01933b5a000070008000000000000001"}`.
    #[schema(value_type = Object)]
    pub input: serde_json::Value,
}

impl From<ScheduleRow> for ScheduleResponse {
    fn from(s: ScheduleRow) -> Self {
        let target_type = match s.target_type {
            ScheduleTargetType::Workflow => "workflow",
            ScheduleTargetType::Activity => "activity",
        };

        Self {
            id: s.id,
            name: s.name,
            description: s.description,
            cron_expression: s.cron_expression,
            timezone: s.timezone,
            target: ScheduleTargetResponse {
                target_type: target_type.to_string(),
                name: s.target_name,
                input: s.target_input,
            },
            enabled: s.enabled,
            max_concurrent: s.max_concurrent,
            catch_up_missed: s.catch_up_missed,
            max_catch_up: s.max_catch_up,
            retry_policy: s.retry_policy,
            last_triggered_at: s.last_triggered_at,
            next_trigger_at: s.next_trigger_at,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

/// Schedules list response
#[derive(Debug, Serialize, ToSchema)]
pub struct SchedulesListResponse {
    /// Page of items returned by this query.
    pub data: Vec<ScheduleResponse>,
    /// Total number of items matching the query, across all pages.
    #[schema(example = 17)]
    pub total: u64,
}

/// Schedule execution response
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleExecutionResponse {
    /// UUID of the schedule execution.
    #[schema(example = "01933b5b-0000-7000-8000-000000000001")]
    pub id: Uuid,
    /// UUID of the owning schedule.
    #[schema(example = "01933b5a-0000-7000-8000-000000000001")]
    pub schedule_id: Uuid,
    /// Timestamp when this resource is scheduled to run (RFC 3339).
    #[schema(example = "2026-05-25T02:00:00Z")]
    pub scheduled_at: DateTime<Utc>,
    /// Timestamp when this resource started, if any (RFC 3339).
    #[schema(example = "2026-05-25T02:00:01Z")]
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Timestamp when this resource completed, if any (RFC 3339).
    #[schema(example = "2026-05-25T02:00:12Z")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Current lifecycle status (`pending`, `running`, `completed`, `failed`, `skipped`).
    #[schema(example = "completed")]
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Durable workflow's identifier.
    #[schema(example = "01933b5c-0000-7000-8000-000000000001")]
    pub workflow_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Durable task's identifier.
    #[schema(example = "01933b5d-0000-7000-8000-000000000001")]
    pub task_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable error message, populated when this resource is in a failed state.
    #[schema(example = "workflow.exec.timeout: activity exceeded 30s budget")]
    pub error: Option<String>,
    /// Total execution duration in milliseconds (`completed_at - started_at`). `None` while still running.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 11000)]
    pub duration_ms: Option<i32>,
    /// Timestamp when this resource was created (RFC 3339).
    #[schema(example = "2026-05-25T02:00:00Z")]
    pub created_at: DateTime<Utc>,
}

impl From<ScheduleExecutionRow> for ScheduleExecutionResponse {
    fn from(e: ScheduleExecutionRow) -> Self {
        let status = match e.status {
            ScheduleExecutionStatus::Pending => "pending",
            ScheduleExecutionStatus::Running => "running",
            ScheduleExecutionStatus::Completed => "completed",
            ScheduleExecutionStatus::Failed => "failed",
            ScheduleExecutionStatus::Skipped => "skipped",
        };

        Self {
            id: e.id,
            schedule_id: e.schedule_id,
            scheduled_at: e.scheduled_at,
            started_at: e.started_at,
            completed_at: e.completed_at,
            status: status.to_string(),
            workflow_id: e.workflow_id,
            task_id: e.task_id,
            error: e.error,
            duration_ms: e.duration_ms,
            created_at: e.created_at,
        }
    }
}

/// Schedule executions list response
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleExecutionsListResponse {
    /// Page of items returned by this query.
    pub data: Vec<ScheduleExecutionResponse>,
    /// Total number of items matching the query, across all pages.
    #[schema(example = 142)]
    pub total: usize,
}

/// Schedule stats response
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleStatsResponse {
    /// Total number of executions recorded for this schedule (sum of successful + failed + skipped).
    #[schema(example = 142)]
    pub total_executions: u64,
    /// Count of executions that completed successfully.
    #[schema(example = 137)]
    pub successful_executions: u64,
    /// Count of executions that ended in failure (after exhausting retries).
    #[schema(example = 3)]
    pub failed_executions: u64,
    /// Count of fires that were intentionally skipped (e.g. blocked by `max_concurrent` or disabled mid-fire).
    #[schema(example = 2)]
    pub skipped_executions: u64,
    /// Average execution duration in milliseconds across `total_executions`. `None` when no executions exist yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 8421)]
    pub avg_duration_ms: Option<u64>,
    /// Status of the most recent execution (one of the `ScheduleExecutionResponse.status` values:
    /// `pending`, `running`, `completed`, `failed`, `skipped`). `None` if the schedule has never fired.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "completed")]
    pub last_execution_status: Option<String>,
}

impl From<ScheduleStats> for ScheduleStatsResponse {
    fn from(s: ScheduleStats) -> Self {
        let last_status = s.last_execution_status.map(|st| {
            match st {
                ScheduleExecutionStatus::Pending => "pending",
                ScheduleExecutionStatus::Running => "running",
                ScheduleExecutionStatus::Completed => "completed",
                ScheduleExecutionStatus::Failed => "failed",
                ScheduleExecutionStatus::Skipped => "skipped",
            }
            .to_string()
        });

        Self {
            total_executions: s.total_executions,
            successful_executions: s.successful_executions,
            failed_executions: s.failed_executions,
            skipped_executions: s.skipped_executions,
            avg_duration_ms: s.avg_duration_ms,
            last_execution_status: last_status,
        }
    }
}

/// Manual trigger response
#[derive(Debug, Serialize, ToSchema)]
pub struct TriggerResponse {
    /// Schedule execution's identifier.
    pub execution_id: Uuid,
}

// ============================================================================
// Query parameters
// ============================================================================

/// Query parameters for `GET /v1/schedules` — optional enabled/target-type
/// filters plus standard offset/limit paging.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListSchedulesQuery {
    /// Filter by enabled status
    #[schema(example = true)]
    pub enabled: Option<bool>,
    /// Filter by target type ("workflow" or "activity")
    #[schema(example = "workflow")]
    pub target_type: Option<String>,
    /// Pagination offset
    #[schema(example = 0)]
    pub offset: Option<u32>,
    /// Pagination limit (default: 20, max: 100)
    #[schema(example = 20)]
    pub limit: Option<u32>,
}

/// Query parameters for listing executions of a schedule — optional status
/// filter plus offset/limit paging.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListExecutionsQuery {
    /// Filter by execution status
    #[schema(example = "completed")]
    pub status: Option<String>,
    /// Pagination offset
    #[schema(example = 0)]
    pub offset: Option<u32>,
    /// Pagination limit (default: 20, max: 100)
    #[schema(example = 20)]
    pub limit: Option<u32>,
}

// ============================================================================
// Route handlers
// ============================================================================

/// POST /v1/durable/schedules - Create a new schedule
#[utoipa::path(
    post,
    path = "/v1/durable/schedules",
    request_body = CreateScheduleRequest,
    responses(
        (status = 201, description = "Schedule created", body = ScheduleResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 401, description = "Authentication required"),
        (status = 409, description = "Schedule name already exists", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn create_schedule(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Json(req): Json<CreateScheduleRequest>,
) -> Result<(StatusCode, Json<ScheduleResponse>), (StatusCode, Json<ErrorResponse>)> {
    if state
        .org_rate_limiter
        .check_schedule_create(auth.0.id)
        .await
        .is_err()
    {
        return Err(
            ErrorResponse::new("Too many requests. Please try again later.")
                .with_code("rate_limited")
                .with_retry_after(60)
                .into_response(StatusCode::TOO_MANY_REQUESTS),
        );
    }
    let schedule = CreateSchedule(req).run(&state.ctx(&auth)?).await?;
    Ok((StatusCode::CREATED, Json(schedule)))
}

/// GET /v1/durable/schedules - List schedules
#[utoipa::path(
    get,
    path = "/v1/durable/schedules",
    params(
        ("enabled" = Option<bool>, Query, description = "Filter by enabled status"),
        ("target_type" = Option<String>, Query, description = "Filter by target type"),
        ("offset" = Option<u32>, Query, description = "Pagination offset"),
        ("limit" = Option<u32>, Query, description = "Pagination limit")
    ),
    responses(
        (status = 200, description = "List of schedules", body = SchedulesListResponse),
        (status = 401, description = "Authentication required"),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn list_schedules(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Query(query): Query<ListSchedulesQuery>,
) -> ApiResult<SchedulesListResponse> {
    let schedules = ListSchedules {
        enabled: query.enabled,
        target_type: query.target_type,
        offset: query.offset,
        limit: query.limit,
    }
    .run(&state.ctx(&auth)?)
    .await?;
    Ok(Json(schedules))
}

/// GET /v1/durable/schedules/:schedule_id - Get schedule details
#[utoipa::path(
    get,
    path = "/v1/durable/schedules/{schedule_id}",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    responses(
        (status = 200, description = "Schedule details", body = ScheduleResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn get_schedule(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> ApiResult<ScheduleResponse> {
    let schedule = GetSchedule { schedule_id }.run(&state.ctx(&auth)?).await?;
    Ok(Json(schedule))
}

/// PATCH /v1/durable/schedules/:schedule_id - Update schedule
#[utoipa::path(
    patch,
    path = "/v1/durable/schedules/{schedule_id}",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    request_body = UpdateScheduleRequest,
    responses(
        (status = 200, description = "Schedule updated", body = ScheduleResponse),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn update_schedule(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
    Json(req): Json<UpdateScheduleRequest>,
) -> ApiResult<ScheduleResponse> {
    let schedule = UpdateScheduleCmd { schedule_id, req }
        .run(&state.ctx(&auth)?)
        .await?;
    Ok(Json(schedule))
}

/// DELETE /v1/durable/schedules/:schedule_id - Delete schedule
#[utoipa::path(
    delete,
    path = "/v1/durable/schedules/{schedule_id}",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    responses(
        (status = 204, description = "Schedule deleted"),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn delete_schedule(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    DeleteSchedule { schedule_id }
        .run(&state.ctx(&auth)?)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/durable/schedules/:schedule_id/pause - Pause schedule
#[utoipa::path(
    post,
    path = "/v1/durable/schedules/{schedule_id}/pause",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    responses(
        (status = 200, description = "Schedule paused", body = ScheduleResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn pause_schedule(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> ApiResult<ScheduleResponse> {
    let schedule = PauseSchedule { schedule_id }
        .run(&state.ctx(&auth)?)
        .await?;
    Ok(Json(schedule))
}

/// POST /v1/durable/schedules/:schedule_id/resume - Resume schedule
#[utoipa::path(
    post,
    path = "/v1/durable/schedules/{schedule_id}/resume",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    responses(
        (status = 200, description = "Schedule resumed", body = ScheduleResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn resume_schedule(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> ApiResult<ScheduleResponse> {
    let schedule = ResumeSchedule { schedule_id }
        .run(&state.ctx(&auth)?)
        .await?;
    Ok(Json(schedule))
}

/// POST /v1/durable/schedules/:schedule_id/trigger - Manually trigger schedule
#[utoipa::path(
    post,
    path = "/v1/durable/schedules/{schedule_id}/trigger",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    responses(
        (status = 200, description = "Schedule triggered", body = TriggerResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn trigger_schedule(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> ApiResult<TriggerResponse> {
    let response = TriggerSchedule { schedule_id }
        .run(&state.ctx(&auth)?)
        .await?;
    Ok(Json(response))
}

/// GET /v1/durable/schedules/:schedule_id/executions - List schedule executions
#[utoipa::path(
    get,
    path = "/v1/durable/schedules/{schedule_id}/executions",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID"),
        ("status" = Option<String>, Query, description = "Filter by status"),
        ("offset" = Option<u32>, Query, description = "Pagination offset"),
        ("limit" = Option<u32>, Query, description = "Pagination limit")
    ),
    responses(
        (status = 200, description = "List of executions", body = ScheduleExecutionsListResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn list_schedule_executions(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
    Query(query): Query<ListExecutionsQuery>,
) -> ApiResult<ScheduleExecutionsListResponse> {
    let executions = ListScheduleExecutions {
        schedule_id,
        status: query.status,
        offset: query.offset,
        limit: query.limit,
    }
    .run(&state.ctx(&auth)?)
    .await?;
    Ok(Json(executions))
}

/// GET /v1/durable/executions/:execution_id - Get execution details
#[utoipa::path(
    get,
    path = "/v1/durable/executions/{execution_id}",
    params(
        ("execution_id" = Uuid, Path, description = "Execution ID")
    ),
    responses(
        (status = 200, description = "Execution details", body = ScheduleExecutionResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Execution not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn get_execution(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(execution_id): Path<Uuid>,
) -> ApiResult<ScheduleExecutionResponse> {
    let execution = GetExecution { execution_id }
        .run(&state.ctx(&auth)?)
        .await?;
    Ok(Json(execution))
}

/// GET /v1/durable/schedules/:schedule_id/stats - Get schedule statistics
#[utoipa::path(
    get,
    path = "/v1/durable/schedules/{schedule_id}/stats",
    params(
        ("schedule_id" = Uuid, Path, description = "Schedule ID")
    ),
    responses(
        (status = 200, description = "Schedule statistics", body = ScheduleStatsResponse),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn get_schedule_stats(
    auth: PlatformUser,
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> ApiResult<ScheduleStatsResponse> {
    let stats = GetScheduleStats { schedule_id }
        .run(&state.ctx(&auth)?)
        .await?;
    Ok(Json(stats))
}

// ============================================================================
// Helper functions
// ============================================================================

/// Calculate next trigger time from cron expression
#[cfg(test)]
fn calculate_next_trigger(cron_expression: &str) -> Result<Option<DateTime<Utc>>, String> {
    use cron::Schedule;
    use std::str::FromStr;

    let schedule =
        Schedule::from_str(cron_expression).map_err(|e| format!("Invalid cron: {}", e))?;

    Ok(schedule.upcoming(chrono::Utc).next())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::backend::AuthBackend;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::Request;
    use everruns_core::{OrgMembership, OrgRole};
    use tower::ServiceExt;

    use crate::auth::config::{AuthConfig, AuthMode, JwtConfig};
    use crate::auth::middleware::{AuthError, AuthMethod, AuthUser};
    use crate::auth::routes::AuthConfigResponse;

    /// AuthState with auth disabled (anonymous admin user)
    fn test_auth_state_none() -> AuthState {
        let config = AuthConfig::default(); // mode = None
        AuthState::builtin(
            config,
            Arc::new(crate::storage::StorageBackend::in_memory()),
        )
    }

    /// AuthState with auth enabled (requires valid JWT/API key)
    fn test_auth_state_full() -> AuthState {
        let config = AuthConfig {
            mode: AuthMode::Full,
            jwt: JwtConfig {
                secret: "test-secret-for-unit-tests-only".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        AuthState::builtin(
            config,
            Arc::new(crate::storage::StorageBackend::in_memory()),
        )
    }

    /// Build schedule router with given auth state (no backing store)
    fn app_with_auth(auth: AuthState) -> Router {
        routes(ScheduleAppState::new(
            Arc::new(crate::storage::StorageBackend::in_memory()),
            None,
            auth,
        ))
    }

    /// Verify unauthenticated request to the given method+path returns 401
    async fn assert_unauthenticated_rejected(method: &str, path: &str) {
        let app = app_with_auth(test_auth_state_full());

        let method = method.parse::<axum::http::Method>().unwrap();
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status().as_u16(),
            401,
            "Expected 401 for unauthenticated {} {}",
            resp.status(),
            path
        );
    }

    /// Verify authenticated request (no-auth mode) passes auth layer
    /// (may fail with 503 since no store, but should NOT be 401)
    async fn assert_authenticated_passes(method: &str, path: &str) {
        let app = app_with_auth(test_auth_state_none());

        let method = method.parse::<axum::http::Method>().unwrap();
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Should not be 401 - auth passed. Likely 503 (no store) or 400/404.
        assert_ne!(
            resp.status().as_u16(),
            401,
            "Authenticated user should not get 401 for {}",
            path
        );
    }

    // ---- Unauthenticated rejection tests (all endpoints) ----

    #[tokio::test]
    async fn test_create_schedule_unauthenticated() {
        assert_unauthenticated_rejected("POST", "/v1/durable/schedules").await;
    }

    #[tokio::test]
    async fn test_list_schedules_unauthenticated() {
        assert_unauthenticated_rejected("GET", "/v1/durable/schedules").await;
    }

    #[tokio::test]
    async fn test_get_schedule_unauthenticated() {
        let id = Uuid::now_v7();
        assert_unauthenticated_rejected("GET", &format!("/v1/durable/schedules/{id}")).await;
    }

    #[tokio::test]
    async fn test_update_schedule_unauthenticated() {
        let id = Uuid::now_v7();
        assert_unauthenticated_rejected("PATCH", &format!("/v1/durable/schedules/{id}")).await;
    }

    #[tokio::test]
    async fn test_delete_schedule_unauthenticated() {
        let id = Uuid::now_v7();
        assert_unauthenticated_rejected("DELETE", &format!("/v1/durable/schedules/{id}")).await;
    }

    #[tokio::test]
    async fn test_pause_schedule_unauthenticated() {
        let id = Uuid::now_v7();
        assert_unauthenticated_rejected("POST", &format!("/v1/durable/schedules/{id}/pause")).await;
    }

    #[tokio::test]
    async fn test_resume_schedule_unauthenticated() {
        let id = Uuid::now_v7();
        assert_unauthenticated_rejected("POST", &format!("/v1/durable/schedules/{id}/resume"))
            .await;
    }

    #[tokio::test]
    async fn test_trigger_schedule_unauthenticated() {
        let id = Uuid::now_v7();
        assert_unauthenticated_rejected("POST", &format!("/v1/durable/schedules/{id}/trigger"))
            .await;
    }

    #[tokio::test]
    async fn test_list_executions_unauthenticated() {
        let id = Uuid::now_v7();
        assert_unauthenticated_rejected("GET", &format!("/v1/durable/schedules/{id}/executions"))
            .await;
    }

    #[tokio::test]
    async fn test_get_execution_unauthenticated() {
        let id = Uuid::now_v7();
        assert_unauthenticated_rejected("GET", &format!("/v1/durable/executions/{id}")).await;
    }

    #[tokio::test]
    async fn test_get_stats_unauthenticated() {
        let id = Uuid::now_v7();
        assert_unauthenticated_rejected("GET", &format!("/v1/durable/schedules/{id}/stats")).await;
    }

    // ---- Authenticated access tests (no-auth mode passes auth layer) ----

    #[tokio::test]
    async fn test_list_schedules_authenticated() {
        assert_authenticated_passes("GET", "/v1/durable/schedules").await;
    }

    #[tokio::test]
    async fn test_get_schedule_authenticated() {
        let id = Uuid::now_v7();
        assert_authenticated_passes("GET", &format!("/v1/durable/schedules/{id}")).await;
    }

    #[tokio::test]
    async fn test_delete_schedule_authenticated() {
        let id = Uuid::now_v7();
        assert_authenticated_passes("DELETE", &format!("/v1/durable/schedules/{id}")).await;
    }

    #[tokio::test]
    async fn test_pause_schedule_authenticated() {
        let id = Uuid::now_v7();
        assert_authenticated_passes("POST", &format!("/v1/durable/schedules/{id}/pause")).await;
    }

    #[tokio::test]
    async fn test_resume_schedule_authenticated() {
        let id = Uuid::now_v7();
        assert_authenticated_passes("POST", &format!("/v1/durable/schedules/{id}/resume")).await;
    }

    #[tokio::test]
    async fn test_trigger_schedule_authenticated() {
        let id = Uuid::now_v7();
        assert_authenticated_passes("POST", &format!("/v1/durable/schedules/{id}/trigger")).await;
    }

    #[tokio::test]
    async fn test_list_executions_authenticated() {
        let id = Uuid::now_v7();
        assert_authenticated_passes("GET", &format!("/v1/durable/schedules/{id}/executions")).await;
    }

    #[tokio::test]
    async fn test_get_execution_authenticated() {
        let id = Uuid::now_v7();
        assert_authenticated_passes("GET", &format!("/v1/durable/executions/{id}")).await;
    }

    #[tokio::test]
    async fn test_get_stats_authenticated() {
        let id = Uuid::now_v7();
        assert_authenticated_passes("GET", &format!("/v1/durable/schedules/{id}/stats")).await;
    }

    #[derive(Clone)]
    struct MockAuthBackend {
        is_platform_user: bool,
    }

    #[async_trait]
    impl AuthBackend for MockAuthBackend {
        async fn validate_token(&self, _token: &str) -> Result<AuthUser, AuthError> {
            Ok(AuthUser {
                id: Uuid::new_v4(),
                email: "test@example.com".to_string(),
                name: "Test User".to_string(),
                roles: vec!["user".to_string()],
                is_platform_user: self.is_platform_user,
                auth_method: AuthMethod::Jwt,
                organizations: vec![OrgMembership {
                    org_id: 1,
                    public_id: "org_00000000000000000000000000000001".to_string(),
                    name: "Test Org".to_string(),
                    role: OrgRole::Owner,
                }],
            })
        }

        async fn validate_personal_access_token(
            &self,
            _token: &str,
        ) -> Result<AuthUser, AuthError> {
            Err(AuthError::unauthorized("not supported"))
        }

        fn auth_routes(&self) -> Option<Router> {
            None
        }

        fn auth_config_response(&self) -> AuthConfigResponse {
            AuthConfigResponse {
                mode: "full".to_string(),
                password_auth_enabled: false,
                signup_enabled: false,
                oauth_providers: vec![],
                signup_email_confirm: false,
                captcha: None,
            }
        }
    }

    fn app_with_platform_user(is_platform_user: bool) -> Router {
        let config = AuthConfig {
            mode: AuthMode::Full,
            jwt: JwtConfig {
                secret: "test-secret-for-unit-tests-only".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let auth = AuthState::new(config, Arc::new(MockAuthBackend { is_platform_user }));
        routes(ScheduleAppState::new(
            Arc::new(crate::storage::StorageBackend::in_memory()),
            None,
            auth,
        ))
    }

    #[tokio::test]
    async fn non_platform_user_cannot_list_schedules() {
        let response = app_with_platform_user(false)
            .oneshot(
                Request::builder()
                    .uri("/v1/durable/schedules")
                    .header("Authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn platform_user_can_list_schedules() {
        let response = app_with_platform_user(true)
            .oneshot(
                Request::builder()
                    .uri("/v1/durable/schedules")
                    .header("Authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn non_platform_user_cannot_create_schedule() {
        let response = app_with_platform_user(false)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/durable/schedules")
                    .header("Authorization", "Bearer test-token")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    // ---- Existing unit tests ----

    #[test]
    fn test_calculate_next_trigger_valid() {
        // 7-field cron: every minute
        let result = calculate_next_trigger("0 * * * * * *");
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_calculate_next_trigger_invalid() {
        let result = calculate_next_trigger("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_schedule_response_from_row() {
        let row = ScheduleRow {
            id: Uuid::now_v7(),
            name: "test-schedule".to_string(),
            description: Some("A test".to_string()),
            cron_expression: "0 * * * * * *".to_string(),
            timezone: "UTC".to_string(),
            target_type: ScheduleTargetType::Workflow,
            target_name: "my-workflow".to_string(),
            target_input: serde_json::json!({"key": "value"}),
            enabled: true,
            max_concurrent: Some(2),
            catch_up_missed: false,
            max_catch_up: None,
            retry_policy: None,
            last_triggered_at: None,
            next_trigger_at: Some(Utc::now()),
            claimed_by: None,
            claimed_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let response = ScheduleResponse::from(row);
        assert_eq!(response.name, "test-schedule");
        assert_eq!(response.target.target_type, "workflow");
        assert_eq!(response.target.name, "my-workflow");
        assert!(response.enabled);
    }

    #[test]
    fn test_execution_response_from_row() {
        let row = ScheduleExecutionRow {
            id: Uuid::now_v7(),
            schedule_id: Uuid::now_v7(),
            scheduled_at: Utc::now(),
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            status: ScheduleExecutionStatus::Completed,
            workflow_id: Some(Uuid::now_v7()),
            task_id: None,
            error: None,
            duration_ms: Some(150),
            created_at: Utc::now(),
        };

        let response = ScheduleExecutionResponse::from(row);
        assert_eq!(response.status, "completed");
        assert!(response.workflow_id.is_some());
        assert_eq!(response.duration_ms, Some(150));
    }
}

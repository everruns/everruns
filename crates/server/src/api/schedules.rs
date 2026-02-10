// Durable Scheduled Tasks API routes
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
    CreateScheduleRow, Pagination, ScheduleExecutionFilter, ScheduleExecutionRow,
    ScheduleExecutionStatus, ScheduleFilter, ScheduleRow, ScheduleStats, ScheduleTargetType,
    StoreError, UpdateSchedule, WorkflowEventStore,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::common::ErrorResponse;

/// App state for schedule routes
#[derive(Clone)]
pub struct ScheduleAppState {
    store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
}

impl ScheduleAppState {
    /// Create new state with an optional workflow event store
    pub fn new(store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>) -> Self {
        Self { store }
    }

    /// Get the store, returning an error response if not available
    fn get_store(
        &self,
    ) -> Result<&Arc<dyn WorkflowEventStore + Send + Sync>, (StatusCode, Json<ErrorResponse>)> {
        self.store.as_ref().ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "Durable execution store not available".to_string(),
                }),
            )
        })
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
pub struct ScheduleTarget {
    /// Target type: "workflow" or "activity"
    #[serde(rename = "type")]
    pub target_type: String,
    /// Workflow type name or activity type name
    pub name: String,
    /// Input JSON for the workflow/activity
    #[serde(default)]
    pub input: serde_json::Value,
}

/// Create schedule request
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateScheduleRequest {
    /// Unique name for the schedule
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// Cron expression (5-field or 7-field)
    pub cron_expression: String,
    /// Timezone (default: UTC)
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// Target to trigger
    pub target: ScheduleTarget,
    /// Whether schedule is enabled (default: true)
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Max concurrent executions
    pub max_concurrent: Option<u32>,
    /// Whether to catch up missed triggers (default: false)
    #[serde(default)]
    pub catch_up_missed: bool,
    /// Max catch-up executions
    pub max_catch_up: Option<u32>,
    /// Retry policy for failed executions
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
    pub description: Option<String>,
    /// New cron expression
    pub cron_expression: Option<String>,
    /// New timezone
    pub timezone: Option<String>,
    /// New target
    pub target: Option<ScheduleTarget>,
    /// Enable/disable
    pub enabled: Option<bool>,
    /// Max concurrent executions
    pub max_concurrent: Option<u32>,
    /// Catch up missed triggers
    pub catch_up_missed: Option<bool>,
    /// Max catch-up executions
    pub max_catch_up: Option<u32>,
    /// Retry policy
    pub retry_policy: Option<serde_json::Value>,
}

// ============================================================================
// Response types
// ============================================================================

/// Schedule response
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleResponse {
    pub id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub cron_expression: String,
    pub timezone: String,
    pub target: ScheduleTargetResponse,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<u32>,
    pub catch_up_missed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_catch_up: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_triggered_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_trigger_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Schedule target response
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleTargetResponse {
    #[serde(rename = "type")]
    pub target_type: String,
    pub name: String,
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
    pub data: Vec<ScheduleResponse>,
    pub total: u64,
}

/// Schedule execution response
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleExecutionResponse {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub scheduled_at: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i32>,
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
    pub data: Vec<ScheduleExecutionResponse>,
    pub total: usize,
}

/// Schedule stats response
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleStatsResponse {
    pub total_executions: u64,
    pub successful_executions: u64,
    pub failed_executions: u64,
    pub skipped_executions: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
    pub execution_id: Uuid,
}

// ============================================================================
// Query parameters
// ============================================================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListSchedulesQuery {
    /// Filter by enabled status
    pub enabled: Option<bool>,
    /// Filter by target type ("workflow" or "activity")
    pub target_type: Option<String>,
    /// Pagination offset
    pub offset: Option<u32>,
    /// Pagination limit (default: 20, max: 100)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListExecutionsQuery {
    /// Filter by execution status
    pub status: Option<String>,
    /// Pagination offset
    pub offset: Option<u32>,
    /// Pagination limit (default: 20, max: 100)
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
        (status = 409, description = "Schedule name already exists", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn create_schedule(
    State(state): State<ScheduleAppState>,
    Json(req): Json<CreateScheduleRequest>,
) -> Result<(StatusCode, Json<ScheduleResponse>), (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;

    // Parse target type
    let target_type = match req.target.target_type.as_str() {
        "workflow" => ScheduleTargetType::Workflow,
        "activity" => ScheduleTargetType::Activity,
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid target type. Must be 'workflow' or 'activity'".to_string(),
                }),
            ));
        }
    };

    // Calculate initial next_trigger_at
    let next_trigger_at = if req.enabled {
        calculate_next_trigger(&req.cron_expression).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid cron expression: {}", e),
                }),
            )
        })?
    } else {
        None
    };

    // Create schedule row
    let create_row = CreateScheduleRow {
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
    };

    let schedule_id = store
        .create_schedule(create_row)
        .await
        .map_err(|e| match e {
            StoreError::ScheduleLimitExceeded { limit, .. } => (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorResponse {
                    error: format!("Schedule limit exceeded: max {} schedules", limit),
                }),
            ),
            StoreError::InvalidCronExpression(msg) => (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid cron expression: {}", msg),
                }),
            ),
            _ => {
                tracing::error!("Failed to create schedule: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Failed to create schedule".to_string(),
                    }),
                )
            }
        })?;

    // Fetch the created schedule
    let schedule = store.get_schedule(schedule_id).await.map_err(|e| {
        tracing::error!("Failed to get created schedule: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Failed to get created schedule".to_string(),
            }),
        )
    })?;

    Ok((StatusCode::CREATED, Json(ScheduleResponse::from(schedule))))
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
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn list_schedules(
    State(state): State<ScheduleAppState>,
    Query(query): Query<ListSchedulesQuery>,
) -> Result<Json<SchedulesListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;

    let target_type = query.target_type.and_then(|t| match t.as_str() {
        "workflow" => Some(ScheduleTargetType::Workflow),
        "activity" => Some(ScheduleTargetType::Activity),
        _ => None,
    });

    let filter = ScheduleFilter {
        enabled: query.enabled,
        target_type,
    };

    let pagination = Pagination {
        offset: query.offset.unwrap_or(0),
        limit: query.limit.unwrap_or(100),
    };

    let schedules = store
        .list_schedules(filter.clone(), pagination)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list schedules: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to list schedules".to_string(),
                }),
            )
        })?;

    let total = store.count_schedules(filter).await.map_err(|e| {
        tracing::error!("Failed to count schedules: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Failed to count schedules".to_string(),
            }),
        )
    })?;

    let data: Vec<ScheduleResponse> = schedules.into_iter().map(ScheduleResponse::from).collect();

    Ok(Json(SchedulesListResponse { data, total }))
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
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn get_schedule(
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> Result<Json<ScheduleResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;

    let schedule = store.get_schedule(schedule_id).await.map_err(|e| match e {
        StoreError::ScheduleNotFound(_) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Schedule not found".to_string(),
            }),
        ),
        _ => {
            tracing::error!("Failed to get schedule: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to get schedule".to_string(),
                }),
            )
        }
    })?;

    Ok(Json(ScheduleResponse::from(schedule)))
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
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn update_schedule(
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
    Json(req): Json<UpdateScheduleRequest>,
) -> Result<Json<ScheduleResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;

    // Parse target type if provided
    let target_type = if let Some(ref target) = req.target {
        match target.target_type.as_str() {
            "workflow" => Some(ScheduleTargetType::Workflow),
            "activity" => Some(ScheduleTargetType::Activity),
            _ => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Invalid target type. Must be 'workflow' or 'activity'".to_string(),
                    }),
                ));
            }
        }
    } else {
        None
    };

    let update = UpdateSchedule {
        name: None, // Name cannot be updated
        description: req.description.map(Some),
        cron_expression: req.cron_expression,
        timezone: req.timezone,
        target_type,
        target_name: req.target.as_ref().map(|t| t.name.clone()),
        target_input: req.target.map(|t| t.input),
        enabled: req.enabled,
        max_concurrent: req.max_concurrent.map(Some),
        catch_up_missed: req.catch_up_missed,
        max_catch_up: req.max_catch_up.map(Some),
        retry_policy: req.retry_policy.map(Some),
        next_trigger_at: None, // Will be calculated based on cron
    };

    store
        .update_schedule(schedule_id, update)
        .await
        .map_err(|e| match e {
            StoreError::ScheduleNotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Schedule not found".to_string(),
                }),
            ),
            StoreError::InvalidCronExpression(msg) => (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid cron expression: {}", msg),
                }),
            ),
            _ => {
                tracing::error!("Failed to update schedule: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Failed to update schedule".to_string(),
                    }),
                )
            }
        })?;

    // Fetch updated schedule
    let schedule = store.get_schedule(schedule_id).await.map_err(|e| {
        tracing::error!("Failed to get updated schedule: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Failed to get updated schedule".to_string(),
            }),
        )
    })?;

    Ok(Json(ScheduleResponse::from(schedule)))
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
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn delete_schedule(
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;

    store
        .delete_schedule(schedule_id)
        .await
        .map_err(|e| match e {
            StoreError::ScheduleNotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Schedule not found".to_string(),
                }),
            ),
            _ => {
                tracing::error!("Failed to delete schedule: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Failed to delete schedule".to_string(),
                    }),
                )
            }
        })?;

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
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn pause_schedule(
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> Result<Json<ScheduleResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;

    let update = UpdateSchedule {
        enabled: Some(false),
        ..Default::default()
    };

    store
        .update_schedule(schedule_id, update)
        .await
        .map_err(|e| match e {
            StoreError::ScheduleNotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Schedule not found".to_string(),
                }),
            ),
            _ => {
                tracing::error!("Failed to pause schedule: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Failed to pause schedule".to_string(),
                    }),
                )
            }
        })?;

    // Clear next_trigger_at when paused
    store
        .update_next_trigger(schedule_id, Utc::now() + chrono::Duration::days(365 * 100))
        .await
        .ok(); // Ignore errors on this update

    let schedule = store.get_schedule(schedule_id).await.map_err(|e| {
        tracing::error!("Failed to get schedule: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Failed to get schedule".to_string(),
            }),
        )
    })?;

    Ok(Json(ScheduleResponse::from(schedule)))
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
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn resume_schedule(
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> Result<Json<ScheduleResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;

    // Get current schedule to get cron expression
    let current = store.get_schedule(schedule_id).await.map_err(|e| match e {
        StoreError::ScheduleNotFound(_) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Schedule not found".to_string(),
            }),
        ),
        _ => {
            tracing::error!("Failed to get schedule: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to get schedule".to_string(),
                }),
            )
        }
    })?;

    // Calculate next trigger time
    let next_trigger = calculate_next_trigger(&current.cron_expression).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to calculate next trigger: {}", e),
            }),
        )
    })?;

    // Enable and set next trigger
    let update = UpdateSchedule {
        enabled: Some(true),
        ..Default::default()
    };

    store
        .update_schedule(schedule_id, update)
        .await
        .map_err(|e| {
            tracing::error!("Failed to resume schedule: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to resume schedule".to_string(),
                }),
            )
        })?;

    if let Some(next) = next_trigger {
        store.update_next_trigger(schedule_id, next).await.ok();
    }

    let schedule = store.get_schedule(schedule_id).await.map_err(|e| {
        tracing::error!("Failed to get schedule: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Failed to get schedule".to_string(),
            }),
        )
    })?;

    Ok(Json(ScheduleResponse::from(schedule)))
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
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn trigger_schedule(
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> Result<Json<TriggerResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;

    // Get schedule to verify it exists
    let _schedule = store.get_schedule(schedule_id).await.map_err(|e| match e {
        StoreError::ScheduleNotFound(_) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Schedule not found".to_string(),
            }),
        ),
        _ => {
            tracing::error!("Failed to get schedule: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to get schedule".to_string(),
                }),
            )
        }
    })?;

    // Create execution record for manual trigger
    let execution_id = store
        .create_schedule_execution(schedule_id, Utc::now())
        .await
        .map_err(|e| {
            tracing::error!("Failed to create execution: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to trigger schedule".to_string(),
                }),
            )
        })?;

    // Note: The actual trigger will be picked up by the scheduler
    // For immediate execution, we'd need to invoke the scheduler directly

    Ok(Json(TriggerResponse { execution_id }))
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
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn list_schedule_executions(
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
    Query(query): Query<ListExecutionsQuery>,
) -> Result<Json<ScheduleExecutionsListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;

    // Verify schedule exists
    let _ = store.get_schedule(schedule_id).await.map_err(|e| match e {
        StoreError::ScheduleNotFound(_) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Schedule not found".to_string(),
            }),
        ),
        _ => {
            tracing::error!("Failed to get schedule: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to get schedule".to_string(),
                }),
            )
        }
    })?;

    let status = query.status.and_then(|s| match s.as_str() {
        "running" => Some(ScheduleExecutionStatus::Running),
        "completed" => Some(ScheduleExecutionStatus::Completed),
        "failed" => Some(ScheduleExecutionStatus::Failed),
        "skipped" => Some(ScheduleExecutionStatus::Skipped),
        _ => None,
    });

    let filter = ScheduleExecutionFilter {
        schedule_id: Some(schedule_id),
        status,
    };

    let pagination = Pagination {
        offset: query.offset.unwrap_or(0),
        limit: query.limit.unwrap_or(100),
    };

    let executions = store
        .list_schedule_executions(filter, pagination)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list executions: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to list executions".to_string(),
                }),
            )
        })?;

    let total = executions.len();
    let data: Vec<ScheduleExecutionResponse> = executions
        .into_iter()
        .map(ScheduleExecutionResponse::from)
        .collect();

    Ok(Json(ScheduleExecutionsListResponse { data, total }))
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
        (status = 404, description = "Execution not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn get_execution(
    State(state): State<ScheduleAppState>,
    Path(execution_id): Path<Uuid>,
) -> Result<Json<ScheduleExecutionResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;

    let execution = store
        .get_schedule_execution(execution_id)
        .await
        .map_err(|e| match e {
            StoreError::ScheduleExecutionNotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Execution not found".to_string(),
                }),
            ),
            _ => {
                tracing::error!("Failed to get execution: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Failed to get execution".to_string(),
                    }),
                )
            }
        })?;

    Ok(Json(ScheduleExecutionResponse::from(execution)))
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
        (status = 404, description = "Schedule not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable-schedules"
)]
pub async fn get_schedule_stats(
    State(state): State<ScheduleAppState>,
    Path(schedule_id): Path<Uuid>,
) -> Result<Json<ScheduleStatsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;

    // Verify schedule exists
    let _ = store.get_schedule(schedule_id).await.map_err(|e| match e {
        StoreError::ScheduleNotFound(_) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Schedule not found".to_string(),
            }),
        ),
        _ => {
            tracing::error!("Failed to get schedule: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to get schedule".to_string(),
                }),
            )
        }
    })?;

    let stats = store.get_schedule_stats(schedule_id).await.map_err(|e| {
        tracing::error!("Failed to get schedule stats: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Failed to get schedule stats".to_string(),
            }),
        )
    })?;

    Ok(Json(ScheduleStatsResponse::from(stats)))
}

// ============================================================================
// Helper functions
// ============================================================================

/// Calculate next trigger time from cron expression
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

// Durable Execution API routes
//
// Provides monitoring and management endpoints for the durable execution engine:
// - System health
// - Worker management
// - Workflow inspection
// - Task queue monitoring
// - Dead letter queue management
// - Circuit breaker status

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use everruns_durable::{
    CircuitBreakerState, CircuitState, DlqEntry, DlqFilter, Pagination, PostgresWorkflowEventStore,
    SystemHealth, TaskFilter, TaskInfo, TaskStatus, WorkerFilter, WorkerInfo, WorkflowEventInfo,
    WorkflowEventStore, WorkflowFilter, WorkflowInfoExtended, WorkflowSignal, WorkflowStatus,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::common::ErrorResponse;

/// App state for durable routes
#[derive(Clone)]
pub struct AppState {
    store: Option<Arc<PostgresWorkflowEventStore>>,
}

impl AppState {
    pub fn new(pool: Option<PgPool>) -> Self {
        Self {
            store: pool.map(|p| Arc::new(PostgresWorkflowEventStore::new(p))),
        }
    }

    /// Get the store, returning an error response if not available (dev mode)
    fn get_store(&self) -> Result<&Arc<PostgresWorkflowEventStore>, (StatusCode, Json<ErrorResponse>)> {
        self.store.as_ref().ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse {
                    error: "Durable execution not available in dev mode".to_string(),
                }),
            )
        })
    }
}

/// Create durable routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        // System health
        .route("/v1/durable/health", get(get_health))
        // Workers
        .route("/v1/durable/workers", get(list_workers))
        .route("/v1/durable/workers/:worker_id/drain", post(drain_worker))
        // Workflows
        .route("/v1/durable/workflows", get(list_workflows))
        .route("/v1/durable/workflows/:workflow_id", get(get_workflow))
        .route(
            "/v1/durable/workflows/:workflow_id/events",
            get(get_workflow_events),
        )
        .route(
            "/v1/durable/workflows/:workflow_id/cancel",
            post(cancel_workflow),
        )
        .route(
            "/v1/durable/workflows/:workflow_id/signal",
            post(send_signal),
        )
        // Tasks
        .route("/v1/durable/tasks", get(list_tasks))
        // DLQ
        .route("/v1/durable/dlq", get(list_dlq))
        .route("/v1/durable/dlq/:dlq_id/retry", post(retry_dlq))
        // Circuit breakers
        .route("/v1/durable/circuit-breakers", get(list_circuit_breakers))
        .with_state(state)
}

// ============================================================================
// Response types
// ============================================================================

/// System health response
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub total_workers: usize,
    pub active_workers: usize,
    pub workers_accepting: usize,
    pub total_capacity: usize,
    pub current_load: usize,
    pub load_percentage: f64,
    pub pending_tasks: usize,
    pub claimed_tasks: usize,
    pub running_workflows: usize,
    pub pending_workflows: usize,
    pub dlq_size: usize,
}

impl From<SystemHealth> for HealthResponse {
    fn from(h: SystemHealth) -> Self {
        let load_percentage = if h.total_capacity > 0 {
            (h.current_load as f64 / h.total_capacity as f64) * 100.0
        } else {
            0.0
        };

        let status = if h.active_workers == 0 {
            "degraded"
        } else if h.dlq_size > 0 {
            "warning"
        } else {
            "healthy"
        };

        Self {
            status: status.to_string(),
            total_workers: h.total_workers,
            active_workers: h.active_workers,
            workers_accepting: h.workers_accepting,
            total_capacity: h.total_capacity,
            current_load: h.current_load,
            load_percentage,
            pending_tasks: h.pending_tasks,
            claimed_tasks: h.claimed_tasks,
            running_workflows: h.running_workflows,
            pending_workflows: h.pending_workflows,
            dlq_size: h.dlq_size,
        }
    }
}

/// Worker response
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkerResponse {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_group: Option<String>,
    pub activity_types: Vec<String>,
    pub max_concurrency: u32,
    pub current_load: u32,
    pub status: String,
    pub accepting_tasks: bool,
    pub started_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
}

impl From<WorkerInfo> for WorkerResponse {
    fn from(w: WorkerInfo) -> Self {
        Self {
            id: w.id,
            worker_group: w.worker_group,
            activity_types: w.activity_types,
            max_concurrency: w.max_concurrency,
            current_load: w.current_load,
            status: w.status,
            accepting_tasks: w.accepting_tasks,
            started_at: w.started_at,
            last_heartbeat_at: w.last_heartbeat_at,
        }
    }
}

/// Workers list response
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkersListResponse {
    pub data: Vec<WorkerResponse>,
    pub total: usize,
}

/// Workflow response
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkflowResponse {
    pub id: Uuid,
    pub workflow_type: String,
    pub status: String,
    pub input: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

impl From<WorkflowInfoExtended> for WorkflowResponse {
    fn from(w: WorkflowInfoExtended) -> Self {
        Self {
            id: w.id,
            workflow_type: w.workflow_type,
            status: w.status.to_string(),
            input: w.input,
            result: w.result,
            error: w.error.map(|e| serde_json::to_value(e).unwrap_or_default()),
            created_at: w.created_at,
            started_at: w.started_at,
            completed_at: w.completed_at,
        }
    }
}

/// Workflows list response
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkflowsListResponse {
    pub data: Vec<WorkflowResponse>,
    pub total: usize,
}

/// Workflow event response
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkflowEventResponse {
    pub sequence_num: i32,
    pub event_type: String,
    pub event_data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl From<WorkflowEventInfo> for WorkflowEventResponse {
    fn from(e: WorkflowEventInfo) -> Self {
        Self {
            sequence_num: e.sequence_num,
            event_type: e.event_type,
            event_data: e.event_data,
            created_at: e.created_at,
        }
    }
}

/// Task response
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskResponse {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub activity_id: String,
    pub activity_type: String,
    pub status: String,
    pub priority: i32,
    pub attempt: u32,
    pub max_attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<DateTime<Utc>>,
}

impl From<TaskInfo> for TaskResponse {
    fn from(t: TaskInfo) -> Self {
        let status = match t.status {
            TaskStatus::Pending => "pending",
            TaskStatus::Claimed => "claimed",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Dead => "dead",
            TaskStatus::Cancelled => "cancelled",
        };

        Self {
            id: t.id,
            workflow_id: t.workflow_id,
            activity_id: t.activity_id,
            activity_type: t.activity_type,
            status: status.to_string(),
            priority: t.priority,
            attempt: t.attempt,
            max_attempts: t.max_attempts,
            claimed_by: t.claimed_by,
            last_error: t.last_error,
            created_at: t.created_at,
            claimed_at: t.claimed_at,
        }
    }
}

/// Tasks list response
#[derive(Debug, Serialize, ToSchema)]
pub struct TasksListResponse {
    pub data: Vec<TaskResponse>,
    pub total: usize,
}

/// DLQ entry response
#[derive(Debug, Serialize, ToSchema)]
pub struct DlqEntryResponse {
    pub id: Uuid,
    pub original_task_id: Uuid,
    pub workflow_id: Uuid,
    pub activity_id: String,
    pub activity_type: String,
    pub input: serde_json::Value,
    pub attempts: u32,
    pub last_error: String,
    pub error_history: Vec<String>,
    pub dead_at: DateTime<Utc>,
}

impl From<DlqEntry> for DlqEntryResponse {
    fn from(d: DlqEntry) -> Self {
        Self {
            id: d.id,
            original_task_id: d.original_task_id,
            workflow_id: d.workflow_id,
            activity_id: d.activity_id,
            activity_type: d.activity_type,
            input: d.input,
            attempts: d.attempts,
            last_error: d.last_error,
            error_history: d.error_history,
            dead_at: d.dead_at,
        }
    }
}

/// DLQ list response
#[derive(Debug, Serialize, ToSchema)]
pub struct DlqListResponse {
    pub data: Vec<DlqEntryResponse>,
    pub total: usize,
}

/// Circuit breaker response
#[derive(Debug, Serialize, ToSchema)]
pub struct CircuitBreakerResponse {
    pub key: String,
    pub state: String,
    pub failure_count: u32,
    pub success_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failure_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opened_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub half_open_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl From<CircuitBreakerState> for CircuitBreakerResponse {
    fn from(cb: CircuitBreakerState) -> Self {
        let state = match cb.state {
            CircuitState::Closed => "closed",
            CircuitState::Open => "open",
            CircuitState::HalfOpen => "half_open",
        };

        Self {
            key: cb.key,
            state: state.to_string(),
            failure_count: cb.failure_count,
            success_count: cb.success_count,
            last_failure_at: cb.last_failure_at,
            opened_at: cb.opened_at,
            half_open_at: cb.half_open_at,
            updated_at: cb.updated_at,
        }
    }
}

/// Circuit breakers list response
#[derive(Debug, Serialize, ToSchema)]
pub struct CircuitBreakersListResponse {
    pub circuit_breakers: Vec<CircuitBreakerResponse>,
    pub total: usize,
}

// ============================================================================
// Query parameters
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListWorkersQuery {
    pub status: Option<String>,
    pub worker_group: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListWorkflowsQuery {
    pub status: Option<String>,
    pub workflow_type: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    pub status: Option<String>,
    pub activity_type: Option<String>,
    pub workflow_id: Option<Uuid>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ListDlqQuery {
    pub workflow_id: Option<Uuid>,
    pub activity_type: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

/// Request to send a signal to a workflow
#[derive(Debug, Deserialize, ToSchema)]
pub struct SendSignalRequest {
    pub signal_type: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

// ============================================================================
// Route handlers
// ============================================================================

/// GET /v1/durable/health - Get system health
#[utoipa::path(
    get,
    path = "/v1/durable/health",
    responses(
        (status = 200, description = "System health", body = HealthResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn get_health(
    State(state): State<AppState>,
) -> Result<Json<HealthResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    let health = store.get_system_health().await.map_err(|e| {
        tracing::error!("Failed to get system health: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal server error".to_string(),
            }),
        )
    })?;

    Ok(Json(HealthResponse::from(health)))
}

/// GET /v1/durable/workers - List workers
#[utoipa::path(
    get,
    path = "/v1/durable/workers",
    params(
        ("status" = Option<String>, Query, description = "Filter by status"),
        ("worker_group" = Option<String>, Query, description = "Filter by worker group")
    ),
    responses(
        (status = 200, description = "List of workers", body = WorkersListResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn list_workers(
    State(state): State<AppState>,
    Query(query): Query<ListWorkersQuery>,
) -> Result<Json<WorkersListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    let filter = WorkerFilter {
        status: query.status,
        worker_group: query.worker_group,
    };

    let workers = store.list_workers(filter).await.map_err(|e| {
        tracing::error!("Failed to list workers: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal server error".to_string(),
            }),
        )
    })?;

    let total = workers.len();
    let data: Vec<WorkerResponse> = workers.into_iter().map(WorkerResponse::from).collect();

    Ok(Json(WorkersListResponse { data, total }))
}

/// POST /v1/durable/workers/:worker_id/drain - Drain a worker
#[utoipa::path(
    post,
    path = "/v1/durable/workers/{worker_id}/drain",
    params(
        ("worker_id" = String, Path, description = "Worker ID")
    ),
    responses(
        (status = 200, description = "Worker drained"),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn drain_worker(
    State(state): State<AppState>,
    Path(worker_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    store.drain_worker(&worker_id).await.map_err(|e| {
        tracing::error!("Failed to drain worker: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal server error".to_string(),
            }),
        )
    })?;

    Ok(StatusCode::OK)
}

/// GET /v1/durable/workflows - List workflows
#[utoipa::path(
    get,
    path = "/v1/durable/workflows",
    params(
        ("status" = Option<String>, Query, description = "Filter by status"),
        ("workflow_type" = Option<String>, Query, description = "Filter by workflow type"),
        ("offset" = Option<u32>, Query, description = "Pagination offset"),
        ("limit" = Option<u32>, Query, description = "Pagination limit")
    ),
    responses(
        (status = 200, description = "List of workflows", body = WorkflowsListResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn list_workflows(
    State(state): State<AppState>,
    Query(query): Query<ListWorkflowsQuery>,
) -> Result<Json<WorkflowsListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    let status = query.status.and_then(|s| match s.as_str() {
        "pending" => Some(WorkflowStatus::Pending),
        "running" => Some(WorkflowStatus::Running),
        "completed" => Some(WorkflowStatus::Completed),
        "failed" => Some(WorkflowStatus::Failed),
        "cancelled" => Some(WorkflowStatus::Cancelled),
        _ => None,
    });

    let filter = WorkflowFilter {
        status,
        workflow_type: query.workflow_type,
    };

    let pagination = Pagination {
        offset: query.offset.unwrap_or(0),
        limit: query.limit.unwrap_or(100),
    };

    let workflows = store
        .list_workflows(filter, pagination)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list workflows: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;

    let total = workflows.len();
    let data: Vec<WorkflowResponse> = workflows.into_iter().map(WorkflowResponse::from).collect();

    Ok(Json(WorkflowsListResponse { data, total }))
}

/// GET /v1/durable/workflows/:workflow_id - Get workflow details
#[utoipa::path(
    get,
    path = "/v1/durable/workflows/{workflow_id}",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID")
    ),
    responses(
        (status = 200, description = "Workflow details", body = WorkflowResponse),
        (status = 404, description = "Workflow not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn get_workflow(
    State(state): State<AppState>,
    Path(workflow_id): Path<Uuid>,
) -> Result<Json<WorkflowResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    // Use list_workflows with a filter to get the extended info
    let filter = WorkflowFilter::default();
    let pagination = Pagination {
        offset: 0,
        limit: 1000,
    };

    let workflows = store
        .list_workflows(filter, pagination)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get workflow: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;

    let workflow = workflows.into_iter().find(|w| w.id == workflow_id).ok_or((
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "Workflow not found".to_string(),
        }),
    ))?;

    Ok(Json(WorkflowResponse::from(workflow)))
}

/// GET /v1/durable/workflows/:workflow_id/events - Get workflow events
#[utoipa::path(
    get,
    path = "/v1/durable/workflows/{workflow_id}/events",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID")
    ),
    responses(
        (status = 200, description = "Workflow events", body = Vec<WorkflowEventResponse>),
        (status = 404, description = "Workflow not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn get_workflow_events(
    State(state): State<AppState>,
    Path(workflow_id): Path<Uuid>,
) -> Result<Json<Vec<WorkflowEventResponse>>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    let events = store
        .get_workflow_events(workflow_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get workflow events: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;

    let events: Vec<WorkflowEventResponse> = events
        .into_iter()
        .map(WorkflowEventResponse::from)
        .collect();

    Ok(Json(events))
}

/// POST /v1/durable/workflows/:workflow_id/cancel - Cancel a workflow
#[utoipa::path(
    post,
    path = "/v1/durable/workflows/{workflow_id}/cancel",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID")
    ),
    responses(
        (status = 200, description = "Workflow cancelled"),
        (status = 404, description = "Workflow not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn cancel_workflow(
    State(state): State<AppState>,
    Path(workflow_id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    store
        .cancel_workflow(workflow_id)
        .await
        .map_err(|e| match e {
            everruns_durable::StoreError::WorkflowNotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Workflow not found".to_string(),
                }),
            ),
            _ => {
                tracing::error!("Failed to cancel workflow: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal server error".to_string(),
                    }),
                )
            }
        })?;

    Ok(StatusCode::OK)
}

/// POST /v1/durable/workflows/:workflow_id/signal - Send signal to workflow
#[utoipa::path(
    post,
    path = "/v1/durable/workflows/{workflow_id}/signal",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID")
    ),
    request_body = SendSignalRequest,
    responses(
        (status = 200, description = "Signal sent"),
        (status = 404, description = "Workflow not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn send_signal(
    State(state): State<AppState>,
    Path(workflow_id): Path<Uuid>,
    Json(req): Json<SendSignalRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    let signal = WorkflowSignal {
        signal_type: req.signal_type,
        payload: req.payload,
        sent_at: Utc::now(),
    };

    store
        .send_signal(workflow_id, signal)
        .await
        .map_err(|e| {
            tracing::error!("Failed to send signal: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;

    Ok(StatusCode::OK)
}

/// GET /v1/durable/tasks - List tasks
#[utoipa::path(
    get,
    path = "/v1/durable/tasks",
    params(
        ("status" = Option<String>, Query, description = "Filter by status"),
        ("activity_type" = Option<String>, Query, description = "Filter by activity type"),
        ("workflow_id" = Option<Uuid>, Query, description = "Filter by workflow ID"),
        ("offset" = Option<u32>, Query, description = "Pagination offset"),
        ("limit" = Option<u32>, Query, description = "Pagination limit")
    ),
    responses(
        (status = 200, description = "List of tasks", body = TasksListResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn list_tasks(
    State(state): State<AppState>,
    Query(query): Query<ListTasksQuery>,
) -> Result<Json<TasksListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    let status = query.status.and_then(|s| match s.as_str() {
        "pending" => Some(TaskStatus::Pending),
        "claimed" => Some(TaskStatus::Claimed),
        "completed" => Some(TaskStatus::Completed),
        "failed" => Some(TaskStatus::Failed),
        "dead" => Some(TaskStatus::Dead),
        "cancelled" => Some(TaskStatus::Cancelled),
        _ => None,
    });

    let filter = TaskFilter {
        status,
        activity_type: query.activity_type,
        workflow_id: query.workflow_id,
    };

    let pagination = Pagination {
        offset: query.offset.unwrap_or(0),
        limit: query.limit.unwrap_or(100),
    };

    let tasks = store
        .list_tasks(filter, pagination)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list tasks: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;

    let total = tasks.len();
    let data: Vec<TaskResponse> = tasks.into_iter().map(TaskResponse::from).collect();

    Ok(Json(TasksListResponse { data, total }))
}

/// GET /v1/durable/dlq - List dead letter queue entries
#[utoipa::path(
    get,
    path = "/v1/durable/dlq",
    params(
        ("workflow_id" = Option<Uuid>, Query, description = "Filter by workflow ID"),
        ("activity_type" = Option<String>, Query, description = "Filter by activity type"),
        ("offset" = Option<u32>, Query, description = "Pagination offset"),
        ("limit" = Option<u32>, Query, description = "Pagination limit")
    ),
    responses(
        (status = 200, description = "List of DLQ entries", body = DlqListResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn list_dlq(
    State(state): State<AppState>,
    Query(query): Query<ListDlqQuery>,
) -> Result<Json<DlqListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    let filter = DlqFilter {
        workflow_id: query.workflow_id,
        activity_type: query.activity_type,
    };

    let pagination = Pagination {
        offset: query.offset.unwrap_or(0),
        limit: query.limit.unwrap_or(100),
    };

    let entries = store
        .list_dlq(filter, pagination)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list DLQ: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;

    let total = entries.len();
    let data: Vec<DlqEntryResponse> = entries.into_iter().map(DlqEntryResponse::from).collect();

    Ok(Json(DlqListResponse { data, total }))
}

/// POST /v1/durable/dlq/:dlq_id/retry - Retry a DLQ entry
#[utoipa::path(
    post,
    path = "/v1/durable/dlq/{dlq_id}/retry",
    params(
        ("dlq_id" = Uuid, Path, description = "DLQ entry ID")
    ),
    responses(
        (status = 200, description = "Task requeued", body = Uuid),
        (status = 404, description = "DLQ entry not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn retry_dlq(
    State(state): State<AppState>,
    Path(dlq_id): Path<Uuid>,
) -> Result<Json<Uuid>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    let task_id = store
        .requeue_from_dlq(dlq_id)
        .await
        .map_err(|e| match e {
            everruns_durable::StoreError::TaskNotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "DLQ entry not found".to_string(),
                }),
            ),
            _ => {
                tracing::error!("Failed to retry DLQ entry: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal server error".to_string(),
                    }),
                )
            }
        })?;

    Ok(Json(task_id))
}

/// GET /v1/durable/circuit-breakers - List circuit breakers
#[utoipa::path(
    get,
    path = "/v1/durable/circuit-breakers",
    responses(
        (status = 200, description = "List of circuit breakers", body = CircuitBreakersListResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn list_circuit_breakers(
    State(state): State<AppState>,
) -> Result<Json<CircuitBreakersListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    let circuit_breakers = store.list_circuit_breakers().await.map_err(|e| {
        tracing::error!("Failed to list circuit breakers: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal server error".to_string(),
            }),
        )
    })?;

    let total = circuit_breakers.len();
    let circuit_breakers: Vec<CircuitBreakerResponse> = circuit_breakers
        .into_iter()
        .map(CircuitBreakerResponse::from)
        .collect();

    Ok(Json(CircuitBreakersListResponse {
        circuit_breakers,
        total,
    }))
}

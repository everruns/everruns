// Durable Execution API routes
//
// Provides monitoring and management endpoints for the durable execution engine:
// - System health
// - Worker management
// - Workflow inspection
// - Task queue monitoring
// - Dead letter queue management
// - Circuit breaker status
// - SSE streaming for real-time updates

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use everruns_durable::{
    CircuitBreakerState, CircuitState, DlqEntry, DlqFilter, Pagination, SystemHealth, TaskFilter,
    TaskInfo, TaskStatus, WorkerFilter, WorkerInfo, WorkflowEventInfo, WorkflowEventStore,
    WorkflowFilter, WorkflowInfoExtended, WorkflowSignal, WorkflowStatus,
};
use futures::{
    StreamExt,
    stream::{self, Stream},
};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tokio::time::Instant;
use utoipa::ToSchema;
use uuid::Uuid;

use super::common::ErrorResponse;
use super::sse::{DisconnectReason, SseStreamConfig};

/// App state for durable routes
#[derive(Clone)]
pub struct AppState {
    store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>,
    metrics: MetricsCollector,
}

impl AppState {
    /// Create new state with an optional workflow event store
    ///
    /// Collector holds 360 points = 1 hour at 10s intervals.
    pub fn new(store: Option<Arc<dyn WorkflowEventStore + Send + Sync>>) -> Self {
        Self {
            store,
            metrics: MetricsCollector::new(360),
        }
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

    /// Spawn background task that samples health metrics every 10 seconds
    pub fn spawn_metrics_sampler(&self) {
        let store = self.store.clone();
        let metrics = self.metrics.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));

            tracing::info!(
                "Started metrics sampler background task (10s interval, 360 max points)"
            );

            loop {
                interval.tick().await;

                let point = match &store {
                    Some(s) => {
                        let health = match s.get_system_health().await {
                            Ok(h) => h,
                            Err(e) => {
                                tracing::error!("Metrics sampler: failed to get health: {}", e);
                                continue;
                            }
                        };

                        let load_percentage = if health.total_capacity > 0 {
                            (health.current_load as f64 / health.total_capacity as f64) * 100.0
                        } else {
                            0.0
                        };

                        MetricsPoint {
                            timestamp: Utc::now(),
                            running_workflows: health.running_workflows,
                            pending_workflows: health.pending_workflows,
                            pending_tasks: health.pending_tasks,
                            claimed_tasks: health.claimed_tasks,
                            active_workers: health.active_workers,
                            load_percentage,
                            dlq_size: health.dlq_size,
                            // Use DB-based cumulative counts (reliable even when workers go stale)
                            tasks_completed_total: health.completed_tasks as u64,
                            tasks_failed_total: health.failed_tasks as u64,
                            tasks_started_total: health.started_tasks as u64,
                            workflows_completed_total: health.completed_workflows as u64,
                            workflows_failed_total: health.failed_workflows as u64,
                            workflows_started_total: health.started_workflows as u64,
                        }
                    }
                    None => {
                        // Dev mode without store: push zeroed data
                        MetricsPoint {
                            timestamp: Utc::now(),
                            running_workflows: 0,
                            pending_workflows: 0,
                            pending_tasks: 0,
                            claimed_tasks: 0,
                            active_workers: 0,
                            load_percentage: 0.0,
                            dlq_size: 0,
                            tasks_completed_total: 0,
                            tasks_failed_total: 0,
                            tasks_started_total: 0,
                            workflows_completed_total: 0,
                            workflows_failed_total: 0,
                            workflows_started_total: 0,
                        }
                    }
                };

                metrics.push(point).await;
            }
        });
    }
}

/// Create durable routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        // SSE streaming
        .route("/v1/durable/sse", get(stream_durable_sse))
        .route(
            "/v1/durable/workflows/{workflow_id}/sse",
            get(stream_workflow_sse),
        )
        // System health
        .route("/v1/durable/health", get(get_health))
        // Workers
        .route("/v1/durable/workers", get(list_workers))
        .route("/v1/durable/workers/{worker_id}/drain", post(drain_worker))
        .route(
            "/v1/durable/workers/{worker_id}/resume",
            post(resume_worker),
        )
        // Workflows
        .route("/v1/durable/workflows", get(list_workflows))
        .route("/v1/durable/workflows/{workflow_id}", get(get_workflow))
        .route(
            "/v1/durable/workflows/{workflow_id}/events",
            get(get_workflow_events),
        )
        .route(
            "/v1/durable/workflows/{workflow_id}/cancel",
            post(cancel_workflow),
        )
        .route(
            "/v1/durable/workflows/{workflow_id}/signal",
            post(send_signal),
        )
        // Tasks
        .route("/v1/durable/tasks", get(list_tasks))
        // DLQ
        .route("/v1/durable/dlq", get(list_dlq))
        .route("/v1/durable/dlq/{dlq_id}/retry", post(retry_dlq))
        // Metrics
        .route(
            "/v1/durable/metrics/timeseries",
            get(get_metrics_timeseries),
        )
        // Circuit breakers
        .route("/v1/durable/circuit-breakers", get(list_circuit_breakers))
        .route(
            "/v1/durable/circuit-breakers/{key}",
            get(get_circuit_breaker),
        )
        .route(
            "/v1/durable/circuit-breakers/{key}/open",
            post(force_open_circuit_breaker),
        )
        .route(
            "/v1/durable/circuit-breakers/{key}/close",
            post(force_close_circuit_breaker),
        )
        .route(
            "/v1/durable/circuit-breakers/{key}",
            axum::routing::delete(delete_circuit_breaker),
        )
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
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub started_tasks: usize,
    pub running_workflows: usize,
    pub pending_workflows: usize,
    pub completed_workflows: usize,
    pub failed_workflows: usize,
    pub started_workflows: usize,
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
            completed_tasks: h.completed_tasks,
            failed_tasks: h.failed_tasks,
            started_tasks: h.started_tasks,
            running_workflows: h.running_workflows,
            pending_workflows: h.pending_workflows,
            completed_workflows: h.completed_workflows,
            failed_workflows: h.failed_workflows,
            started_workflows: h.started_workflows,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backpressure_reason: Option<String>,
    pub started_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// Total tasks completed by this worker
    pub tasks_completed: u64,
    /// Total tasks failed by this worker
    pub tasks_failed: u64,
    /// Average task duration in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_task_duration_ms: Option<u64>,
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
            backpressure_reason: w.backpressure_reason,
            started_at: w.started_at,
            last_heartbeat_at: w.last_heartbeat_at,
            hostname: w.hostname,
            version: w.version,
            metadata: w.metadata,
            tasks_completed: w.tasks_completed,
            tasks_failed: w.tasks_failed,
            avg_task_duration_ms: w.avg_task_duration_ms,
        }
    }
}

/// Workers summary stats
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkersSummaryResponse {
    pub active: usize,
    pub draining: usize,
    pub stopped: usize,
    pub total_capacity: usize,
    pub total_load: usize,
}

/// Workers list response
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkersListResponse {
    pub data: Vec<WorkerResponse>,
    pub total: usize,
    pub summary: WorkersSummaryResponse,
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
    pub id: i64,
    pub workflow_id: Uuid,
    pub sequence_num: i32,
    pub event_type: String,
    pub event_data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl From<WorkflowEventInfo> for WorkflowEventResponse {
    fn from(e: WorkflowEventInfo) -> Self {
        Self {
            id: e.id,
            workflow_id: e.workflow_id,
            sequence_num: e.sequence_num,
            event_type: e.event_type,
            event_data: e.event_data,
            created_at: e.created_at,
        }
    }
}

/// Workflow events list response
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkflowEventsListResponse {
    pub data: Vec<WorkflowEventResponse>,
    pub total: usize,
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
    pub data: Vec<CircuitBreakerResponse>,
    pub total: usize,
}

/// Single metrics data point
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MetricsPoint {
    pub timestamp: DateTime<Utc>,
    // Workflow gauges
    pub running_workflows: usize,
    pub pending_workflows: usize,
    // Task gauges
    pub pending_tasks: usize,
    pub claimed_tasks: usize,
    // System gauges
    pub active_workers: usize,
    pub load_percentage: f64,
    pub dlq_size: usize,
    // Cumulative totals (for computing deltas / rates on frontend)
    pub tasks_completed_total: u64,
    pub tasks_failed_total: u64,
    pub tasks_started_total: u64,
    pub workflows_completed_total: u64,
    pub workflows_failed_total: u64,
    pub workflows_started_total: u64,
}

/// Metrics time series response
#[derive(Debug, Serialize, ToSchema)]
pub struct MetricsTimeSeriesResponse {
    pub points: Vec<MetricsPoint>,
    pub resolution_seconds: u64,
}

/// Background metrics collector maintaining a ring buffer of timestamped health snapshots
#[derive(Clone)]
pub struct MetricsCollector {
    points: Arc<RwLock<VecDeque<MetricsPoint>>>,
    max_points: usize,
}

impl MetricsCollector {
    pub fn new(max_points: usize) -> Self {
        Self {
            points: Arc::new(RwLock::new(VecDeque::with_capacity(max_points))),
            max_points,
        }
    }

    /// Push a new metrics point, removing oldest if at capacity
    pub async fn push(&self, point: MetricsPoint) {
        let mut points = self.points.write().await;
        if points.len() >= self.max_points {
            points.pop_front();
        }
        points.push_back(point);
    }

    /// Get all points
    pub async fn get_points(&self) -> Vec<MetricsPoint> {
        self.points.read().await.iter().cloned().collect()
    }
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

/// GET /v1/durable/metrics/timeseries - Get metrics time series
#[utoipa::path(
    get,
    path = "/v1/durable/metrics/timeseries",
    responses(
        (status = 200, description = "Metrics time series", body = MetricsTimeSeriesResponse),
    ),
    tag = "durable"
)]
pub async fn get_metrics_timeseries(
    State(state): State<AppState>,
) -> Result<Json<MetricsTimeSeriesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let points = state.metrics.get_points().await;
    Ok(Json(MetricsTimeSeriesResponse {
        points,
        resolution_seconds: 10,
    }))
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

    let summary = WorkersSummaryResponse {
        active: data.iter().filter(|w| w.status == "active").count(),
        draining: data.iter().filter(|w| w.status == "draining").count(),
        stopped: data.iter().filter(|w| w.status == "stopped").count(),
        total_capacity: data
            .iter()
            .filter(|w| w.status == "active")
            .map(|w| w.max_concurrency as usize)
            .sum(),
        total_load: data
            .iter()
            .filter(|w| w.status == "active")
            .map(|w| w.current_load as usize)
            .sum(),
    };

    Ok(Json(WorkersListResponse {
        data,
        total,
        summary,
    }))
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

/// POST /v1/durable/workers/:worker_id/resume - Resume a draining worker
#[utoipa::path(
    post,
    path = "/v1/durable/workers/{worker_id}/resume",
    params(
        ("worker_id" = String, Path, description = "Worker ID")
    ),
    responses(
        (status = 200, description = "Worker resumed"),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn resume_worker(
    State(state): State<AppState>,
    Path(worker_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    store.resume_worker(&worker_id).await.map_err(|e| {
        tracing::error!("Failed to resume worker: {}", e);
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
        (status = 200, description = "List of workflow events", body = WorkflowEventsListResponse),
        (status = 404, description = "Workflow not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn get_workflow_events(
    State(state): State<AppState>,
    Path(workflow_id): Path<Uuid>,
) -> Result<Json<WorkflowEventsListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    let events = store.get_workflow_events(workflow_id).await.map_err(|e| {
        tracing::error!("Failed to get workflow events: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal server error".to_string(),
            }),
        )
    })?;

    let total = events.len();
    let data: Vec<WorkflowEventResponse> = events
        .into_iter()
        .map(WorkflowEventResponse::from)
        .collect();

    Ok(Json(WorkflowEventsListResponse { data, total }))
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

    store.send_signal(workflow_id, signal).await.map_err(|e| {
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

    let tasks = store.list_tasks(filter, pagination).await.map_err(|e| {
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

    let entries = store.list_dlq(filter, pagination).await.map_err(|e| {
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
    let task_id = store.requeue_from_dlq(dlq_id).await.map_err(|e| match e {
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
    let data: Vec<CircuitBreakerResponse> = circuit_breakers
        .into_iter()
        .map(CircuitBreakerResponse::from)
        .collect();

    Ok(Json(CircuitBreakersListResponse { data, total }))
}

/// GET /v1/durable/circuit-breakers/:key - Get a single circuit breaker
#[utoipa::path(
    get,
    path = "/v1/durable/circuit-breakers/{key}",
    params(
        ("key" = String, Path, description = "Circuit breaker key")
    ),
    responses(
        (status = 200, description = "Circuit breaker details", body = CircuitBreakerResponse),
        (status = 404, description = "Circuit breaker not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn get_circuit_breaker(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<CircuitBreakerResponse>, (StatusCode, Json<ErrorResponse>)> {
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

    let breaker = circuit_breakers
        .into_iter()
        .find(|cb| cb.key == key)
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Circuit breaker not found".to_string(),
            }),
        ))?;

    Ok(Json(CircuitBreakerResponse::from(breaker)))
}

/// POST /v1/durable/circuit-breakers/:key/open - Force open a circuit breaker
#[utoipa::path(
    post,
    path = "/v1/durable/circuit-breakers/{key}/open",
    params(
        ("key" = String, Path, description = "Circuit breaker key")
    ),
    responses(
        (status = 200, description = "Circuit breaker opened"),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn force_open_circuit_breaker(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    store.force_open_circuit_breaker(&key).await.map_err(|e| {
        tracing::error!("Failed to force open circuit breaker: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal server error".to_string(),
            }),
        )
    })?;

    tracing::info!(key = %key, "Force opened circuit breaker");
    Ok(StatusCode::OK)
}

/// POST /v1/durable/circuit-breakers/:key/close - Force close a circuit breaker
#[utoipa::path(
    post,
    path = "/v1/durable/circuit-breakers/{key}/close",
    params(
        ("key" = String, Path, description = "Circuit breaker key")
    ),
    responses(
        (status = 200, description = "Circuit breaker closed"),
        (status = 404, description = "Circuit breaker not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn force_close_circuit_breaker(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    store
        .force_close_circuit_breaker(&key)
        .await
        .map_err(|e| match e {
            everruns_durable::StoreError::CircuitBreakerNotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Circuit breaker not found".to_string(),
                }),
            ),
            _ => {
                tracing::error!("Failed to force close circuit breaker: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal server error".to_string(),
                    }),
                )
            }
        })?;

    tracing::info!(key = %key, "Force closed circuit breaker");
    Ok(StatusCode::OK)
}

/// DELETE /v1/durable/circuit-breakers/:key - Delete/reset a circuit breaker
#[utoipa::path(
    delete,
    path = "/v1/durable/circuit-breakers/{key}",
    params(
        ("key" = String, Path, description = "Circuit breaker key")
    ),
    responses(
        (status = 200, description = "Circuit breaker deleted"),
        (status = 404, description = "Circuit breaker not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "durable"
)]
pub async fn delete_circuit_breaker(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let store = state.get_store()?;
    store
        .delete_circuit_breaker(&key)
        .await
        .map_err(|e| match e {
            everruns_durable::StoreError::CircuitBreakerNotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Circuit breaker not found".to_string(),
                }),
            ),
            _ => {
                tracing::error!("Failed to delete circuit breaker: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal server error".to_string(),
                    }),
                )
            }
        })?;

    tracing::info!(key = %key, "Deleted circuit breaker");
    Ok(StatusCode::OK)
}

// ============================================================================
// SSE Streaming
// ============================================================================

/// SSE snapshot data for global durable state
#[derive(Debug, Serialize)]
struct DurableSnapshot {
    health: HealthResponse,
    workers: Vec<WorkerResponse>,
    workflows: WorkflowsListResponse,
    tasks: TasksListResponse,
    dlq: DlqListResponse,
    circuit_breakers: CircuitBreakersListResponse,
    metrics_history: Vec<MetricsPoint>,
}

/// SSE snapshot data for a single workflow
#[derive(Debug, Serialize)]
struct WorkflowSnapshot {
    workflow: WorkflowResponse,
    events: Vec<WorkflowEventResponse>,
}

/// GET /v1/durable/sse - Stream global durable state (SSE)
///
/// Establishes a Server-Sent Events (SSE) connection for real-time durable system monitoring.
///
/// ## Connection Lifecycle Events
///
/// - **connected**: Sent immediately when the stream is established.
/// - **snapshot**: Sent when system state changes (health, workers, workflows, tasks, DLQ, circuit breakers).
/// - **disconnecting**: Sent before the server closes the connection for graceful cycling.
///   Data: `{"reason":"connection_cycle","retry_ms":1000}`
///
/// ## Connection Cycling
///
/// Connections are automatically cycled every 10 minutes. Before closing, the server sends
/// a `disconnecting` event so clients can reconnect seamlessly.
///
/// ## Retry Hints
///
/// Each SSE event includes a `retry:` field (in milliseconds) that hints reconnection timing:
/// - During active updates: 1000ms
/// - During idle periods: increases with backoff up to 20000ms
/// - After `disconnecting` event: 1000ms
#[utoipa::path(
    get,
    path = "/v1/durable/sse",
    responses(
        (status = 200, description = "SSE event stream with 'connected', 'snapshot', and 'disconnecting' events", content_type = "text/event-stream"),
        (status = 503, description = "Durable store not available")
    ),
    tag = "durable"
)]
pub async fn stream_durable_sse(
    State(state): State<AppState>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, StatusCode> {
    // Verify store is available
    let _ = state
        .get_store()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    tracing::info!("Starting global durable SSE stream");

    // Use monitoring config (relaxed polling for dashboards)
    let config = SseStreamConfig::monitoring();
    let connection_start = Instant::now();

    // Stream state machine
    #[derive(Clone)]
    enum StreamPhase {
        SendConnected,
        Polling,
        SendDisconnecting,
        Closed,
    }

    #[derive(Clone)]
    struct StreamState {
        phase: StreamPhase,
        backoff_ms: u64,
        config: SseStreamConfig,
        connection_start: Instant,
        /// Jittered max connection duration (prevents thundering herd reconnections)
        max_duration: std::time::Duration,
        // Track last snapshot hash to detect changes
        last_hash: Option<u64>,
        // Reason for disconnection (used when phase is SendDisconnecting)
        disconnect_reason: DisconnectReason,
    }

    let initial_state = StreamState {
        phase: StreamPhase::SendConnected,
        backoff_ms: config.min_backoff_ms,
        max_duration: config.jittered_max_connection_duration(),
        config,
        connection_start,
        last_hash: None,
        disconnect_reason: DisconnectReason::ConnectionCycle,
    };

    let stream = stream::unfold((state, initial_state), |(state, stream_state)| async move {
        match stream_state.phase {
            StreamPhase::Closed => None,

            StreamPhase::SendDisconnecting => {
                tracing::info!(
                    duration_secs = stream_state.connection_start.elapsed().as_secs(),
                    reason = stream_state.disconnect_reason.as_str(),
                    "Durable SSE: sending disconnecting event"
                );
                let disconnect_data = format!(
                    r#"{{"reason":"{}","retry_ms":{}}}"#,
                    stream_state.disconnect_reason.as_str(),
                    stream_state.config.disconnect_retry_ms
                );
                let disconnecting_event = Ok(SseEvent::default()
                    .event("disconnecting")
                    .data(disconnect_data)
                    .retry(stream_state.config.disconnect_retry()));

                let new_state = StreamState {
                    phase: StreamPhase::Closed,
                    ..stream_state
                };
                Some((stream::iter(vec![disconnecting_event]), (state, new_state)))
            }

            StreamPhase::SendConnected => {
                tracing::debug!("Durable SSE: sending connected event");
                let connected_event = Ok(SseEvent::default()
                    .event("connected")
                    .data(r#"{"status":"connected"}"#)
                    .retry(stream_state.config.retry_hint(stream_state.backoff_ms)));
                let new_state = StreamState {
                    phase: StreamPhase::Polling,
                    ..stream_state
                };
                Some((stream::iter(vec![connected_event]), (state, new_state)))
            }

            StreamPhase::Polling => {
                // Helper macro to handle errors with graceful disconnect
                macro_rules! fetch_or_disconnect {
                    ($result:expr, $msg:expr) => {
                        match $result {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::error!("{}: {}", $msg, e);
                                let new_state = StreamState {
                                    phase: StreamPhase::SendDisconnecting,
                                    disconnect_reason: DisconnectReason::Error,
                                    ..stream_state
                                };
                                return Some((stream::iter(vec![]), (state, new_state)));
                            }
                        }
                    };
                }

                // Check for connection cycling (uses jittered duration)
                if stream_state.connection_start.elapsed() > stream_state.max_duration {
                    let new_state = StreamState {
                        phase: StreamPhase::SendDisconnecting,
                        disconnect_reason: DisconnectReason::ConnectionCycle,
                        ..stream_state
                    };
                    return Some((stream::iter(vec![]), (state, new_state)));
                }

                // Fetch current state
                let store = match state.get_store() {
                    Ok(s) => s,
                    Err(_) => {
                        tracing::error!("Durable store not available");
                        let new_state = StreamState {
                            phase: StreamPhase::SendDisconnecting,
                            disconnect_reason: DisconnectReason::Error,
                            ..stream_state
                        };
                        return Some((stream::iter(vec![]), (state, new_state)));
                    }
                };

                // Fetch all data - disconnect gracefully on any error
                let health = HealthResponse::from(fetch_or_disconnect!(
                    store.get_system_health().await,
                    "Failed to fetch health"
                ));

                let workers: Vec<WorkerResponse> = fetch_or_disconnect!(
                    store.list_workers(WorkerFilter::default()).await,
                    "Failed to fetch workers"
                )
                .into_iter()
                .map(WorkerResponse::from)
                .collect();

                let workflows_data = fetch_or_disconnect!(
                    store
                        .list_workflows(
                            WorkflowFilter::default(),
                            Pagination {
                                offset: 0,
                                limit: 100,
                            },
                        )
                        .await,
                    "Failed to fetch workflows"
                );
                let workflows = WorkflowsListResponse {
                    total: workflows_data.len(),
                    data: workflows_data
                        .into_iter()
                        .map(WorkflowResponse::from)
                        .collect(),
                };

                let tasks_data = fetch_or_disconnect!(
                    store
                        .list_tasks(
                            TaskFilter::default(),
                            Pagination {
                                offset: 0,
                                limit: 100,
                            },
                        )
                        .await,
                    "Failed to fetch tasks"
                );
                let tasks = TasksListResponse {
                    total: tasks_data.len(),
                    data: tasks_data.into_iter().map(TaskResponse::from).collect(),
                };

                let dlq_data = fetch_or_disconnect!(
                    store
                        .list_dlq(
                            DlqFilter::default(),
                            Pagination {
                                offset: 0,
                                limit: 100,
                            },
                        )
                        .await,
                    "Failed to fetch DLQ"
                );
                let dlq = DlqListResponse {
                    total: dlq_data.len(),
                    data: dlq_data.into_iter().map(DlqEntryResponse::from).collect(),
                };

                let cb_data = fetch_or_disconnect!(
                    store.list_circuit_breakers().await,
                    "Failed to fetch circuit breakers"
                );
                let circuit_breakers = CircuitBreakersListResponse {
                    total: cb_data.len(),
                    data: cb_data
                        .into_iter()
                        .map(CircuitBreakerResponse::from)
                        .collect(),
                };

                // Fetch metrics history (always succeeds, no store needed)
                let metrics_history = state.metrics.get_points().await;

                let snapshot = DurableSnapshot {
                    health,
                    workers,
                    workflows,
                    tasks,
                    dlq,
                    circuit_breakers,
                    metrics_history,
                };

                // Simple hash based on key metrics to detect changes
                let current_hash = {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    snapshot.health.status.hash(&mut hasher);
                    snapshot.health.active_workers.hash(&mut hasher);
                    snapshot.health.running_workflows.hash(&mut hasher);
                    snapshot.health.pending_tasks.hash(&mut hasher);
                    snapshot.health.claimed_tasks.hash(&mut hasher);
                    snapshot.health.completed_tasks.hash(&mut hasher);
                    snapshot.health.completed_workflows.hash(&mut hasher);
                    snapshot.health.dlq_size.hash(&mut hasher);
                    snapshot.workers.len().hash(&mut hasher);
                    snapshot.workflows.total.hash(&mut hasher);
                    snapshot.tasks.total.hash(&mut hasher);
                    snapshot.dlq.total.hash(&mut hasher);
                    snapshot.circuit_breakers.total.hash(&mut hasher);
                    hasher.finish()
                };

                // Only send if changed or first snapshot
                let has_changes = stream_state.last_hash != Some(current_hash);

                if has_changes {
                    // Push a metrics point for extra granularity while dashboard is open
                    // Use DB-based cumulative counts (reliable even when workers go stale)
                    state
                        .metrics
                        .push(MetricsPoint {
                            timestamp: Utc::now(),
                            running_workflows: snapshot.health.running_workflows,
                            pending_workflows: snapshot.health.pending_workflows,
                            pending_tasks: snapshot.health.pending_tasks,
                            claimed_tasks: snapshot.health.claimed_tasks,
                            active_workers: snapshot.health.active_workers,
                            load_percentage: snapshot.health.load_percentage,
                            dlq_size: snapshot.health.dlq_size,
                            tasks_completed_total: snapshot.health.completed_tasks as u64,
                            tasks_failed_total: snapshot.health.failed_tasks as u64,
                            tasks_started_total: snapshot.health.started_tasks as u64,
                            workflows_completed_total: snapshot.health.completed_workflows as u64,
                            workflows_failed_total: snapshot.health.failed_workflows as u64,
                            workflows_started_total: snapshot.health.started_workflows as u64,
                        })
                        .await;

                    let json =
                        serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".to_string());
                    let new_backoff = stream_state.config.min_backoff_ms;
                    let event = Ok(SseEvent::default()
                        .event("snapshot")
                        .data(json)
                        .retry(stream_state.config.retry_hint(new_backoff)));

                    let new_state = StreamState {
                        phase: StreamPhase::Polling,
                        backoff_ms: new_backoff,
                        last_hash: Some(current_hash),
                        ..stream_state
                    };
                    Some((stream::iter(vec![event]), (state, new_state)))
                } else {
                    // No changes, wait with backoff
                    tokio::time::sleep(Duration::from_millis(stream_state.backoff_ms)).await;
                    let new_backoff = stream_state.config.next_backoff(stream_state.backoff_ms);
                    let new_state = StreamState {
                        backoff_ms: new_backoff,
                        ..stream_state
                    };
                    Some((stream::iter(vec![]), (state, new_state)))
                }
            }
        }
    })
    .flatten();

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// GET /v1/durable/workflows/:workflow_id/sse - Stream workflow state (SSE)
///
/// Establishes a Server-Sent Events (SSE) connection for real-time workflow monitoring.
///
/// ## Connection Lifecycle Events
///
/// - **connected**: Sent immediately when the stream is established.
/// - **snapshot**: Sent when workflow state or events change.
/// - **disconnecting**: Sent before the server closes the connection for graceful cycling.
///   Data: `{"reason":"connection_cycle","retry_ms":1000}`
///
/// ## Connection Cycling
///
/// Connections are automatically cycled every 10 minutes. Before closing, the server sends
/// a `disconnecting` event so clients can reconnect seamlessly.
///
/// ## Retry Hints
///
/// Each SSE event includes a `retry:` field (in milliseconds) that hints reconnection timing.
#[utoipa::path(
    get,
    path = "/v1/durable/workflows/{workflow_id}/sse",
    params(
        ("workflow_id" = Uuid, Path, description = "Workflow ID")
    ),
    responses(
        (status = 200, description = "SSE event stream with 'connected', 'snapshot', and 'disconnecting' events", content_type = "text/event-stream"),
        (status = 404, description = "Workflow not found"),
        (status = 503, description = "Durable store not available")
    ),
    tag = "durable"
)]
pub async fn stream_workflow_sse(
    State(state): State<AppState>,
    Path(workflow_id): Path<Uuid>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, StatusCode> {
    // Verify store is available
    let store = state
        .get_store()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    // Verify workflow exists
    let workflows = store
        .list_workflows(
            WorkflowFilter::default(),
            Pagination {
                offset: 0,
                limit: 1000,
            },
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to get workflows: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let workflow_exists = workflows.iter().any(|w| w.id == workflow_id);
    if !workflow_exists {
        return Err(StatusCode::NOT_FOUND);
    }

    tracing::info!(workflow_id = %workflow_id, "Starting workflow SSE stream");

    // Use monitoring config
    let config = SseStreamConfig::monitoring();
    let connection_start = Instant::now();

    // Stream state machine
    #[derive(Clone)]
    enum StreamPhase {
        SendConnected,
        Polling,
        SendDisconnecting,
        Closed,
    }

    #[derive(Clone)]
    struct StreamState {
        phase: StreamPhase,
        workflow_id: Uuid,
        backoff_ms: u64,
        config: SseStreamConfig,
        connection_start: Instant,
        /// Jittered max connection duration (prevents thundering herd reconnections)
        max_duration: std::time::Duration,
        last_event_count: usize,
        last_status: Option<String>,
        disconnect_reason: DisconnectReason,
    }

    let initial_state = StreamState {
        phase: StreamPhase::SendConnected,
        workflow_id,
        backoff_ms: config.min_backoff_ms,
        max_duration: config.jittered_max_connection_duration(),
        config,
        connection_start,
        last_event_count: 0,
        last_status: None,
        disconnect_reason: DisconnectReason::ConnectionCycle,
    };

    let stream = stream::unfold((state, initial_state), |(state, stream_state)| async move {
        match stream_state.phase {
            StreamPhase::Closed => None,

            StreamPhase::SendDisconnecting => {
                tracing::info!(
                    workflow_id = %stream_state.workflow_id,
                    duration_secs = stream_state.connection_start.elapsed().as_secs(),
                    reason = stream_state.disconnect_reason.as_str(),
                    "Workflow SSE: sending disconnecting event"
                );
                let disconnect_data = format!(
                    r#"{{"reason":"{}","retry_ms":{}}}"#,
                    stream_state.disconnect_reason.as_str(),
                    stream_state.config.disconnect_retry_ms
                );
                let disconnecting_event = Ok(SseEvent::default()
                    .event("disconnecting")
                    .data(disconnect_data)
                    .retry(stream_state.config.disconnect_retry()));

                let new_state = StreamState {
                    phase: StreamPhase::Closed,
                    ..stream_state
                };
                Some((stream::iter(vec![disconnecting_event]), (state, new_state)))
            }

            StreamPhase::SendConnected => {
                tracing::debug!(workflow_id = %stream_state.workflow_id, "Workflow SSE: sending connected event");
                let connected_event = Ok(SseEvent::default()
                    .event("connected")
                    .data(r#"{"status":"connected"}"#)
                    .retry(stream_state.config.retry_hint(stream_state.backoff_ms)));
                let new_state = StreamState {
                    phase: StreamPhase::Polling,
                    ..stream_state
                };
                Some((stream::iter(vec![connected_event]), (state, new_state)))
            }

            StreamPhase::Polling => {
                // Helper macro to handle errors with graceful disconnect
                macro_rules! fetch_or_disconnect {
                    ($result:expr, $msg:expr) => {
                        match $result {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::error!("{}: {}", $msg, e);
                                let new_state = StreamState {
                                    phase: StreamPhase::SendDisconnecting,
                                    disconnect_reason: DisconnectReason::Error,
                                    ..stream_state
                                };
                                return Some((stream::iter(vec![]), (state, new_state)));
                            }
                        }
                    };
                }

                // Check for connection cycling (uses jittered duration)
                if stream_state.connection_start.elapsed() > stream_state.max_duration {
                    let new_state = StreamState {
                        phase: StreamPhase::SendDisconnecting,
                        disconnect_reason: DisconnectReason::ConnectionCycle,
                        ..stream_state
                    };
                    return Some((stream::iter(vec![]), (state, new_state)));
                }

                let store = match state.get_store() {
                    Ok(s) => s,
                    Err(_) => {
                        tracing::error!("Durable store not available");
                        let new_state = StreamState {
                            phase: StreamPhase::SendDisconnecting,
                            disconnect_reason: DisconnectReason::Error,
                            ..stream_state
                        };
                        return Some((stream::iter(vec![]), (state, new_state)));
                    }
                };

                // Fetch workflow
                let workflows = fetch_or_disconnect!(
                    store.list_workflows(WorkflowFilter::default(), Pagination { offset: 0, limit: 1000 }).await,
                    "Failed to fetch workflows"
                );

                let workflow = match workflows.into_iter().find(|w| w.id == stream_state.workflow_id) {
                    Some(w) => WorkflowResponse::from(w),
                    None => {
                        // Workflow deleted, send graceful disconnect
                        tracing::info!(workflow_id = %stream_state.workflow_id, "Workflow deleted, closing SSE stream");
                        let new_state = StreamState {
                            phase: StreamPhase::SendDisconnecting,
                            disconnect_reason: DisconnectReason::Error,
                            ..stream_state
                        };
                        return Some((stream::iter(vec![]), (state, new_state)));
                    }
                };

                // Fetch events
                let events: Vec<WorkflowEventResponse> = fetch_or_disconnect!(
                    store.get_workflow_events(stream_state.workflow_id).await,
                    "Failed to fetch workflow events"
                )
                .into_iter()
                .map(WorkflowEventResponse::from)
                .collect();

                // Detect changes
                let status_changed = stream_state.last_status.as_ref() != Some(&workflow.status);
                let events_changed = events.len() != stream_state.last_event_count;
                let has_changes = status_changed || events_changed;

                if has_changes {
                    let new_status = workflow.status.clone();
                    let event_count = events.len();
                    let new_backoff = stream_state.config.min_backoff_ms;
                    let snapshot = WorkflowSnapshot { workflow, events };
                    let json = serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".to_string());
                    let event = Ok(SseEvent::default()
                        .event("snapshot")
                        .data(json)
                        .retry(stream_state.config.retry_hint(new_backoff)));

                    let new_state = StreamState {
                        phase: StreamPhase::Polling,
                        backoff_ms: new_backoff,
                        last_event_count: event_count,
                        last_status: Some(new_status),
                        ..stream_state
                    };
                    Some((stream::iter(vec![event]), (state, new_state)))
                } else {
                    // No changes, wait with backoff
                    tokio::time::sleep(Duration::from_millis(stream_state.backoff_ms)).await;
                    let new_backoff = stream_state.config.next_backoff(stream_state.backoff_ms);
                    let new_state = StreamState {
                        backoff_ms: new_backoff,
                        ..stream_state
                    };
                    Some((stream::iter(vec![]), (state, new_state)))
                }
            }
        }
    })
    .flatten();

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_durable_snapshot_serialization() {
        let snapshot = DurableSnapshot {
            health: HealthResponse {
                status: "healthy".to_string(),
                total_workers: 2,
                active_workers: 2,
                workers_accepting: 2,
                total_capacity: 10,
                current_load: 3,
                load_percentage: 30.0,
                pending_tasks: 5,
                claimed_tasks: 3,
                completed_tasks: 20,
                failed_tasks: 1,
                started_tasks: 24,
                running_workflows: 1,
                pending_workflows: 0,
                completed_workflows: 10,
                failed_workflows: 0,
                started_workflows: 11,
                dlq_size: 0,
            },
            workers: vec![],
            workflows: WorkflowsListResponse {
                data: vec![],
                total: 0,
            },
            tasks: TasksListResponse {
                data: vec![],
                total: 0,
            },
            dlq: DlqListResponse {
                data: vec![],
                total: 0,
            },
            circuit_breakers: CircuitBreakersListResponse {
                data: vec![],
                total: 0,
            },
            metrics_history: vec![MetricsPoint {
                timestamp: Utc::now(),
                running_workflows: 1,
                pending_workflows: 0,
                pending_tasks: 5,
                claimed_tasks: 3,
                active_workers: 2,
                load_percentage: 30.0,
                dlq_size: 0,
                tasks_completed_total: 42,
                tasks_failed_total: 1,
                tasks_started_total: 46,
                workflows_completed_total: 10,
                workflows_failed_total: 0,
                workflows_started_total: 11,
            }],
        };

        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("\"status\":\"healthy\""));
        assert!(json.contains("\"active_workers\":2"));
        assert!(json.contains("\"load_percentage\":30.0"));
        assert!(json.contains("\"metrics_history\""));
        assert!(json.contains("\"tasks_completed_total\":42"));
    }

    #[test]
    fn test_workflow_snapshot_serialization() {
        let snapshot = WorkflowSnapshot {
            workflow: WorkflowResponse {
                id: Uuid::nil(),
                workflow_type: "test_workflow".to_string(),
                status: "running".to_string(),
                input: serde_json::json!({"key": "value"}),
                result: None,
                error: None,
                created_at: Utc::now(),
                started_at: Some(Utc::now()),
                completed_at: None,
            },
            events: vec![],
        };

        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("\"workflow_type\":\"test_workflow\""));
        assert!(json.contains("\"status\":\"running\""));
        assert!(json.contains("\"events\":[]"));
    }

    #[test]
    fn test_health_response_from_system_health() {
        let health = SystemHealth {
            total_workers: 5,
            active_workers: 3,
            workers_accepting: 2,
            total_capacity: 50,
            current_load: 10,
            pending_tasks: 5,
            claimed_tasks: 10,
            completed_tasks: 0,
            failed_tasks: 0,
            started_tasks: 10,
            running_workflows: 2,
            pending_workflows: 1,
            completed_workflows: 0,
            failed_workflows: 0,
            started_workflows: 2,
            dlq_size: 0,
        };

        let response = HealthResponse::from(health);

        assert_eq!(response.status, "healthy");
        assert_eq!(response.total_workers, 5);
        assert_eq!(response.active_workers, 3);
        assert_eq!(response.load_percentage, 20.0); // 10/50 * 100
    }

    #[test]
    fn test_health_status_degraded_when_no_active_workers() {
        let health = SystemHealth {
            total_workers: 5,
            active_workers: 0,
            workers_accepting: 0,
            total_capacity: 0,
            current_load: 0,
            pending_tasks: 0,
            claimed_tasks: 0,
            completed_tasks: 0,
            failed_tasks: 0,
            started_tasks: 0,
            running_workflows: 0,
            pending_workflows: 0,
            completed_workflows: 0,
            failed_workflows: 0,
            started_workflows: 0,
            dlq_size: 0,
        };

        let response = HealthResponse::from(health);
        assert_eq!(response.status, "degraded");
    }

    #[test]
    fn test_health_status_warning_when_dlq_has_items() {
        let health = SystemHealth {
            total_workers: 2,
            active_workers: 2,
            workers_accepting: 2,
            total_capacity: 10,
            current_load: 0,
            pending_tasks: 0,
            claimed_tasks: 0,
            completed_tasks: 0,
            failed_tasks: 0,
            started_tasks: 0,
            running_workflows: 0,
            pending_workflows: 0,
            completed_workflows: 0,
            failed_workflows: 0,
            started_workflows: 0,
            dlq_size: 5,
        };

        let response = HealthResponse::from(health);
        assert_eq!(response.status, "warning");
    }

    #[test]
    fn test_load_percentage_zero_when_no_capacity() {
        let health = SystemHealth {
            total_workers: 0,
            active_workers: 0,
            workers_accepting: 0,
            total_capacity: 0,
            current_load: 0,
            pending_tasks: 0,
            completed_tasks: 0,
            failed_tasks: 0,
            started_tasks: 0,
            claimed_tasks: 0,
            running_workflows: 0,
            pending_workflows: 0,
            completed_workflows: 0,
            failed_workflows: 0,
            started_workflows: 0,
            dlq_size: 0,
        };

        let response = HealthResponse::from(health);
        assert_eq!(response.load_percentage, 0.0);
    }

    #[test]
    fn test_app_state_get_store_none() {
        let state = AppState::new(None);
        let result = state.get_store();
        assert!(result.is_err());
    }

    #[test]
    fn test_disconnect_event_format_connection_cycle() {
        // Test that connection cycle disconnect event has correct format
        let config = SseStreamConfig::monitoring();
        let disconnect_data = format!(
            r#"{{"reason":"{}","retry_ms":{}}}"#,
            DisconnectReason::ConnectionCycle.as_str(),
            config.disconnect_retry_ms
        );

        let parsed: serde_json::Value = serde_json::from_str(&disconnect_data).unwrap();
        assert_eq!(parsed["reason"], "connection_cycle");
        assert_eq!(parsed["retry_ms"], 1000);
    }

    #[test]
    fn test_disconnect_event_format_error() {
        // Test that error disconnect event has correct format
        // This verifies the graceful disconnect on error feature
        let config = SseStreamConfig::monitoring();
        let disconnect_data = format!(
            r#"{{"reason":"{}","retry_ms":{}}}"#,
            DisconnectReason::Error.as_str(),
            config.disconnect_retry_ms
        );

        let parsed: serde_json::Value = serde_json::from_str(&disconnect_data).unwrap();
        assert_eq!(parsed["reason"], "error");
        assert_eq!(parsed["retry_ms"], 1000);
    }

    #[test]
    fn test_disconnect_event_parseable_by_client() {
        // Simulate what the client receives and parses
        // This matches the JS code: const data = JSON.parse(event.data)
        let config = SseStreamConfig::monitoring();
        let disconnect_data = format!(
            r#"{{"reason":"{}","retry_ms":{}}}"#,
            DisconnectReason::Error.as_str(),
            config.disconnect_retry_ms
        );

        // Parse like the client does
        let parsed: serde_json::Value = serde_json::from_str(&disconnect_data).unwrap();

        // Extract retry_ms like client: const retryMs = data.retry_ms ?? 1000;
        let retry_ms = parsed["retry_ms"].as_i64().unwrap_or(1000);
        assert_eq!(retry_ms, 1000);

        // Client uses reason to log
        let reason = parsed["reason"].as_str().unwrap_or("unknown");
        assert_eq!(reason, "error");
    }

    // ============================================
    // WorkersSummaryResponse tests
    // ============================================

    /// Helper to create a WorkerResponse for testing
    fn make_worker(
        id: &str,
        status: &str,
        max_concurrency: u32,
        current_load: u32,
    ) -> WorkerResponse {
        WorkerResponse {
            id: id.to_string(),
            worker_group: Some("default".to_string()),
            activity_types: vec!["run_session".to_string()],
            max_concurrency,
            current_load,
            status: status.to_string(),
            accepting_tasks: status == "active",
            backpressure_reason: None,
            started_at: Utc::now(),
            last_heartbeat_at: Utc::now(),
            hostname: Some("host1".to_string()),
            version: Some("0.8.1".to_string()),
            metadata: None,
            tasks_completed: 100,
            tasks_failed: 2,
            avg_task_duration_ms: Some(500),
        }
    }

    /// Build a WorkersListResponse the same way list_workers() does
    fn build_workers_list(workers: Vec<WorkerResponse>) -> WorkersListResponse {
        let total = workers.len();
        let summary = WorkersSummaryResponse {
            active: workers.iter().filter(|w| w.status == "active").count(),
            draining: workers.iter().filter(|w| w.status == "draining").count(),
            stopped: workers.iter().filter(|w| w.status == "stopped").count(),
            total_capacity: workers
                .iter()
                .filter(|w| w.status == "active")
                .map(|w| w.max_concurrency as usize)
                .sum(),
            total_load: workers
                .iter()
                .filter(|w| w.status == "active")
                .map(|w| w.current_load as usize)
                .sum(),
        };
        WorkersListResponse {
            data: workers,
            total,
            summary,
        }
    }

    #[test]
    fn test_workers_summary_mixed_statuses() {
        let workers = vec![
            make_worker("w1", "active", 10, 3),
            make_worker("w2", "active", 8, 5),
            make_worker("w3", "draining", 10, 2),
            make_worker("w4", "stopped", 10, 0),
        ];

        let resp = build_workers_list(workers);

        assert_eq!(resp.total, 4);
        assert_eq!(resp.summary.active, 2);
        assert_eq!(resp.summary.draining, 1);
        assert_eq!(resp.summary.stopped, 1);
        // Only active workers count toward capacity/load
        assert_eq!(resp.summary.total_capacity, 18); // 10 + 8
        assert_eq!(resp.summary.total_load, 8); // 3 + 5
    }

    #[test]
    fn test_workers_summary_all_active() {
        let workers = vec![
            make_worker("w1", "active", 10, 5),
            make_worker("w2", "active", 10, 3),
            make_worker("w3", "active", 10, 7),
        ];

        let resp = build_workers_list(workers);

        assert_eq!(resp.summary.active, 3);
        assert_eq!(resp.summary.draining, 0);
        assert_eq!(resp.summary.stopped, 0);
        assert_eq!(resp.summary.total_capacity, 30);
        assert_eq!(resp.summary.total_load, 15);
    }

    #[test]
    fn test_workers_summary_no_active_workers() {
        let workers = vec![
            make_worker("w1", "draining", 10, 2),
            make_worker("w2", "stopped", 10, 0),
        ];

        let resp = build_workers_list(workers);

        assert_eq!(resp.summary.active, 0);
        assert_eq!(resp.summary.draining, 1);
        assert_eq!(resp.summary.stopped, 1);
        // Draining/stopped workers don't count toward capacity/load
        assert_eq!(resp.summary.total_capacity, 0);
        assert_eq!(resp.summary.total_load, 0);
    }

    #[test]
    fn test_workers_summary_empty_list() {
        let resp = build_workers_list(vec![]);

        assert_eq!(resp.total, 0);
        assert_eq!(resp.summary.active, 0);
        assert_eq!(resp.summary.draining, 0);
        assert_eq!(resp.summary.stopped, 0);
        assert_eq!(resp.summary.total_capacity, 0);
        assert_eq!(resp.summary.total_load, 0);
    }

    #[test]
    fn test_workers_summary_serialization_includes_summary() {
        let workers = vec![
            make_worker("w1", "active", 10, 3),
            make_worker("w2", "draining", 8, 1),
        ];

        let resp = build_workers_list(workers);
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Verify summary field exists and has correct values
        assert!(
            parsed["summary"].is_object(),
            "summary must be present in JSON"
        );
        assert_eq!(parsed["summary"]["active"], 1);
        assert_eq!(parsed["summary"]["draining"], 1);
        assert_eq!(parsed["summary"]["stopped"], 0);
        assert_eq!(parsed["summary"]["total_capacity"], 10);
        assert_eq!(parsed["summary"]["total_load"], 3);

        // Verify top-level fields
        assert_eq!(parsed["total"], 2);
        assert!(parsed["data"].is_array());
        assert_eq!(parsed["data"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_workers_summary_only_active_contribute_to_capacity() {
        // Edge case: a draining worker with high load should NOT inflate totals
        let workers = vec![
            make_worker("w1", "active", 5, 2),
            make_worker("w2", "draining", 100, 80),
            make_worker("w3", "stopped", 50, 0),
        ];

        let resp = build_workers_list(workers);

        assert_eq!(resp.summary.total_capacity, 5); // Only w1
        assert_eq!(resp.summary.total_load, 2); // Only w1
    }

    #[test]
    fn test_workers_summary_single_active_worker() {
        let workers = vec![make_worker("w1", "active", 20, 15)];

        let resp = build_workers_list(workers);

        assert_eq!(resp.summary.active, 1);
        assert_eq!(resp.summary.draining, 0);
        assert_eq!(resp.summary.stopped, 0);
        assert_eq!(resp.summary.total_capacity, 20);
        assert_eq!(resp.summary.total_load, 15);
    }

    #[test]
    fn test_workers_summary_zero_load_active_workers() {
        let workers = vec![
            make_worker("w1", "active", 10, 0),
            make_worker("w2", "active", 10, 0),
        ];

        let resp = build_workers_list(workers);

        assert_eq!(resp.summary.active, 2);
        assert_eq!(resp.summary.total_capacity, 20);
        assert_eq!(resp.summary.total_load, 0);
    }

    // ============================================
    // MetricsCollector tests
    // ============================================

    #[tokio::test]
    async fn test_metrics_collector_new_creates_empty() {
        let collector = MetricsCollector::new(10);
        let points = collector.get_points().await;
        assert!(points.is_empty());
    }

    #[tokio::test]
    async fn test_metrics_collector_push_adds_points() {
        let collector = MetricsCollector::new(10);

        collector
            .push(MetricsPoint {
                timestamp: Utc::now(),
                running_workflows: 1,
                pending_workflows: 0,
                pending_tasks: 0,
                claimed_tasks: 0,
                active_workers: 1,
                load_percentage: 10.0,
                dlq_size: 0,
                tasks_completed_total: 0,
                tasks_failed_total: 0,
                tasks_started_total: 0,
                workflows_completed_total: 0,
                workflows_failed_total: 0,
                workflows_started_total: 0,
            })
            .await;

        let points = collector.get_points().await;
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].running_workflows, 1);
    }

    #[tokio::test]
    async fn test_metrics_collector_evicts_oldest_when_full() {
        let collector = MetricsCollector::new(3);

        for i in 0..5 {
            collector
                .push(MetricsPoint {
                    timestamp: Utc::now(),
                    running_workflows: i,
                    pending_workflows: 0,
                    pending_tasks: 0,
                    claimed_tasks: 0,
                    active_workers: 0,
                    load_percentage: 0.0,
                    dlq_size: 0,
                    tasks_completed_total: 0,
                    tasks_failed_total: 0,
                    tasks_started_total: 0,
                    workflows_completed_total: 0,
                    workflows_failed_total: 0,
                    workflows_started_total: 0,
                })
                .await;
        }

        let points = collector.get_points().await;
        assert_eq!(points.len(), 3);
        // Oldest (0, 1) should be evicted; remaining are 2, 3, 4
        assert_eq!(points[0].running_workflows, 2);
        assert_eq!(points[1].running_workflows, 3);
        assert_eq!(points[2].running_workflows, 4);
    }

    #[tokio::test]
    async fn test_metrics_collector_get_points_returns_in_order() {
        let collector = MetricsCollector::new(100);

        for i in 0..5 {
            collector
                .push(MetricsPoint {
                    timestamp: Utc::now(),
                    running_workflows: i,
                    pending_workflows: 0,
                    pending_tasks: 0,
                    claimed_tasks: 0,
                    active_workers: 0,
                    load_percentage: 0.0,
                    dlq_size: 0,
                    tasks_completed_total: 0,
                    tasks_failed_total: 0,
                    tasks_started_total: 0,
                    workflows_completed_total: 0,
                    workflows_failed_total: 0,
                    workflows_started_total: 0,
                })
                .await;
        }

        let points = collector.get_points().await;
        assert_eq!(points.len(), 5);
        for (idx, point) in points.iter().enumerate() {
            assert_eq!(point.running_workflows, idx);
        }
    }

    #[test]
    fn test_metrics_timeseries_response_serialization() {
        let response = MetricsTimeSeriesResponse {
            points: vec![
                MetricsPoint {
                    timestamp: Utc::now(),
                    running_workflows: 3,
                    pending_workflows: 1,
                    pending_tasks: 5,
                    claimed_tasks: 2,
                    active_workers: 4,
                    load_percentage: 25.5,
                    dlq_size: 0,
                    tasks_completed_total: 100,
                    tasks_failed_total: 2,
                    tasks_started_total: 104,
                    workflows_completed_total: 50,
                    workflows_failed_total: 1,
                    workflows_started_total: 54,
                },
                MetricsPoint {
                    timestamp: Utc::now(),
                    running_workflows: 4,
                    pending_workflows: 0,
                    pending_tasks: 3,
                    claimed_tasks: 1,
                    active_workers: 4,
                    load_percentage: 30.0,
                    dlq_size: 1,
                    tasks_completed_total: 105,
                    tasks_failed_total: 3,
                    tasks_started_total: 112,
                    workflows_completed_total: 52,
                    workflows_failed_total: 1,
                    workflows_started_total: 57,
                },
            ],
            resolution_seconds: 10,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"resolution_seconds\":10"));
        assert!(json.contains("\"points\""));
        assert!(json.contains("\"tasks_completed_total\":100"));
        assert!(json.contains("\"tasks_completed_total\":105"));

        // Verify round-trip deserialization shape
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["resolution_seconds"], 10);
        assert_eq!(parsed["points"].as_array().unwrap().len(), 2);
    }
}

//! Durable health response projection, shared by polling and live metrics.

use everruns_durable::SystemHealth;
use serde::Serialize;
use utoipa::ToSchema;

/// System health response
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    /// Aggregate system status: `healthy`, `degraded`, or `unhealthy`. Derived from worker availability, load, and queue depths.
    #[schema(example = "healthy")]
    pub status: String,
    /// Total number of workers registered (heartbeating in the last window).
    #[schema(example = 4)]
    pub total_workers: usize,
    /// Number of workers in the `running` state, ready to claim tasks.
    #[schema(example = 4)]
    pub active_workers: usize,
    /// Number of workers currently accepting new task assignments (subset of `active_workers`; drains/backpressure excluded).
    #[schema(example = 4)]
    pub workers_accepting: usize,
    /// Sum of `max_concurrency` across all workers (the upper bound on concurrent task execution).
    #[schema(example = 32)]
    pub total_capacity: usize,
    /// Total tasks currently in flight across all workers.
    #[schema(example = 7)]
    pub current_load: usize,
    /// `current_load / total_capacity * 100`. 0.0 when no workers are registered.
    #[schema(example = 21.875)]
    pub load_percentage: f64,
    /// Tasks waiting to be claimed (gauge).
    #[schema(example = 2)]
    pub pending_tasks: usize,
    /// Tasks currently claimed by a worker (gauge).
    #[schema(example = 7)]
    pub claimed_tasks: usize,
    /// Cumulative count of tasks that completed successfully (monotonic counter).
    #[schema(example = 12041)]
    pub completed_tasks: usize,
    /// Cumulative count of tasks that failed terminally or were sent to the DLQ (monotonic counter).
    #[schema(example = 34)]
    pub failed_tasks: usize,
    /// Cumulative count of tasks claimed at least once (monotonic counter).
    #[schema(example = 12082)]
    pub started_tasks: usize,
    /// Workflows currently executing (gauge).
    #[schema(example = 3)]
    pub running_workflows: usize,
    /// Workflows waiting to be claimed (gauge).
    #[schema(example = 1)]
    pub pending_workflows: usize,
    /// Cumulative count of workflows that completed successfully (monotonic counter).
    #[schema(example = 4128)]
    pub completed_workflows: usize,
    /// Cumulative count of workflows that ended in failure (monotonic counter).
    #[schema(example = 12)]
    pub failed_workflows: usize,
    /// Cumulative count of workflows that started (monotonic counter).
    #[schema(example = 4144)]
    pub started_workflows: usize,
    /// Size of the dead-letter queue (gauge). High values indicate stuck activities.
    #[schema(example = 0)]
    pub dlq_size: usize,
    /// Event-delivery backend in use: `nats` for distributed deployments, `in_memory` for single-instance. `None` if the field was omitted by an older server.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schema(example = "nats")]
    pub event_delivery: Option<String>,
}

// Allow tests to construct HealthResponse without specifying all fields
impl Default for HealthResponse {
    fn default() -> Self {
        Self {
            status: "unknown".to_string(),
            total_workers: 0,
            active_workers: 0,
            workers_accepting: 0,
            total_capacity: 0,
            current_load: 0,
            load_percentage: 0.0,
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
            event_delivery: None,
        }
    }
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
            event_delivery: None,
        }
    }
}

impl HealthResponse {
    /// Set the event delivery backend name
    pub fn with_event_delivery(mut self, backend: String) -> Self {
        self.event_delivery = Some(backend);
        self
    }
}

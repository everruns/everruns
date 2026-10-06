// Prometheus /metrics endpoint
//
// Decision: Uses `metrics` with an in-tree recorder (see `prometheus_recorder`),
// which is lighter than both the OTel bridge and `metrics-exporter-prometheus`.
// Decision: Metrics collection is enabled by default (METRICS_ENABLED=true),
// but public serving on the main API port is disabled by default.
// Decision: Two serving modes to keep metrics internal in production:
//   1. METRICS_ADDR set (e.g. 127.0.0.1:9090) → dedicated internal-only HTTP server
//      serving only /metrics. Not reachable from outside the pod/host. This is the
//      recommended production pattern; scrapers access via sidecar or pod-local.
//   2. METRICS_ADDR unset + METRICS_PUBLIC_ON_MAIN=true → /metrics mounted on the
//      main API server for local/dev convenience. Keep disabled in production.
// Decision: No auth on /metrics (Prometheus standard).
// Decision: Horizontal scaling model:
//   - Gauges from DB (each replica reports the same logical value; Prometheus
//     keeps one series per `instance`, so queries for cluster-level values
//     should aggregate, e.g. `max without(instance)`)
//   - Counters from local events only (each replica counts its own work — no
//     double-counting across replicas; use `sum without(instance)` for totals)
//   - Histograms from local observations (naturally partitioned per instance)

use super::prometheus_recorder::{self, PrometheusHandle};
use axum::http::header;
use axum::response::IntoResponse;
use axum::{Router, extract::State, routing::get};
use everruns_core::config::{env_bool, env_opt_string};
use sqlx::PgPool;
use tokio::task::JoinHandle;

/// Configuration for the Prometheus metrics endpoint.
pub struct PrometheusConfig {
    /// Whether metrics collection is enabled.
    pub enabled: bool,
    /// Optional separate bind address for the metrics server (e.g. "127.0.0.1:9090").
    /// When set, /metrics is served on a dedicated internal-only HTTP server
    /// instead of the main API server — keeping it off the public interface.
    pub metrics_addr: Option<String>,
    /// Whether to expose `/metrics` on the main API server when `METRICS_ADDR`
    /// is unset. Defaults to false for safer production posture.
    pub public_on_main: bool,
}

impl PrometheusConfig {
    /// Load from environment.
    ///
    /// - `METRICS_ENABLED`: enable/disable metrics (default: true)
    /// - `METRICS_ADDR`: separate bind address for internal-only metrics server
    ///   (e.g. "127.0.0.1:9090"). When set, /metrics is NOT mounted on the main
    ///   API server. Recommended for production to avoid external exposure.
    /// - `METRICS_PUBLIC_ON_MAIN`: allow mounting unauthenticated `/metrics` on
    ///   the main API server when `METRICS_ADDR` is unset (default: false).
    pub fn from_env() -> Self {
        Self {
            enabled: env_bool("METRICS_ENABLED", true),
            metrics_addr: env_opt_string("METRICS_ADDR"),
            public_on_main: env_bool("METRICS_PUBLIC_ON_MAIN", false),
        }
    }
}

/// Install the in-tree Prometheus recorder and return the render handle.
///
/// Must be called exactly once before any `metrics::*!` macros are used.
/// Returns `None` if installation fails (e.g. another recorder is already set).
pub fn install_prometheus_recorder() -> Option<PrometheusHandle> {
    prometheus_recorder::install_recorder()
        .map_err(|e| {
            tracing::warn!(error = %e, "Failed to install Prometheus recorder — metrics endpoint disabled");
        })
        .ok()
}

/// GET /metrics — render all metrics in Prometheus exposition format.
async fn metrics_handler(State(handle): State<PrometheusHandle>) -> impl IntoResponse {
    let body = handle.render();
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

/// Build the `/metrics` route. Mounted at the root (outside API prefix).
pub fn route(handle: PrometheusHandle) -> Router {
    Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(handle)
}

/// Spawn a dedicated HTTP server that only serves `/metrics` on the given address.
///
/// Used in production to keep metrics on an internal-only port (e.g. 127.0.0.1:9090)
/// that is not reachable from outside the pod/host. Scrapers access via sidecar,
/// pod-local networking, or service mesh.
pub fn spawn_metrics_server(handle: PrometheusHandle, addr: String) -> JoinHandle<()> {
    tokio::spawn(async move {
        let app = route(handle);
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                tracing::info!(addr = %addr, "Metrics server listening (internal-only)");
                if let Err(e) = axum::serve(listener, app).await {
                    tracing::error!(error = %e, "Metrics server error");
                }
            }
            Err(e) => {
                tracing::error!(addr = %addr, error = %e, "Failed to bind metrics server");
            }
        }
    })
}

// ============================================================================
// Metric names (all prefixed `everruns_`)
// ============================================================================

/// Well-known metric names to avoid typos across the codebase.
pub mod names {
    // === Gauges (from DB via MetricsCollector bridge) ===
    // Each replica reads the same DB and emits the same values. Prometheus
    // keeps separate series per `instance`. Queries for cluster-level values
    // should aggregate: `max without(instance) (everruns_workflows_running)`.
    pub const WORKFLOWS_RUNNING: &str = "everruns_workflows_running";
    pub const WORKFLOWS_PENDING: &str = "everruns_workflows_pending";
    pub const TASKS_PENDING: &str = "everruns_tasks_pending";
    pub const TASKS_CLAIMED: &str = "everruns_tasks_claimed";
    pub const WORKERS_ACTIVE: &str = "everruns_workers_active";
    pub const LOAD_RATIO: &str = "everruns_load_ratio";
    pub const DLQ_SIZE: &str = "everruns_dlq_size";
    /// Current connections opened by an sqlx pool. Label: pool.
    pub const DATABASE_POOL_SIZE: &str = "everruns_database_pool_size";
    /// Configured connection ceiling for an sqlx pool. Label: pool.
    pub const DATABASE_POOL_MAX_SIZE: &str = "everruns_database_pool_max_size";
    /// Open connections currently idle in an sqlx pool. Label: pool.
    pub const DATABASE_POOL_IDLE: &str = "everruns_database_pool_idle";
    /// Open connections currently checked out from an sqlx pool. Label: pool.
    pub const DATABASE_POOL_IN_USE: &str = "everruns_database_pool_in_use";
    // DB cumulative totals as gauges (not _total — these are global state, not
    // per-instance counters). These are monotonically increasing in normal
    // operation. Use delta() in PromQL for rate-like queries on gauges.
    pub const TASKS_COMPLETED: &str = "everruns_tasks_completed";
    pub const TASKS_FAILED: &str = "everruns_tasks_failed";
    pub const TASKS_STARTED: &str = "everruns_tasks_started";
    pub const WORKFLOWS_COMPLETED: &str = "everruns_workflows_completed";
    pub const WORKFLOWS_FAILED: &str = "everruns_workflows_failed";
    pub const WORKFLOWS_STARTED: &str = "everruns_workflows_started";

    // === Counters (from local events — per-instance, no double-counting) ===
    pub const HTTP_REQUESTS_TOTAL: &str = "everruns_http_requests_total";
    pub const LLM_REQUESTS_TOTAL: &str = "everruns_llm_requests_total";
    pub const TOOL_EXECUTIONS_TOTAL: &str = "everruns_tool_executions_total";
    /// Generations by normalized finish reason. Labels: provider, model,
    /// finish_reason (`stop`, `tool_calls`, `length`, `content_filter`, ...).
    pub const LLM_FINISH_REASON_TOTAL: &str = "everruns_llm_finish_reason_total";
    /// Tool calls a driver discarded because the response was cut off or
    /// rejected. Labels: provider, model, reason (the finish reason).
    pub const LLM_TOOL_CALLS_DROPPED_TOTAL: &str = "everruns_llm_tool_calls_dropped_total";
    /// Tool calls run from a truncated response; their own arguments are
    /// complete. Labels: provider, model, reason (the finish reason).
    pub const LLM_TOOL_CALLS_TRUNCATED_EXECUTED_TOTAL: &str =
        "everruns_llm_tool_calls_truncated_executed_total";
    /// Generations the output-truncation gate acted on. Labels: provider,
    /// model, action (`retried` | `failed`).
    pub const LLM_TRUNCATION_GATE_TOTAL: &str = "everruns_llm_truncation_gate_total";
    /// Provider retries before a generation succeeded. Label: provider.
    pub const LLM_RETRIES_TOTAL: &str = "everruns_llm_retries_total";
    /// Counter for every domain Command invocation across HTTP, MCP and
    /// gRPC ExecuteCommand. Labels: name, category, status (ok |
    /// bad_request | unprocessable | forbidden | not_found | conflict |
    /// rate_limited | unavailable | internal).
    pub const COMMANDS_TOTAL: &str = "everruns_commands_total";
    /// Entity history rows that could not be written after their mutation
    /// committed (see `domains::change_history`). Should stay at zero.
    pub const ENTITY_HISTORY_WRITE_FAILURES: &str = "everruns_entity_history_write_failures_total";
    /// Agent-made changes recorded without a reason, while the org does not
    /// require one. Label: entity_kind.
    pub const ENTITY_CHANGES_WITHOUT_REASON: &str = "everruns_entity_changes_without_reason_total";

    /// Orphaned blob objects deleted by the object-storage GC sweep (objects
    /// present in the bucket with no live sidecar pointer, older than the grace
    /// period). Per-instance counter.
    pub const BLOB_GC_ORPHANS_DELETED_TOTAL: &str = "everruns_blob_gc_orphans_deleted_total";
    /// Bytes reclaimed by the object-storage GC sweep. Per-instance counter.
    pub const BLOB_GC_BYTES_RECLAIMED_TOTAL: &str = "everruns_blob_gc_bytes_reclaimed_total";

    // === Histograms (from local observations — per-instance) ===
    pub const HTTP_REQUEST_DURATION: &str = "everruns_http_request_duration_seconds";
    pub const LLM_REQUEST_DURATION: &str = "everruns_llm_request_duration_seconds";
    pub const TOOL_EXECUTION_DURATION: &str = "everruns_tool_execution_duration_seconds";
    /// Total backoff (Retry-After or computed) a retried generation waited.
    /// Label: provider.
    pub const LLM_RETRY_WAIT_DURATION: &str = "everruns_llm_retry_wait_seconds";
    /// Wall-clock duration of every domain Command invocation. Same labels
    /// as `COMMANDS_TOTAL`.
    pub const COMMAND_DURATION: &str = "everruns_command_duration_seconds";
    /// Input message persisted to `turn.started`: enqueue plus worker pickup.
    pub const TURN_PICKUP_DURATION: &str = "everruns_turn_pickup_seconds";
    /// Input message to the first streamed token of the turn's first LLM call.
    pub const TURN_FIRST_TOKEN_DURATION: &str = "everruns_turn_first_token_seconds";
    /// Hand-off between durable turn phases (queue wait). Label: phase
    /// (reason | act), the phase being started.
    pub const TURN_PHASE_GAP_DURATION: &str = "everruns_turn_phase_gap_seconds";
    /// Turn wall-clock not spent in the LLM or tools. Label: outcome.
    pub const TURN_OVERHEAD_DURATION: &str = "everruns_turn_overhead_seconds";
}

// ============================================================================
// Gauge bridge: MetricsCollector → Prometheus gauges
// ============================================================================

use super::durable::MetricsCollector;

/// Spawn a background task that copies the latest MetricsCollector snapshot
/// into Prometheus gauges every 10 seconds (aligned with the sampler).
///
/// All metrics here are gauges (absolute DB state). Every replica reads the same
/// shared DB and emits the same values. Prometheus keeps separate series per
/// `instance` — queries should aggregate for cluster-level values, e.g.
/// `max without(instance) (everruns_workflows_running)`.
pub fn spawn_gauge_bridge(collector: MetricsCollector) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        tracing::info!("Prometheus gauge bridge started (10s interval)");

        loop {
            interval.tick().await;

            let Some(latest) = collector.get_latest().await else {
                continue;
            };

            // Current state gauges
            metrics::gauge!(names::WORKFLOWS_RUNNING).set(latest.running_workflows as f64);
            metrics::gauge!(names::WORKFLOWS_PENDING).set(latest.pending_workflows as f64);
            metrics::gauge!(names::TASKS_PENDING).set(latest.pending_tasks as f64);
            metrics::gauge!(names::TASKS_CLAIMED).set(latest.claimed_tasks as f64);
            metrics::gauge!(names::WORKERS_ACTIVE).set(latest.active_workers as f64);
            metrics::gauge!(names::LOAD_RATIO).set(latest.load_percentage / 100.0);
            metrics::gauge!(names::DLQ_SIZE).set(latest.dlq_size as f64);

            // DB cumulative totals as gauges. These represent global state, not
            // per-instance throughput, so gauges are correct even with multiple
            // replicas. Use delta() in PromQL for rate-like queries on gauges.
            metrics::gauge!(names::TASKS_COMPLETED).set(latest.tasks_completed_total as f64);
            metrics::gauge!(names::TASKS_FAILED).set(latest.tasks_failed_total as f64);
            metrics::gauge!(names::TASKS_STARTED).set(latest.tasks_started_total as f64);
            metrics::gauge!(names::WORKFLOWS_COMPLETED)
                .set(latest.workflows_completed_total as f64);
            metrics::gauge!(names::WORKFLOWS_FAILED).set(latest.workflows_failed_total as f64);
            metrics::gauge!(names::WORKFLOWS_STARTED).set(latest.workflows_started_total as f64);
        }
    });
}

/// Start the pool gauge bridge for the storage backend's PostgreSQL pools.
pub fn spawn_storage_pool_gauge_bridge(storage: &crate::storage::StorageBackend) {
    spawn_pool_gauge_bridge(storage.pool().clone(), storage.background_pool().clone());
}

/// Sample the process-local request and background sqlx pools every 10 seconds.
pub fn spawn_pool_gauge_bridge(request_pool: PgPool, background_pool: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        tracing::info!("Prometheus database pool gauge bridge started (10s interval)");

        loop {
            interval.tick().await;
            record_pool_gauges(&request_pool, "request");
            record_pool_gauges(&background_pool, "background");
        }
    });
}

fn record_pool_gauges(pool: &PgPool, pool_name: &'static str) {
    let size = pool.size();
    let idle = u32::try_from(pool.num_idle()).unwrap_or(u32::MAX);
    let in_use = size.saturating_sub(idle);

    metrics::gauge!(names::DATABASE_POOL_SIZE, "pool" => pool_name).set(size as f64);
    metrics::gauge!(names::DATABASE_POOL_MAX_SIZE, "pool" => pool_name)
        .set(pool.options().get_max_connections() as f64);
    metrics::gauge!(names::DATABASE_POOL_IDLE, "pool" => pool_name).set(idle as f64);
    metrics::gauge!(names::DATABASE_POOL_IN_USE, "pool" => pool_name).set(in_use as f64);
}

// ============================================================================
// HTTP request duration middleware
// ============================================================================

use axum::extract::MatchedPath;
use axum::middleware::Next;

/// Axum middleware layer that records `everruns_http_request_duration_seconds`
/// histogram with labels `method`, `path`, `status`.
///
/// Must be applied as `route_layer` (not `layer`) so `MatchedPath` is available.
pub async fn http_metrics_layer(
    matched_path: Option<MatchedPath>,
    req: axum::extract::Request,
    next: Next,
) -> impl IntoResponse {
    let method = req.method().clone();
    // Use matched route template for low-cardinality labels.
    // Fall back to "unmatched" to avoid cardinality explosion from 404 scans.
    let path = matched_path
        .map(|mp| mp.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".to_owned());

    let start = std::time::Instant::now();
    let response = next.run(req).await;
    let duration = start.elapsed();

    let status = response.status().as_u16().to_string();
    let method_str = method.to_string();
    metrics::counter!(
        names::HTTP_REQUESTS_TOTAL,
        "method" => method_str.clone(),
        "path" => path.clone(),
        "status" => status.clone(),
    )
    .increment(1);
    metrics::histogram!(
        names::HTTP_REQUEST_DURATION,
        "method" => method_str,
        "path" => path,
        "status" => status,
    )
    .record(duration.as_secs_f64());

    response
}

// ============================================================================
// EventListener for LLM + tool duration histograms
// ============================================================================

use async_trait::async_trait;
use everruns_core::EventListener;
use everruns_core::events::{
    Event, EventData, LLM_GENERATION, LlmGenerationMetadata, TOOL_COMPLETED,
};

/// The LLM edge cases one generation contributes to the counters (see
/// `everruns_contracts::llm_telemetry`). Pure, so the mapping is testable
/// without a recorder.
#[derive(Debug, Default, PartialEq)]
struct LlmOutcomeSample {
    finish_reason: Option<String>,
    tool_calls_dropped: u32,
    tool_calls_truncated_executed: u32,
    truncation_gate: Option<String>,
    retries: u32,
    retry_wait_secs: Option<f64>,
}

impl LlmOutcomeSample {
    fn of(meta: &LlmGenerationMetadata) -> Self {
        Self {
            finish_reason: meta
                .finish_reasons
                .as_ref()
                .and_then(|reasons| reasons.first().cloned()),
            tool_calls_dropped: meta.tool_calls_dropped,
            tool_calls_truncated_executed: meta.tool_calls_truncated_executed,
            truncation_gate: meta.truncation_gate.clone(),
            retries: meta.retry.as_ref().map_or(0, |retry| retry.attempts),
            retry_wait_secs: meta
                .retry
                .as_ref()
                .filter(|retry| retry.attempts > 0)
                .map(|retry| retry.total_wait_ms as f64 / 1000.0),
        }
    }

    fn record(self, provider: &str, model: &str) {
        let reason = self.finish_reason.unwrap_or_else(|| "unknown".to_string());
        let labels = [
            ("provider", provider.to_string()),
            ("model", model.to_string()),
        ];
        let counter = |name: &'static str, value: u32, label: &'static str| {
            if value > 0 {
                let mut labels = labels.to_vec();
                labels.push((label, reason.clone()));
                metrics::counter!(name, &labels).increment(u64::from(value));
            }
        };
        counter(names::LLM_FINISH_REASON_TOTAL, 1, "finish_reason");
        counter(
            names::LLM_TOOL_CALLS_DROPPED_TOTAL,
            self.tool_calls_dropped,
            "reason",
        );
        counter(
            names::LLM_TOOL_CALLS_TRUNCATED_EXECUTED_TOTAL,
            self.tool_calls_truncated_executed,
            "reason",
        );
        if let Some(action) = self.truncation_gate {
            let mut labels = labels.to_vec();
            labels.push(("action", action));
            metrics::counter!(names::LLM_TRUNCATION_GATE_TOTAL, &labels).increment(1);
        }
        if self.retries > 0 {
            metrics::counter!(names::LLM_RETRIES_TOTAL, "provider" => provider.to_string())
                .increment(u64::from(self.retries));
        }
        if let Some(wait) = self.retry_wait_secs {
            metrics::histogram!(names::LLM_RETRY_WAIT_DURATION, "provider" => provider.to_string())
                .record(wait);
        }
    }
}

/// Event listener that records per-instance LLM/tool counters and duration histograms.
///
/// Counters are naturally partitioned per replica — each instance only counts
/// events it processes. No double-counting under horizontal scaling.
pub struct PrometheusMetricsListener;

#[async_trait]
impl EventListener for PrometheusMetricsListener {
    async fn on_event(&self, event: &Event) {
        match &event.data {
            EventData::LlmGeneration(data) => {
                let provider = data
                    .metadata
                    .provider
                    .as_deref()
                    .unwrap_or("unknown")
                    .to_string();
                let model = data.metadata.model.clone();

                // Counter: always increment (even if duration is unknown)
                metrics::counter!(
                    names::LLM_REQUESTS_TOTAL,
                    "provider" => provider.clone(),
                    "model" => model.clone(),
                )
                .increment(1);

                // Histogram: only when duration is available
                if let Some(duration_ms) = data.metadata.duration_ms {
                    metrics::histogram!(
                        names::LLM_REQUEST_DURATION,
                        "provider" => provider.clone(),
                        "model" => model.clone(),
                    )
                    .record(duration_ms as f64 / 1000.0);
                }
                if data.metadata.success {
                    LlmOutcomeSample::of(&data.metadata).record(&provider, &model);
                }
            }
            EventData::ToolCompleted(data) => {
                let tool = data.tool_name.clone();

                metrics::counter!(
                    names::TOOL_EXECUTIONS_TOTAL,
                    "tool" => tool.clone(),
                )
                .increment(1);

                if let Some(duration_ms) = data.duration_ms {
                    metrics::histogram!(
                        names::TOOL_EXECUTION_DURATION,
                        "tool" => tool,
                    )
                    .record(duration_ms as f64 / 1000.0);
                }
            }
            _ => {}
        }
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![LLM_GENERATION, TOOL_COMPLETED])
    }

    fn name(&self) -> &'static str {
        "PrometheusMetricsListener"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generation(finish: &str) -> everruns_core::events::LlmGenerationData {
        everruns_core::events::LlmGenerationData::success_with_metadata(
            Vec::new(),
            Vec::new(),
            None,
            Vec::new(),
            "claude-sonnet-4-6".into(),
            Some("anthropic".into()),
            None,
            None,
            None,
            Some(vec![finish.to_string()]),
            None,
        )
    }

    #[test]
    fn llm_outcome_counts_truncation_and_retries() {
        let data = generation("length")
            .with_stop_details(Some("max_tokens".into()), 2, 1)
            .with_truncation_gate(Some("retried"))
            .with_retry(everruns_core::events::LlmRetryInfo {
                attempts: 2,
                total_wait_ms: 1500,
            });
        assert_eq!(
            LlmOutcomeSample::of(&data.metadata),
            LlmOutcomeSample {
                finish_reason: Some("length".into()),
                tool_calls_dropped: 2,
                tool_calls_truncated_executed: 1,
                truncation_gate: Some("retried".into()),
                retries: 2,
                retry_wait_secs: Some(1.5),
            }
        );
    }

    #[test]
    fn llm_outcome_of_a_clean_generation_counts_only_its_finish_reason() {
        assert_eq!(
            LlmOutcomeSample::of(&generation("stop").metadata),
            LlmOutcomeSample {
                finish_reason: Some("stop".into()),
                ..LlmOutcomeSample::default()
            }
        );
    }

    #[test]
    fn config_defaults_safe() {
        // Note: this test reads real env vars; METRICS_ENABLED unset → defaults true.
        // If CI sets METRICS_ENABLED=false this will fail — intentional canary.
        let config = PrometheusConfig::from_env();
        assert!(config.enabled);
        assert!(!config.public_on_main);
    }
}

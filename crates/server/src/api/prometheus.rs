// Prometheus /metrics endpoint
//
// Decision: Uses `metrics` + `metrics-exporter-prometheus` (lighter than OTel bridge).
// Decision: Metrics are always enabled by default (METRICS_ENABLED=true).
// Decision: Two serving modes to keep metrics internal in production:
//   1. METRICS_ADDR set (e.g. 127.0.0.1:9090) → dedicated internal-only HTTP server
//      serving only /metrics. Not reachable from outside the pod/host. This is the
//      recommended production pattern; scrapers access via sidecar or pod-local.
//   2. METRICS_ADDR unset → /metrics mounted on the main API server (convenient
//      for dev/local). In production without METRICS_ADDR, restrict via network
//      policy or ingress rules.
// Decision: No auth on /metrics (Prometheus standard).
// Decision: Horizontal scaling model:
//   - Gauges from DB (each replica reports the same logical value; Prometheus
//     keeps one series per `instance`, so queries for cluster-level values
//     should aggregate, e.g. `max without(instance)`)
//   - Counters from local events only (each replica counts its own work — no
//     double-counting across replicas; use `sum without(instance)` for totals)
//   - Histograms from local observations (naturally partitioned per instance)

use axum::http::header;
use axum::response::IntoResponse;
use axum::{Router, extract::State, routing::get};
use everruns_config::{env_bool, env_opt_string};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

/// Configuration for the Prometheus metrics endpoint.
pub struct PrometheusConfig {
    /// Whether metrics collection is enabled.
    pub enabled: bool,
    /// Optional separate bind address for the metrics server (e.g. "127.0.0.1:9090").
    /// When set, /metrics is served on a dedicated internal-only HTTP server
    /// instead of the main API server — keeping it off the public interface.
    pub metrics_addr: Option<String>,
}

impl PrometheusConfig {
    /// Load from environment.
    ///
    /// - `METRICS_ENABLED`: enable/disable metrics (default: true)
    /// - `METRICS_ADDR`: separate bind address for internal-only metrics server
    ///   (e.g. "127.0.0.1:9090"). When set, /metrics is NOT mounted on the main
    ///   API server. Recommended for production to avoid external exposure.
    pub fn from_env() -> Self {
        Self {
            enabled: env_bool("METRICS_ENABLED", true),
            metrics_addr: env_opt_string("METRICS_ADDR"),
        }
    }
}

/// Install the `metrics-exporter-prometheus` recorder and return the render handle.
///
/// Must be called exactly once before any `metrics::*!` macros are used.
/// Returns `None` if installation fails (e.g. another recorder is already set).
pub fn install_prometheus_recorder() -> Option<PrometheusHandle> {
    PrometheusBuilder::new()
        .install_recorder()
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
pub fn spawn_metrics_server(handle: PrometheusHandle, addr: String) {
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
    });
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

    // === Histograms (from local observations — per-instance) ===
    pub const HTTP_REQUEST_DURATION: &str = "everruns_http_request_duration_seconds";
    pub const LLM_REQUEST_DURATION: &str = "everruns_llm_request_duration_seconds";
    pub const TOOL_EXECUTION_DURATION: &str = "everruns_tool_execution_duration_seconds";
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
use everruns_core::events::{Event, EventData, LLM_GENERATION, TOOL_COMPLETED};

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
                        "provider" => provider,
                        "model" => model,
                    )
                    .record(duration_ms as f64 / 1000.0);
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

    #[test]
    fn config_defaults_enabled() {
        // Note: this test reads real env vars; METRICS_ENABLED unset → defaults true.
        // If CI sets METRICS_ENABLED=false this will fail — intentional canary.
        let config = PrometheusConfig::from_env();
        assert!(config.enabled);
    }
}

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
// Decision: Gauges/counters bridged from existing MetricsCollector via background sampler.
//           Histograms recorded inline via `metrics` macros.

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
    // Gauges (bridged from MetricsCollector)
    pub const WORKFLOWS_RUNNING: &str = "everruns_workflows_running";
    pub const WORKFLOWS_PENDING: &str = "everruns_workflows_pending";
    pub const TASKS_PENDING: &str = "everruns_tasks_pending";
    pub const TASKS_CLAIMED: &str = "everruns_tasks_claimed";
    pub const WORKERS_ACTIVE: &str = "everruns_workers_active";
    pub const LOAD_RATIO: &str = "everruns_load_ratio";
    pub const DLQ_SIZE: &str = "everruns_dlq_size";

    // Counters (monotonic, emitted as deltas from DB cumulative totals)
    pub const TASKS_TOTAL: &str = "everruns_tasks_total";
    pub const WORKFLOWS_TOTAL: &str = "everruns_workflows_total";

    // Histograms
    pub const HTTP_REQUEST_DURATION: &str = "everruns_http_request_duration_seconds";
    pub const LLM_REQUEST_DURATION: &str = "everruns_llm_request_duration_seconds";
    pub const TOOL_EXECUTION_DURATION: &str = "everruns_tool_execution_duration_seconds";
}

// ============================================================================
// Gauge bridge: MetricsCollector → Prometheus gauges + counters
// ============================================================================

use super::durable::MetricsCollector;

/// Spawn a background task that copies the latest MetricsCollector snapshot
/// into Prometheus gauges/counters every 10 seconds (aligned with the sampler).
pub fn spawn_gauge_bridge(collector: MetricsCollector) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        tracing::info!("Prometheus gauge bridge started (10s interval)");

        // Track last-seen counter values to emit deltas (counters are monotonic)
        let mut last_tasks_completed: u64 = 0;
        let mut last_tasks_failed: u64 = 0;
        let mut last_tasks_started: u64 = 0;
        let mut last_workflows_completed: u64 = 0;
        let mut last_workflows_failed: u64 = 0;
        let mut last_workflows_started: u64 = 0;

        loop {
            interval.tick().await;

            let Some(latest) = collector.get_latest().await else {
                continue;
            };

            // Gauges — absolute values
            metrics::gauge!(names::WORKFLOWS_RUNNING).set(latest.running_workflows as f64);
            metrics::gauge!(names::WORKFLOWS_PENDING).set(latest.pending_workflows as f64);
            metrics::gauge!(names::TASKS_PENDING).set(latest.pending_tasks as f64);
            metrics::gauge!(names::TASKS_CLAIMED).set(latest.claimed_tasks as f64);
            metrics::gauge!(names::WORKERS_ACTIVE).set(latest.active_workers as f64);
            metrics::gauge!(names::LOAD_RATIO).set(latest.load_percentage / 100.0);
            metrics::gauge!(names::DLQ_SIZE).set(latest.dlq_size as f64);

            // Counters — emit deltas from DB cumulative totals so Prometheus
            // sees a proper monotonic counter (compatible with rate() queries).
            let emit_delta =
                |name: &'static str, label: &'static str, current: u64, last: &mut u64| {
                    if current >= *last {
                        let delta = current - *last;
                        if delta > 0 {
                            metrics::counter!(name, "status" => label).increment(delta);
                        }
                        *last = current;
                    }
                };

            emit_delta(
                names::TASKS_TOTAL,
                "completed",
                latest.tasks_completed_total,
                &mut last_tasks_completed,
            );
            emit_delta(
                names::TASKS_TOTAL,
                "failed",
                latest.tasks_failed_total,
                &mut last_tasks_failed,
            );
            emit_delta(
                names::TASKS_TOTAL,
                "started",
                latest.tasks_started_total,
                &mut last_tasks_started,
            );
            emit_delta(
                names::WORKFLOWS_TOTAL,
                "completed",
                latest.workflows_completed_total,
                &mut last_workflows_completed,
            );
            emit_delta(
                names::WORKFLOWS_TOTAL,
                "failed",
                latest.workflows_failed_total,
                &mut last_workflows_failed,
            );
            emit_delta(
                names::WORKFLOWS_TOTAL,
                "started",
                latest.workflows_started_total,
                &mut last_workflows_started,
            );
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
    metrics::histogram!(
        names::HTTP_REQUEST_DURATION,
        "method" => method.to_string(),
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

/// Event listener that records LLM and tool execution duration histograms.
pub struct PrometheusMetricsListener;

#[async_trait]
impl EventListener for PrometheusMetricsListener {
    async fn on_event(&self, event: &Event) {
        match &event.data {
            EventData::LlmGeneration(data) => {
                if let Some(duration_ms) = data.metadata.duration_ms {
                    let provider = data
                        .metadata
                        .provider
                        .as_deref()
                        .unwrap_or("unknown")
                        .to_string();
                    let model = data.metadata.model.clone();
                    metrics::histogram!(
                        names::LLM_REQUEST_DURATION,
                        "provider" => provider,
                        "model" => model,
                    )
                    .record(duration_ms as f64 / 1000.0);
                }
            }
            EventData::ToolCompleted(data) => {
                if let Some(duration_ms) = data.duration_ms {
                    let tool = data.tool_name.clone();
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

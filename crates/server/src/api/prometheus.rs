// Prometheus /metrics endpoint
//
// Decision: Uses `metrics` + `metrics-exporter-prometheus` (lighter than OTel bridge).
// Decision: Endpoint is mounted outside the API prefix at `/metrics` (like `/health`).
// Decision: No auth on /metrics — restrict via network policy in production.
//           In SaaS (DeploymentGrade::Prod without METRICS_ENABLED=true), the endpoint
//           is disabled by default to prevent external exposure.
// Decision: Gauges/counters bridged from existing MetricsCollector via background sampler.
//           Histograms recorded inline via `metrics` macros.

use axum::http::header;
use axum::response::IntoResponse;
use axum::{Router, extract::State, routing::get};
use everruns_config::env_bool;
use everruns_core::DeploymentGrade;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

/// Configuration for the Prometheus metrics endpoint.
pub struct PrometheusConfig {
    /// Whether the /metrics endpoint is enabled.
    pub enabled: bool,
    /// Metric name prefix (default: "everruns").
    pub prefix: String,
}

impl PrometheusConfig {
    /// Load from environment, applying deployment-grade defaults.
    ///
    /// - `METRICS_ENABLED`: explicit override (default: true for Dev/Poc/Preview, false for Prod)
    /// - `METRICS_PREFIX`: metric name prefix (default: "everruns")
    pub fn from_env(grade: &DeploymentGrade) -> Self {
        // In prod/SaaS, default off to avoid external exposure.
        // Operators opt in by setting METRICS_ENABLED=true behind a network policy.
        let default_enabled = !matches!(grade, DeploymentGrade::Prod);
        Self {
            enabled: env_bool("METRICS_ENABLED", default_enabled),
            prefix: std::env::var("METRICS_PREFIX").unwrap_or_else(|_| "everruns".to_string()),
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

    // Counters (absolute values set from DB cumulative totals)
    pub const TASKS_TOTAL: &str = "everruns_tasks_total";
    pub const WORKFLOWS_TOTAL: &str = "everruns_workflows_total";

    // Histograms
    pub const HTTP_REQUEST_DURATION: &str = "everruns_http_request_duration_seconds";
    pub const LLM_REQUEST_DURATION: &str = "everruns_llm_request_duration_seconds";
    pub const TOOL_EXECUTION_DURATION: &str = "everruns_tool_execution_duration_seconds";
}

// ============================================================================
// Gauge bridge: MetricsCollector → Prometheus gauges
// ============================================================================

use super::durable::MetricsCollector;

/// Spawn a background task that copies the latest MetricsCollector snapshot
/// into Prometheus gauges/counters every 10 seconds (aligned with the sampler).
pub fn spawn_gauge_bridge(collector: MetricsCollector) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        tracing::info!("Prometheus gauge bridge started (10s interval)");

        loop {
            interval.tick().await;

            let points = collector.get_points().await;
            let Some(latest) = points.last() else {
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

            // Counters — set to absolute DB totals.
            // The `metrics` crate counters only support increment, so we track
            // the delta from the last emitted value.  Use absolute() on gauge
            // instead to emit the raw DB counter (Prometheus can compute rate).
            metrics::gauge!(names::TASKS_TOTAL, "status" => "completed")
                .set(latest.tasks_completed_total as f64);
            metrics::gauge!(names::TASKS_TOTAL, "status" => "failed")
                .set(latest.tasks_failed_total as f64);
            metrics::gauge!(names::TASKS_TOTAL, "status" => "started")
                .set(latest.tasks_started_total as f64);
            metrics::gauge!(names::WORKFLOWS_TOTAL, "status" => "completed")
                .set(latest.workflows_completed_total as f64);
            metrics::gauge!(names::WORKFLOWS_TOTAL, "status" => "failed")
                .set(latest.workflows_failed_total as f64);
            metrics::gauge!(names::WORKFLOWS_TOTAL, "status" => "started")
                .set(latest.workflows_started_total as f64);
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
pub async fn http_metrics_layer(
    matched_path: Option<MatchedPath>,
    req: axum::extract::Request,
    next: Next,
) -> impl IntoResponse {
    let method = req.method().clone();
    let path = matched_path
        .map(|mp| mp.as_str().to_owned())
        .unwrap_or_else(|| req.uri().path().to_owned());

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
    fn config_dev_defaults_enabled() {
        // Dev grade should default to enabled
        let config = PrometheusConfig::from_env(&DeploymentGrade::Dev);
        assert!(config.enabled);
        assert_eq!(config.prefix, "everruns");
    }

    #[test]
    fn config_prod_defaults_disabled() {
        let config = PrometheusConfig::from_env(&DeploymentGrade::Prod);
        // Without env var override, prod defaults to disabled
        assert!(!config.enabled);
    }
}

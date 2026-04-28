// HTTP access log middleware
//
// Emits one structured tracing event per HTTP request with method, matched
// route, status, latency, and the correlation `request_id` set by
// `RequestIdLayer`. Mirrors the field set so a single
// `request_id=<x>` grep returns the wire-side line plus every child span
// (LLM calls, DB queries, durable activities) that runs under the
// per-request span built by `TraceLayer` in `app_builder.rs`.
//
// Decisions:
// - Uses `MatchedPath` (e.g. `/v1/sessions/{id}`) instead of the raw URI to
//   keep log/index cardinality bounded. Falls back to `unmatched` for 404s
//   so scanners cannot blow up downstream indexes.
// - Must be applied as `route_layer` so axum populates `MatchedPath` before
//   this middleware runs.
// - Health and metrics endpoints are downgraded to DEBUG to keep INFO
//   clean. Operators can promote them with `RUST_LOG`.
// - 5xx responses log at WARN; everything else logs at INFO (or DEBUG for
//   noise paths). The fields are identical regardless of level.

use crate::middleware::request_id::RequestId;
use axum::extract::MatchedPath;
use axum::middleware::Next;
use axum::response::Response;

const NOISE_PATHS: &[&str] = &["/health", "/metrics"];

/// Axum middleware that emits one tracing event per HTTP request.
///
/// Apply with `route_layer(axum::middleware::from_fn(http_access_log_layer))`
/// so `MatchedPath` is available for the route field.
pub async fn http_access_log_layer(
    matched_path: Option<MatchedPath>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let route = matched_path
        .map(|mp| mp.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".to_owned());
    let request_id = req
        .extensions()
        .get::<RequestId>()
        .map(|r| r.0.clone())
        .unwrap_or_default();

    let start = std::time::Instant::now();
    let response = next.run(req).await;
    let latency_ms = start.elapsed().as_millis() as u64;
    let status = response.status().as_u16();

    let is_noise = NOISE_PATHS.iter().any(|p| route == *p);

    if status >= 500 {
        tracing::warn!(
            method = %method,
            route = %route,
            status = status,
            latency_ms = latency_ms,
            request_id = %request_id,
            "http request failed",
        );
    } else if is_noise {
        tracing::debug!(
            method = %method,
            route = %route,
            status = status,
            latency_ms = latency_ms,
            request_id = %request_id,
            "http request",
        );
    } else {
        tracing::info!(
            method = %method,
            route = %route,
            status = status,
            latency_ms = latency_ms,
            request_id = %request_id,
            "http request",
        );
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;
    use tracing::subscriber::with_default;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::{Layer, Registry};

    /// A tracing layer that captures every event into a shared buffer so
    /// tests can assert on field values.
    #[derive(Clone, Default)]
    struct CaptureLayer {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    #[derive(Clone, Debug)]
    struct CapturedEvent {
        level: tracing::Level,
        fields: std::collections::HashMap<String, String>,
    }

    impl<S> Layer<S> for CaptureLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = FieldVisitor::default();
            event.record(&mut visitor);
            self.events.lock().unwrap().push(CapturedEvent {
                level: *event.metadata().level(),
                fields: visitor.fields,
            });
        }
    }

    #[derive(Default)]
    struct FieldVisitor {
        fields: std::collections::HashMap<String, String>,
    }

    impl tracing::field::Visit for FieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.fields
                .insert(field.name().to_string(), format!("{:?}", value));
        }
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }
        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }
        fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }
        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }
    }

    fn build_app() -> Router {
        async fn ok() -> &'static str {
            "ok"
        }
        async fn boom() -> (StatusCode, &'static str) {
            (StatusCode::INTERNAL_SERVER_ERROR, "kaboom")
        }
        Router::new()
            .route("/users/{id}", get(ok))
            .route("/health", get(ok))
            .route("/boom", get(boom))
            .route_layer(axum::middleware::from_fn(http_access_log_layer))
    }

    fn run_request(app: Router, uri: &str, request_id: &str) -> CaptureLayer {
        let capture = CaptureLayer::default();
        let subscriber = Registry::default().with(capture.clone());
        with_default(subscriber, || {
            let req = Request::builder()
                .uri(uri)
                .extension(RequestId(request_id.to_string()))
                .body(Body::empty())
                .unwrap();
            futures::executor::block_on(async {
                app.oneshot(req).await.unwrap();
            });
        });
        capture
    }

    #[test]
    fn emits_info_event_with_method_route_status_latency_request_id() {
        let capture = run_request(build_app(), "/users/42", "req-abc");
        let events = capture.events.lock().unwrap().clone();
        assert_eq!(events.len(), 1, "expected exactly one event");
        let evt = &events[0];
        assert_eq!(evt.level, tracing::Level::INFO);
        assert_eq!(evt.fields.get("method").map(String::as_str), Some("GET"));
        assert_eq!(
            evt.fields.get("route").map(String::as_str),
            Some("/users/{id}")
        );
        assert_eq!(evt.fields.get("status").map(String::as_str), Some("200"));
        assert!(evt.fields.contains_key("latency_ms"));
        assert_eq!(
            evt.fields.get("request_id").map(String::as_str),
            Some("req-abc")
        );
    }

    #[test]
    fn downgrades_health_to_debug() {
        let capture = run_request(build_app(), "/health", "req-health");
        let events = capture.events.lock().unwrap().clone();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].level, tracing::Level::DEBUG);
        assert_eq!(
            events[0].fields.get("route").map(String::as_str),
            Some("/health")
        );
    }

    #[test]
    fn upgrades_5xx_to_warn_with_same_request_id() {
        let capture = run_request(build_app(), "/boom", "req-boom");
        let events = capture.events.lock().unwrap().clone();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].level, tracing::Level::WARN);
        assert_eq!(
            events[0].fields.get("status").map(String::as_str),
            Some("500")
        );
        assert_eq!(
            events[0].fields.get("request_id").map(String::as_str),
            Some("req-boom")
        );
    }

    #[test]
    fn unmatched_route_does_not_log_via_route_layer() {
        // Documented behaviour: `route_layer` only fires on matched routes
        // (axum requires this so `MatchedPath` is populated). 404s for
        // unmatched paths therefore do not produce an access-log line — the
        // TraceLayer span around the request still emits its own record.
        let capture = run_request(build_app(), "/totally/unknown/path", "req-404");
        let events = capture.events.lock().unwrap().clone();
        assert!(events.is_empty(), "expected no events for unmatched route");
    }
}

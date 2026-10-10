// HTTP middleware stacks applied around the routers.
//
// Decision: layer order is load-bearing. Each `.layer` call wraps outside the
//   ones before it, so the last layer runs first on a request. Keep the order
//   below when adding a layer (see the RequestIdLayer note at the end).

use crate::api;
use crate::api::agent_api_cors::ApiChannelOrigins;
use crate::auth::rate_limit::{ApiRateLimiter, api_rate_limit_middleware};
use crate::domains;
use crate::middleware::RequestIdLayer;
use crate::middleware::request_id::RequestId;
use crate::records::FeatureFlags;
use axum::Router;
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::middleware::from_fn;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

/// Response decoration for the `/v1` API surface: entity links, pagination
/// links, problem details, and change-intent capture.
pub(super) fn decorate_api_routes(
    api_routes: Router,
    auth_config: &crate::auth::AuthConfig,
) -> Router {
    let link_builder = api::common::UrlBuilder::from_auth_config(auth_config);
    let pagination_builder = link_builder.clone();
    let api_routes = api_routes.layer(from_fn(move |req, next| {
        let link_builder = link_builder.clone();
        api::common::decorate_json_response_links(link_builder, req, next)
    }));

    // AI-friendly: add `next_url` / `prev_url` to paginated list responses.
    // Layered after entity-link decoration so both can run; only mutates
    // objects shaped like PaginatedResponse.
    let api_routes = api_routes.layer(from_fn(move |req, next| {
        let builder = pagination_builder.clone();
        api::common::decorate_pagination_links(builder, req, next)
    }));

    // RFC 9457: rewrite JSON error responses (4xx/5xx) to `problem+json` and
    // mirror retry metadata into `Retry-After`; runs after link decoration,
    // which only touches success responses. Then capture
    // `Everruns-Change-Reason` for the commands a request runs.
    api_routes
        .layer(from_fn(api::problem_details::standard_error_headers))
        .layer(from_fn(domains::change_history::http_change_intent_layer))
}

/// The shared per-IP limiter, or `None` when
/// `RATE_LIMIT_API_REQUESTS_PER_MINUTE=0` disables it.
pub(super) fn api_rate_limiter(
    valkey: Option<crate::valkey::ValkeyClient>,
) -> Option<ApiRateLimiter> {
    if ApiRateLimiter::is_disabled() {
        tracing::info!("API rate limiting disabled via RATE_LIMIT_API_REQUESTS_PER_MINUTE=0");
        return None;
    }
    Some(ApiRateLimiter::from_env_with_valkey(valkey))
}

/// Apply the per-IP limiter to `router`; identity when limiting is disabled.
pub(super) fn rate_limit(router: Router, limiter: Option<&ApiRateLimiter>) -> Router {
    match limiter {
        Some(limiter) => {
            let limiter = limiter.clone();
            router.layer(from_fn(move |req, next| {
                let limiter = limiter.clone();
                api_rate_limit_middleware(limiter, req, next)
            }))
        }
        None => router,
    }
}

/// CORS for cross-origin browser clients (TM-API-007): an explicit origin
/// list with credentials, plus each `api` channel's own origins for that
/// channel's routes (see `api::agent_api_cors`).
///
/// Decision: allow every request header the API reads, not just the standard
/// ones. A cross-origin browser client (the TypeScript SDK in a web app) sends
/// `X-Org-Id`, `Idempotency-Key` and the change-intent headers; a header
/// missing here fails the preflight and the browser drops the whole request.
fn cors_layer(
    cors_origins: &[HeaderValue],
    channel_origins: Option<ApiChannelOrigins>,
) -> CorsLayer {
    use domains::change_history::intent::{CHANGE_REASON_HEADER, CONTEXT_REVISION_HEADER};
    let global: Arc<[HeaderValue]> = cors_origins.into();
    let allow_origin = AllowOrigin::async_predicate(move |origin, parts| {
        let global = global.clone();
        let channel_origins = channel_origins.clone();
        let path = parts.uri.path().to_owned();
        async move {
            if global.contains(&origin) {
                return true;
            }
            match channel_origins {
                Some(channel_origins) => channel_origins.allows(&path, &origin).await,
                None => false,
            }
        }
    });
    CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::ACCEPT,
            header::ORIGIN,
            header::CACHE_CONTROL,
            HeaderName::from_static("x-org-id"),
            HeaderName::from_static(api::command_dispatch::IDEMPOTENCY_KEY_HEADER),
            HeaderName::from_static(CHANGE_REASON_HEADER),
            HeaderName::from_static(CONTEXT_REVISION_HEADER),
        ])
        .expose_headers([HeaderName::from_static("idempotent-replayed")])
        .allow_credentials(true)
}

/// Outermost layers on the whole app: CORS, security headers, tracing, the
/// access log, request metrics, and the request ID.
pub(super) fn apply_outer_layers(
    app: Router,
    cors_origins: &[HeaderValue],
    channel_origins: ApiChannelOrigins,
    feature_flags: &FeatureFlags,
    prometheus_enabled: bool,
) -> Router {
    // CORS: always installed, since any api channel may list origins.
    let app = app.layer(cors_layer(cors_origins, Some(channel_origins)));

    // TM-WEB-004/005: Security response headers
    let app = app
        .layer(SetResponseHeaderLayer::if_not_present(
            axum::http::header::X_FRAME_OPTIONS,
            axum::http::HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            axum::http::header::X_CONTENT_TYPE_OPTIONS,
            axum::http::HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            axum::http::header::REFERRER_POLICY,
            axum::http::HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            axum::http::header::HeaderName::from_static("permissions-policy"),
            crate::security_headers::permissions_policy_header_value(
                feature_flags.voice,
                feature_flags.webmcp,
            ),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            axum::http::header::HeaderName::from_static("content-security-policy"),
            axum::http::HeaderValue::from_static(
                crate::security_headers::BASE_CONTENT_SECURITY_POLICY,
            ),
        ));

    let app = app.layer(TraceLayer::new_for_http().make_span_with(
        |req: &axum::http::Request<_>| {
            let request_id = req
                .extensions()
                .get::<RequestId>()
                .map(|r| r.0.as_str())
                .unwrap_or("")
                .to_string();
            tracing::info_span!(
                "http_request",
                method = %req.method(),
                uri = %req.uri().path(),
                request_id = %request_id,
                session_id = tracing::field::Empty,
            )
        },
    ));

    // Per-request access log: applied as route_layer so axum's MatchedPath
    // extractor is available for low-cardinality `route` labels. Emits one
    // tracing event per request with method, route, status, latency_ms,
    // and request_id (DEBUG for /health and /metrics, WARN for 5xx,
    // INFO otherwise). See EVE-399 / knowledge/operations/correlation-ids.md.
    let app = app.route_layer(from_fn(crate::middleware::http_access_log_layer));

    // HTTP request duration histogram: applied as route_layer so axum's
    // MatchedPath extractor is available for low-cardinality path labels.
    let app = if prometheus_enabled {
        app.route_layer(from_fn(api::prometheus::http_metrics_layer))
    } else {
        app
    };

    // RequestIdLayer must be applied LAST of this group, because each call
    // wraps outside what came before: it has to run first so TraceLayer's
    // span and the access log above can both read the ID it inserts.
    // EVE-1075 was exactly this — applied before the access log, it ended
    // up inside it, and every production access-log line carried
    // `request_id=""`. `the_logged_request_id_is_the_one_the_response_echoes`
    // in `middleware/access_log.rs` pins the order.
    // See knowledge/operations/correlation-ids.md.
    app.layer(RequestIdLayer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::post;
    use tower::ServiceExt;

    const ORIGIN: &str = "https://app.example.com";

    async fn preflight(request_headers: &str) -> axum::http::Response<Body> {
        let app = Router::new()
            .route("/v1/agents", post(|| async { "ok" }))
            .layer(cors_layer(&[HeaderValue::from_static(ORIGIN)], None));
        app.oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/v1/agents")
                .header(header::ORIGIN, ORIGIN)
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(header::ACCESS_CONTROL_REQUEST_HEADERS, request_headers)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    fn allowed_headers(response: &axum::http::Response<Body>) -> String {
        response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase()
    }

    #[tokio::test]
    async fn preflight_allows_the_headers_api_clients_send() {
        let response = preflight(
            "content-type,authorization,x-org-id,idempotency-key,everruns-change-reason,everruns-context-revision",
        )
        .await;
        let allowed = allowed_headers(&response);
        for name in [
            "x-org-id",
            "idempotency-key",
            "everruns-change-reason",
            "everruns-context-revision",
        ] {
            assert!(allowed.contains(name), "{name} missing from {allowed}");
        }
        assert_eq!(
            response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static(ORIGIN))
        );
    }

    #[tokio::test]
    async fn preflight_does_not_allow_unlisted_headers() {
        let response = preflight("x-something-else").await;
        assert!(!allowed_headers(&response).contains("x-something-else"));
    }
}

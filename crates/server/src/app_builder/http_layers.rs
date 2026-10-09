// HTTP middleware stacks applied around the routers.
//
// Decision: layer order is load-bearing. Each `.layer` call wraps outside the
//   ones before it, so the last layer runs first on a request. Keep the order
//   below when adding a layer (see the RequestIdLayer note at the end).

use crate::api;
use crate::auth::rate_limit::{ApiRateLimiter, api_rate_limit_middleware};
use crate::domains;
use crate::middleware::RequestIdLayer;
use crate::middleware::request_id::RequestId;
use crate::records::FeatureFlags;
use axum::Router;
use axum::http::{HeaderValue, Method, header};
use axum::middleware::from_fn;
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

/// Outermost layers on the whole app: CORS, security headers, tracing, the
/// access log, request metrics, and the request ID.
pub(super) fn apply_outer_layers(
    app: Router,
    cors_origins: &[HeaderValue],
    feature_flags: &FeatureFlags,
    prometheus_enabled: bool,
) -> Router {
    // CORS
    let app = if !cors_origins.is_empty() {
        app.layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(cors_origins.to_vec()))
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
                ])
                .allow_credentials(true),
        )
    } else {
        app
    };

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

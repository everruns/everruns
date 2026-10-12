// Route classification layer for personal access token scopes.
//
// Decision: one `from_fn` layer on the assembled `/v1` router reads axum's
//   `MatchedPath` (set by routing before route layers run) and the method,
//   strips the API prefix, and inserts the route's `RouteAccess` as a request
//   extension. It never looks at credentials, and never answers a request
//   itself: a missing extension simply means "unclassified".
// Decision: embedder classifications (`ExtraRouteClasses`) only fill in what
//   the built-in classification leaves unclassified.

use std::sync::Arc;

use axum::Router;
use axum::extract::{MatchedPath, Request};
use axum::middleware::{Next, from_fn};
use axum::response::Response;

use super::{ExtraRouteClasses, classify};

/// Classify every route of the `/v1` router. `api_prefix` is the prefix the
/// router is nested under (`ServerConfig::api_prefix`, may be empty);
/// `extra` holds the embedder's classifications.
pub fn classify_api_routes(
    api_routes: Router,
    api_prefix: &str,
    extra: ExtraRouteClasses,
) -> Router {
    let prefix: Arc<str> = Arc::from(api_prefix.trim_end_matches('/'));
    let extra = Arc::new(extra);
    api_routes.layer(from_fn(move |req, next| {
        let prefix = prefix.clone();
        let extra = extra.clone();
        insert_route_access(prefix, extra, req, next)
    }))
}

async fn insert_route_access(
    prefix: Arc<str>,
    extra: Arc<ExtraRouteClasses>,
    mut req: Request,
    next: Next,
) -> Response {
    let access = req.extensions().get::<MatchedPath>().and_then(|matched| {
        let template = matched.as_str();
        let template = template.strip_prefix(&*prefix).unwrap_or(template);
        classify(req.method(), template).or_else(|| extra.get(req.method(), template))
    });
    if let Some(access) = access {
        req.extensions_mut().insert(access);
    }
    next.run(req).await
}

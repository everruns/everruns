// Route classification layer for personal access token scopes.
//
// Decision: one `from_fn` layer on the assembled `/v1` router reads axum's
//   `MatchedPath` (set by routing before route layers run) and the method,
//   strips the API prefix, and inserts the route's `RouteAccess` as a request
//   extension. It never looks at credentials, and never answers a request
//   itself: a missing extension simply means "unclassified".

use std::sync::Arc;

use axum::Router;
use axum::extract::{MatchedPath, Request};
use axum::middleware::{Next, from_fn};
use axum::response::Response;

use super::classify;

/// Classify every route of the `/v1` router. `api_prefix` is the prefix the
/// router is nested under (`ServerConfig::api_prefix`, may be empty).
pub fn classify_api_routes(api_routes: Router, api_prefix: &str) -> Router {
    let prefix: Arc<str> = Arc::from(api_prefix.trim_end_matches('/'));
    api_routes.layer(from_fn(move |req, next| {
        let prefix = prefix.clone();
        insert_route_access(prefix, req, next)
    }))
}

async fn insert_route_access(prefix: Arc<str>, mut req: Request, next: Next) -> Response {
    let access = req.extensions().get::<MatchedPath>().and_then(|matched| {
        let template = matched.as_str();
        let template = template.strip_prefix(&*prefix).unwrap_or(template);
        classify(req.method(), template)
    });
    if let Some(access) = access {
        req.extensions_mut().insert(access);
    }
    next.run(req).await
}

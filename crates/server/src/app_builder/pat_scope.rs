// Embedder hook for personal access token scope classification
// (knowledge/security/pat-scopes.md, Decision 2 "Fail closed").
//
// Decision: routes merged in by an embedder (`routes()`, a backend's
//   `auth_routes()`) are unclassified unless registered here, so a narrow
//   token is denied them. The built-in classification wins over anything
//   registered, so this can only open routes the server crate does not know.

use super::ServerAppBuilder;
use crate::auth::pat_scope::RouteAccess;
use axum::Router;

pub(super) type PersonalAccessTokenRoutesWrapFn = Box<dyn FnOnce(Router) -> Router + Send>;

/// Apply the optional personal-access-token-routes wrap closure to the router.
/// Identity when no wrap is configured.
pub(super) fn apply_personal_access_token_routes_wrap(
    wrap: Option<PersonalAccessTokenRoutesWrapFn>,
    router: Router,
) -> Router {
    match wrap {
        Some(wrap) => wrap(router),
        None => router,
    }
}

impl ServerAppBuilder {
    /// Classify embedder routes for personal access token scopes.
    ///
    /// Each row is `(method, path, class)`, with `path` as the route is
    /// registered under the API prefix (for example `/v1/billing/usage`);
    /// path parameter names are ignored. Call multiple times to add more.
    /// A route registered nowhere stays denied to a `read` token.
    ///
    /// # Panics
    /// On `RouteAccess::PerCommand` or an invalid method.
    pub fn classify_pat_routes(mut self, rows: &[(&str, &str, RouteAccess)]) -> Self {
        for (method, path, access) in rows {
            self.pat_route_classes.insert(method, path, *access);
        }
        self
    }
}

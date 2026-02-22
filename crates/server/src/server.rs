// Server configuration and backward-compatible entrypoint
//
// Decision: ServerConfig stays here; orchestration logic moved to app_builder.rs
// Decision: run() kept as thin wrapper around ServerAppBuilder for backward compat

use crate::app_builder::ServerAppBuilder;
use crate::auth::AuthBackend;
use crate::storage::StorageBackend;

use anyhow::Result;
use axum::Router;
use axum::http::HeaderValue;
use std::sync::Arc;

/// Server configuration loaded from environment
pub struct ServerConfig {
    pub dev_mode: bool,
    pub no_migrations: bool,
    pub api_prefix: String,
    pub cors_origins: Vec<HeaderValue>,
    pub addr: String,
    pub grpc_addr: String,
}

impl ServerConfig {
    /// Load server configuration from environment variables
    pub fn from_env() -> Self {
        let dev_mode = std::env::var("DEV_MODE")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let api_prefix = std::env::var("API_PREFIX").unwrap_or_default();

        let cors_origins: Vec<HeaderValue> = std::env::var("CORS_ALLOWED_ORIGINS")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| s.split(',').filter_map(|s| s.trim().parse().ok()).collect())
            .unwrap_or_default();

        let addr = std::env::var("ADDR").unwrap_or_else(|_| "0.0.0.0:9000".to_string());
        let grpc_addr = std::env::var("GRPC_ADDR").unwrap_or_else(|_| "0.0.0.0:9001".to_string());

        Self {
            dev_mode,
            no_migrations: false,
            api_prefix,
            cors_origins,
            addr,
            grpc_addr,
        }
    }
}

/// Start the Everruns server (backward-compatible wrapper around `ServerAppBuilder`).
///
/// Prefer using `ServerAppBuilder` directly for new code — it exposes additional
/// extension points (event listeners, migrations, background tasks).
///
/// `config` — server configuration (dev mode, addresses, CORS, etc.)
/// `auth_factory` — creates an auth backend given the storage backend.
///   Use `|db| Arc::new(BuiltinAuthBackend::new(auth_config, db))` for OSS default.
/// `extra_routes` — additional routes merged into the API router (billing, etc.)
pub async fn run(
    config: ServerConfig,
    auth_factory: impl FnOnce(Arc<StorageBackend>) -> Arc<dyn AuthBackend> + Send + 'static,
    extra_routes: Option<Router>,
) -> Result<()> {
    let mut builder = ServerAppBuilder::new(config).auth(auth_factory);
    if let Some(routes) = extra_routes {
        builder = builder.routes(routes);
    }
    builder.run().await
}

/// Build router with optional API prefix
pub(crate) fn build_router_with_prefix<S: Clone + Send + Sync + 'static>(
    api_routes: Router<S>,
    api_prefix: &str,
) -> Router<S> {
    if api_prefix.is_empty() {
        api_routes
    } else {
        Router::new().nest(api_prefix, api_routes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, routing::get};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_routes() -> Router {
        Router::new().route("/v1/test", get(|| async { "ok" }))
    }

    #[tokio::test]
    async fn test_api_prefix_empty() {
        let app = build_router_with_prefix(test_routes(), "");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn test_api_prefix_set() {
        let app = build_router_with_prefix(test_routes(), "/api");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 404);
    }
}

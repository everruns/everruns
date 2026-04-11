// Server configuration and router helpers
//
// Decision: ServerConfig stays here; orchestration logic moved to app_builder.rs

use axum::Router;
use axum::http::HeaderValue;
use everruns_config::{env_bool, env_list, env_or, env_string};

pub const DEFAULT_API_PREFIX: &str = "/api";

/// Configurable resource limits for orgs, members, API keys.
///
/// Defaults are sane for self-hosted. SaaS overrides per plan.
#[derive(Debug, Clone)]
pub struct ResourceLimitsConfig {
    pub max_orgs_per_user: i64,
    pub max_members_per_org: i64,
    pub max_api_keys_per_user_per_org: i64,
}

impl Default for ResourceLimitsConfig {
    fn default() -> Self {
        Self {
            max_orgs_per_user: 5,
            max_members_per_org: 50,
            max_api_keys_per_user_per_org: 10,
        }
    }
}

impl ResourceLimitsConfig {
    pub fn from_env() -> Self {
        Self {
            max_orgs_per_user: env_or("RESOURCE_LIMIT_MAX_ORGS_PER_USER", 5),
            max_members_per_org: env_or("RESOURCE_LIMIT_MAX_MEMBERS_PER_ORG", 50),
            max_api_keys_per_user_per_org: env_or(
                "RESOURCE_LIMIT_MAX_API_KEYS_PER_USER_PER_ORG",
                10,
            ),
        }
    }
}

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
        Self {
            dev_mode: env_bool("DEV_MODE", false),
            no_migrations: false,
            api_prefix: std::env::var("API_PREFIX")
                .unwrap_or_else(|_| DEFAULT_API_PREFIX.to_string()),
            cors_origins: env_list::<String>("CORS_ALLOWED_ORIGINS")
                .into_iter()
                .filter_map(|s| s.parse::<HeaderValue>().ok())
                .collect(),
            addr: env_string("ADDR", "0.0.0.0:9000"),
            grpc_addr: env_string("WORKER_GRPC_ADDR", "0.0.0.0:9001"),
        }
    }
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

// Everruns API server
// Decision: Flexible auth with support for no-auth, admin-only, and full auth modes
// M2: Agent/Session/Messages model with Events as SSE notifications

mod grpc_service;

// Use modules from library
use everruns_control_plane::api;
use everruns_control_plane::auth;
use everruns_control_plane::openapi::ApiDoc;
use everruns_control_plane::services;
use everruns_control_plane::storage::{Database, EncryptionService};

use anyhow::{Context, Result};
use axum::http::{header, HeaderValue, Method};
use axum::{extract::State, routing::get, Json, Router};
use everruns_core::telemetry::{init_telemetry, TelemetryConfig};
use everruns_core::{EventListener, OtelEventListener};
use everruns_worker::{create_runner, RunnerConfig};
use serde::Serialize;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

/// App state shared across routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    auth_mode: String,
}

async fn health(State(state): State<HealthState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        auth_mode: state.auth_mode.clone(),
    })
}

/// State for health endpoint
#[derive(Clone)]
struct HealthState {
    auth_mode: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize telemetry with OpenTelemetry support
    // Configure via environment variables:
    // - OTEL_SERVICE_NAME: Service name (default: "everruns-control-plane")
    // - OTEL_EXPORTER_OTLP_ENDPOINT: OTLP endpoint (e.g., "http://localhost:4317")
    // - RUST_LOG: Log filter (default: "everruns_api=debug,tower_http=debug")
    let mut telemetry_config = TelemetryConfig::from_env();
    if telemetry_config.service_name == "everruns" {
        telemetry_config.service_name = "everruns-control-plane".to_string();
    }
    if telemetry_config.log_filter.is_none() {
        telemetry_config.log_filter = Some("everruns_api=debug,tower_http=debug".to_string());
    }
    telemetry_config.service_version = Some(env!("CARGO_PKG_VERSION").to_string());

    // Keep the guard alive for the lifetime of the application
    let _telemetry_guard = init_telemetry(telemetry_config);

    tracing::info!("everrun-api starting...");

    // Initialize database
    let database_url =
        std::env::var("DATABASE_URL").context("DATABASE_URL environment variable required")?;
    let db = Database::from_url(&database_url)
        .await
        .context("Failed to connect to database")?;
    tracing::info!("Connected to database");

    // Load runner configuration from environment
    let runner_config = RunnerConfig::from_env();

    // Create the agent runner (Temporal or Durable based on RUNNER_MODE)
    // Note: Runner no longer needs db - session status is managed by control-plane
    let runner = create_runner(&runner_config)
        .await
        .context("Failed to create agent runner")?;

    match runner_config.mode {
        everruns_worker::RunnerMode::Temporal => {
            tracing::info!(
                mode = %runner_config.mode,
                address = %runner_config.temporal_address(),
                namespace = %runner_config.temporal_namespace(),
                task_queue = %runner_config.temporal_task_queue(),
                "Using Temporal agent runner"
            );
        }
        everruns_worker::RunnerMode::Durable => {
            tracing::info!(
                mode = %runner_config.mode,
                "Using Durable execution engine runner (PostgreSQL-backed)"
            );
        }
    }

    // Create app state
    let db = Arc::new(db);

    // Initialize encryption service for API keys (optional - gracefully degrade if not configured)
    let encryption = match EncryptionService::from_env() {
        Ok(svc) => {
            tracing::info!("Encryption service initialized for API key storage");
            Some(Arc::new(svc))
        }
        Err(e) => {
            tracing::warn!("Encryption service not configured (SECRETS_ENCRYPTION_KEY not set): {}. API key storage disabled.", e);
            None
        }
    };

    // Load authentication configuration
    let auth_config = auth::AuthConfig::from_env();
    tracing::info!(
        mode = ?auth_config.mode,
        password_auth = auth_config.password_auth_enabled(),
        oauth = auth_config.oauth_enabled(),
        "Authentication configured"
    );

    // Create auth state
    let auth_state = auth::AuthState::new(auth_config.clone(), db.clone());

    // Create module-specific states
    let agents_state = api::agents::AppState::new(db.clone());
    let sessions_state = api::sessions::AppState::new(db.clone());
    let messages_state = api::messages::AppState::new(db.clone(), runner.clone());

    // Create event listeners for observability
    // OtelEventListener generates gen-ai semantic convention spans from events
    let otel_listener: Arc<dyn EventListener> = Arc::new(OtelEventListener::new());

    // Create EventService with listeners - shared between HTTP API and gRPC service
    let event_service = Arc::new(services::EventService::with_listeners(
        db.clone(),
        vec![otel_listener],
    ));

    let events_state = api::events::AppState {
        session_service: Arc::new(services::SessionService::new(db.clone())),
        event_service: event_service.clone(),
    };
    let llm_providers_state = api::llm_providers::AppState::new(db.clone(), encryption.clone());
    let llm_models_state = api::llm_models::AppState::new(db.clone());
    let capability_service = Arc::new(services::CapabilityService::new(db.clone()));
    let capabilities_state = api::capabilities::AppState::new(capability_service);
    let session_files_state = api::session_files::AppState::new(db.clone());
    let users_state = api::users::UsersState {
        db: db.clone(),
        auth: auth_state.clone(),
    };
    let health_state = HealthState {
        auth_mode: format!("{:?}", auth_config.mode),
    };

    // Load API prefix from environment (default: empty)
    // Example: API_PREFIX="/api" results in routes like /api/v1/agents
    let api_prefix = std::env::var("API_PREFIX").unwrap_or_default();
    if !api_prefix.is_empty() {
        tracing::info!(prefix = %api_prefix, "API prefix configured");
    }

    // Load CORS allowed origins from environment (optional)
    // Only needed when UI is served from a different origin than the API
    // Example: CORS_ALLOWED_ORIGINS="https://app.example.com,https://admin.example.com"
    let cors_origins: Vec<HeaderValue> = std::env::var("CORS_ALLOWED_ORIGINS")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.split(',').filter_map(|s| s.trim().parse().ok()).collect())
        .unwrap_or_default();

    if cors_origins.is_empty() {
        tracing::info!("CORS not configured (same-origin requests only)");
    } else {
        tracing::info!(origins = ?cors_origins, "CORS origins configured");
    }

    // Build API routes (including auth)
    // Note: llm_models routes must be merged BEFORE llm_providers
    // because /v1/llm-providers/{provider_id}/models is more specific
    // than /v1/llm-providers/{id}
    let api_routes = Router::new()
        .merge(api::agents::routes(agents_state))
        .merge(api::sessions::routes(sessions_state))
        .merge(api::messages::routes(messages_state))
        .merge(api::events::routes(events_state))
        .merge(api::llm_models::routes(llm_models_state))
        .merge(api::llm_providers::routes(llm_providers_state))
        .merge(api::capabilities::routes(capabilities_state))
        .merge(api::session_files::routes(session_files_state))
        .merge(api::users::routes(users_state))
        .merge(auth::routes(auth_state));

    // Build main router with health (not prefixed) and prefixed API routes
    let mut app = Router::new().route("/health", get(health).with_state(health_state));

    // Apply API prefix if configured (affects all API routes including auth)
    app = app.merge(build_router_with_prefix(api_routes, &api_prefix));

    // Add Swagger UI
    let app =
        app.merge(SwaggerUi::new("/swagger-ui").url("/api-doc/openapi.json", ApiDoc::openapi()));

    // Add CORS layer only if origins are configured
    let app = if !cors_origins.is_empty() {
        app.layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(cors_origins))
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

    // Add tracing
    let app = app.layer(TraceLayer::new_for_http());

    // Start gRPC server for worker communication
    let grpc_addr = std::env::var("GRPC_ADDR").unwrap_or_else(|_| "0.0.0.0:9001".to_string());
    let grpc_db = db.clone();
    let grpc_encryption = encryption.clone();
    let grpc_event_service = event_service.clone();
    tokio::spawn(async move {
        // Use the shared EventService with listeners (OTel, etc.)
        let grpc_service = grpc_service::WorkerServiceImpl::new(
            (*grpc_event_service).clone(),
            grpc_db,
            grpc_encryption,
        );
        let addr = grpc_addr.parse().expect("Invalid GRPC_ADDR");
        tracing::info!("gRPC server listening on {}", addr);
        if let Err(e) = tonic::transport::Server::builder()
            .add_service(grpc_service.into_server())
            .serve(addr)
            .await
        {
            tracing::error!("gRPC server error: {}", e);
        }
    });

    // Start background task for stale task reclamation (durable mode only)
    if matches!(runner_config.mode, everruns_worker::RunnerMode::Durable) {
        use everruns_durable::{PostgresWorkflowEventStore, WorkflowEventStore};
        use std::time::Duration;

        let reclaim_db = db.clone();
        let stale_threshold = Duration::from_secs(30); // Tasks without heartbeat for 30s are stale
        let reclaim_interval = Duration::from_secs(10); // Check every 10 seconds

        tokio::spawn(async move {
            let store = PostgresWorkflowEventStore::new(reclaim_db.pool().clone());
            let mut interval = tokio::time::interval(reclaim_interval);

            tracing::info!(
                stale_threshold_secs = stale_threshold.as_secs(),
                reclaim_interval_secs = reclaim_interval.as_secs(),
                "Started stale task reclamation background task"
            );

            loop {
                interval.tick().await;
                match store.reclaim_stale_tasks(stale_threshold).await {
                    Ok(reclaimed) => {
                        if !reclaimed.is_empty() {
                            tracing::info!(
                                count = reclaimed.len(),
                                task_ids = ?reclaimed,
                                "Reclaimed stale tasks"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to reclaim stale tasks: {}", e);
                    }
                }
            }
        });
    }

    // Start HTTP server
    let addr = "0.0.0.0:9000";
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("Failed to bind to address")?;
    tracing::info!("HTTP server listening on {}", addr);

    axum::serve(listener, app).await.context("Server error")?;

    Ok(())
}

/// Build router with optional API prefix (extracted for testing)
fn build_router_with_prefix<S: Clone + Send + Sync + 'static>(
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
    use axum::{body::Body, http::Request};
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

        // Route should work with prefix
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

        // Route should NOT work without prefix
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

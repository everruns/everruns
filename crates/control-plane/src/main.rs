// Everruns API server
// Decision: Flexible auth with support for no-auth, admin-only, and full auth modes
// Decision: DEV_MODE=true uses in-memory storage, no PostgreSQL required
// Decision: Always seed default data on startup (agents, etc.)
// M2: Agent/Session/Messages model with Events as SSE notifications

mod grpc_service;
mod seed;

// Use modules from library
use everruns_control_plane::DirectWorkerAdapters;
use everruns_control_plane::api;
use everruns_control_plane::auth;
use everruns_control_plane::openapi::ApiDoc;
use everruns_control_plane::services;
use everruns_control_plane::storage::{EncryptionService, StorageBackend};
use everruns_control_plane::task_notifications::TaskNotificationBroadcaster;
use everruns_worker::{TaskWorker, TaskWorkerConfig};

use anyhow::{Context, Result};
use axum::http::{HeaderValue, Method, header};
use axum::{Json, Router, extract::State, routing::get};
use everruns_core::CapabilityRegistry;
use everruns_core::telemetry::{TelemetryConfig, init_telemetry};
use everruns_core::{BraintrustListener, EventListener, OtelEventListener};
use everruns_durable::{
    InMemoryWorkflowEventStore, PostgresWorkflowEventStore, WorkflowEventStore,
};
use everruns_worker::{RunnerBackend, create_driver_registry, create_runner_with_backend};
use serde::Serialize;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

/// App state shared across routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
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

    // Check for dev mode (no PostgreSQL required)
    let dev_mode = std::env::var("DEV_MODE")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    // Initialize storage backend and runner based on mode
    // In DEV_MODE, we create a shared InMemoryWorkflowEventStore for both runner and in-process worker
    let (db, runner, shared_durable_store) = if dev_mode {
        tracing::info!("Starting in DEV MODE (in-memory storage, no PostgreSQL required)");

        // Create in-memory storage
        let db = Arc::new(StorageBackend::in_memory());

        // Create shared in-memory durable store
        let shared_store = Arc::new(InMemoryWorkflowEventStore::new());

        // Create in-memory runner with shared store
        let runner =
            create_runner_with_backend(RunnerBackend::SharedInMemory(shared_store.clone()))
                .await
                .context("Failed to create in-memory agent runner")?;

        tracing::info!(
            "Using in-memory storage and durable execution engine with in-process worker"
        );
        (db, runner, Some(shared_store))
    } else {
        // Production mode: require PostgreSQL
        let database_url =
            std::env::var("DATABASE_URL").context("DATABASE_URL environment variable required")?;
        let backend = StorageBackend::postgres(&database_url)
            .await
            .context("Failed to connect to database")?;
        tracing::info!("Connected to PostgreSQL database");

        // Create PostgreSQL-backed runner
        let pool = backend
            .pool()
            .expect("PostgreSQL backend should have pool")
            .clone();
        let runner = create_runner_with_backend(RunnerBackend::Postgres(pool))
            .await
            .context("Failed to create agent runner")?;

        tracing::info!("Using Durable execution engine runner (PostgreSQL-backed)");
        (Arc::new(backend), runner, None)
    };

    // Spawn seeding in background (non-blocking, non-fatal)
    // This creates default agents if they don't exist
    seed::spawn_seed_task(db.clone());

    // Initialize encryption service for API keys (optional - gracefully degrade if not configured)
    let encryption = match EncryptionService::from_env() {
        Ok(svc) => {
            tracing::info!("Encryption service initialized for API key storage");
            Some(Arc::new(svc))
        }
        Err(e) => {
            tracing::warn!(
                "Encryption service not configured (SECRETS_ENCRYPTION_KEY not set): {}. API key storage disabled.",
                e
            );
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
    let sessions_state =
        api::sessions::AppState::new(db.clone(), runner.clone(), auth_state.clone());
    let messages_state =
        api::messages::AppState::new(db.clone(), runner.clone(), auth_state.clone());

    // Create event listeners for observability and tracking
    // OtelEventListener generates gen-ai semantic convention spans from events
    let otel_listener: Arc<dyn EventListener> = Arc::new(OtelEventListener::new());
    // UsageTrackingListener tracks LLM token usage in llm_generations table
    let usage_listener: Arc<dyn EventListener> =
        Arc::new(services::UsageTrackingListener::new(db.clone()));

    // Build list of event listeners
    let mut event_listeners: Vec<Arc<dyn EventListener>> = vec![otel_listener, usage_listener];

    // BraintrustListener sends LLM generation events to Braintrust for observability
    // Enabled when BRAINTRUST_API_KEY and BRAINTRUST_PROJECT_ID are set
    if let Some(braintrust_listener) = BraintrustListener::from_env() {
        tracing::info!("Braintrust integration enabled");
        event_listeners.push(Arc::new(braintrust_listener));
    }

    // Create EventService with listeners - shared between HTTP API and gRPC service
    let event_service = Arc::new(services::EventService::with_listeners(
        db.clone(),
        event_listeners,
    ));

    let events_state = api::events::AppState {
        session_service: Arc::new(services::SessionService::new(db.clone())),
        event_service: event_service.clone(),
        auth: auth_state.clone(),
    };
    // AG-UI protocol state (secondary SSE API)
    let ag_ui_state = api::ag_ui::AppState {
        session_service: Arc::new(services::SessionService::new(db.clone())),
        event_service: event_service.clone(),
        auth: auth_state.clone(),
    };
    // Create driver registry for model sync operations
    let driver_registry = Arc::new(create_driver_registry());
    let llm_providers_state = api::llm_providers::AppState::new(
        db.clone(),
        encryption.clone(),
        driver_registry.clone(),
        auth_state.clone(),
    );
    let llm_models_state = api::llm_models::AppState::new(db.clone(), auth_state.clone());
    let mcp_servers_state =
        api::mcp_servers::AppState::new(db.clone(), encryption.clone(), auth_state.clone());
    let capability_service = Arc::new(services::CapabilityService::new(
        db.clone(),
        encryption.clone(),
    ));
    let capabilities_state =
        api::capabilities::AppState::new(capability_service.clone(), auth_state.clone());
    let agents_state =
        api::agents::AppState::new(db.clone(), capability_service, auth_state.clone());
    let session_files_state = api::session_files::AppState::new(db.clone());
    let session_storage_state = api::session_storage::AppState::new(db.clone(), auth_state.clone());
    let users_state = api::users::UsersState {
        db: db.clone(),
        auth: auth_state.clone(),
    };
    // Create durable execution store based on mode
    // In DEV_MODE, use the shared store; in production, use PostgreSQL
    let durable_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>> =
        if let Some(ref shared_store) = shared_durable_store {
            tracing::info!("Using shared in-memory workflow event store for DEV MODE");
            Some(shared_store.clone() as Arc<dyn WorkflowEventStore + Send + Sync>)
        } else {
            db.pool().cloned().map(|p| {
                Arc::new(PostgresWorkflowEventStore::new(p))
                    as Arc<dyn WorkflowEventStore + Send + Sync>
            })
        };
    let durable_state = api::durable::AppState::new(durable_store);
    let images_state = api::images::AppState::new(db.clone(), auth_state.clone());
    let organizations_state = api::organizations::AppState::new(db.clone(), auth_state.clone());
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
        .merge(api::ag_ui::routes(ag_ui_state))
        .merge(api::llm_models::routes(llm_models_state))
        .merge(api::llm_providers::routes(llm_providers_state))
        .merge(api::mcp_servers::routes(mcp_servers_state))
        .merge(api::capabilities::routes(capabilities_state))
        .merge(api::session_files::routes(session_files_state))
        .merge(api::session_storage::routes(session_storage_state))
        .merge(api::users::routes(users_state))
        .merge(api::durable::routes(durable_state))
        .merge(api::images::routes(images_state))
        .merge(api::organizations::routes(organizations_state))
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

    // Start gRPC server for worker communication (only in production mode)
    // In dev mode, workers are not supported - everything runs in-process
    if !dev_mode {
        let grpc_addr = std::env::var("GRPC_ADDR").unwrap_or_else(|_| "0.0.0.0:9001".to_string());
        let grpc_db = db.clone();
        let grpc_encryption = encryption.clone();
        let grpc_event_service = event_service.clone();

        // Create task notification broadcaster for push-based notifications
        // This uses PostgreSQL NOTIFY/LISTEN for low-latency task delivery
        let task_broadcaster = if let Some(pool) = db.pool() {
            let broadcaster = TaskNotificationBroadcaster::new(pool.clone()).await;
            tracing::info!(
                "Task notification broadcaster initialized for push-based notifications"
            );
            Some(Arc::new(broadcaster))
        } else {
            tracing::warn!("Task notification broadcaster not available (no PostgreSQL pool)");
            None
        };

        tokio::spawn(async move {
            // Use the shared EventService with listeners (OTel, etc.)
            let mut grpc_service = grpc_service::WorkerServiceImpl::new(
                (*grpc_event_service).clone(),
                grpc_db,
                grpc_encryption,
            );

            // Set task broadcaster for push-based notifications
            if let Some(broadcaster) = task_broadcaster {
                grpc_service.set_task_broadcaster(broadcaster);
            }

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

        // Start background task for stale task reclamation (only with PostgreSQL)
        {
            use std::time::Duration;

            if let Some(pool) = db.pool() {
                let pool = pool.clone();
                let stale_threshold = Duration::from_secs(30); // Tasks without heartbeat for 30s are stale
                let reclaim_interval = Duration::from_secs(10); // Check every 10 seconds

                tokio::spawn(async move {
                    let store = PostgresWorkflowEventStore::new(pool);
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
        }

        // Start background task for model discovery sync (only with PostgreSQL)
        {
            use std::time::Duration;

            // Sync interval defaults to 24 hours, configurable via MODEL_SYNC_INTERVAL_HOURS
            let sync_interval_hours: u64 = std::env::var("MODEL_SYNC_INTERVAL_HOURS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(24);

            // Skip sync if interval is 0 (disabled)
            if sync_interval_hours > 0 {
                let sync_service = Arc::new(services::ModelSyncService::new(
                    db.clone(),
                    driver_registry.clone(),
                ));
                let sync_interval = Duration::from_secs(sync_interval_hours * 3600);

                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(sync_interval);
                    // Skip the first tick (don't sync immediately on startup)
                    interval.tick().await;

                    tracing::info!(
                        interval_hours = sync_interval_hours,
                        "Started model discovery sync background task"
                    );

                    loop {
                        interval.tick().await;
                        tracing::info!("Starting scheduled model sync for all providers");

                        match sync_service.sync_all().await {
                            Ok(results) => {
                                for (provider_id, result) in results {
                                    match result {
                                        services::SyncResult::Success {
                                            created,
                                            updated,
                                            stale,
                                        } => {
                                            tracing::info!(
                                                %provider_id,
                                                created,
                                                updated,
                                                stale,
                                                "Model sync completed for provider"
                                            );
                                        }
                                        services::SyncResult::NotSupported => {
                                            tracing::debug!(
                                                %provider_id,
                                                "Model sync not supported for provider"
                                            );
                                        }
                                        services::SyncResult::Failed { error } => {
                                            tracing::warn!(
                                                %provider_id,
                                                %error,
                                                "Model sync failed for provider"
                                            );
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("Failed to run model sync: {}", e);
                            }
                        }
                    }
                });
            } else {
                tracing::info!("Model sync background task disabled (MODEL_SYNC_INTERVAL_HOURS=0)");
            }
        }
    } else {
        // DEV MODE: Start in-process task worker instead of gRPC server
        if let Some(shared_store) = shared_durable_store {
            tracing::info!("DEV MODE: Starting task worker for in-process execution");

            // Create LLM resolver service for the task worker
            let llm_resolver = Arc::new(services::LlmResolverService::new(
                db.clone(),
                encryption.clone(),
            ));

            // Create MCP server service for the task worker
            let mcp_server_service = Arc::new(services::McpServerService::new(
                db.clone(),
                encryption.clone(),
            ));

            // Create capability registry for tool execution
            let capability_registry = CapabilityRegistry::with_builtins();

            // Create direct adapters for in-process worker (no gRPC overhead)
            let adapters = DirectWorkerAdapters::new(
                db.clone(),
                event_service.clone(),
                llm_resolver,
                mcp_server_service,
                capability_registry,
            );

            // Create and spawn the task worker with in-memory store
            let worker_config = TaskWorkerConfig::dev_mode();
            tokio::spawn(async move {
                let mut worker = TaskWorker::new(worker_config, shared_store, adapters);

                if let Err(e) = worker.run().await {
                    tracing::error!("Task worker error: {}", e);
                }
            });

            tracing::info!("DEV MODE: Task worker started - control-plane is fully functional");
        } else {
            tracing::info!("DEV MODE: gRPC server disabled, no task worker available");
        }
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

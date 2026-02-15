// Server entrypoint extracted for reuse by SaaS binary
// Decision: Factory pattern for auth backend — StorageBackend created inside run(),
//   auth_factory receives Arc<StorageBackend> to avoid duplicating migration logic.
// Decision: Optional extra_routes for SaaS-only endpoints (billing, etc.)

use crate::auth::{self, AuthBackend};
use crate::direct_worker_adapters::DirectWorkerAdapters;
use crate::grpc_service;
use crate::openapi::ApiDoc;
use crate::storage::{EncryptionService, StorageBackend};
use crate::task_notifications::TaskNotificationBroadcaster;
use crate::{api, seed, services};

use anyhow::{Context, Result};
use axum::http::{HeaderValue, Method, header};
use axum::{Json, Router, extract::State, routing::get};
use everruns_core::CapabilityRegistry;
use everruns_core::{BraintrustListener, EventListener, OtelEventListener};
use everruns_durable::{
    InMemoryWorkflowEventStore, PostgresWorkflowEventStore, WorkflowEventStore,
};
use everruns_worker::{
    RunnerBackend, TaskWorker, TaskWorkerConfig, create_driver_registry, create_runner_with_backend,
};
use serde::Serialize;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

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

#[derive(Clone)]
struct HealthState {
    auth_mode: String,
}

/// Start the Everruns server.
///
/// `config` — server configuration (dev mode, addresses, CORS, etc.)
/// `auth_factory` — creates an auth backend given the storage backend.
///   Use `|db| Arc::new(BuiltinAuthBackend::new(auth_config, db))` for OSS default.
/// `extra_routes` — additional routes merged into the API router (billing, etc.)
pub async fn run(
    config: ServerConfig,
    auth_factory: impl FnOnce(Arc<StorageBackend>) -> Arc<dyn AuthBackend>,
    extra_routes: Option<Router>,
) -> Result<()> {
    tracing::info!("everrun-api starting...");

    // Initialize storage backend and runner based on mode
    let (db, runner, shared_durable_store) = if config.dev_mode {
        tracing::info!("Starting in DEV MODE (in-memory storage, no PostgreSQL required)");

        let db = Arc::new(StorageBackend::in_memory());
        let shared_store = Arc::new(InMemoryWorkflowEventStore::new());
        let runner =
            create_runner_with_backend(RunnerBackend::SharedInMemory(shared_store.clone()))
                .await
                .context("Failed to create in-memory agent runner")?;

        tracing::info!(
            "Using in-memory storage and durable execution engine with in-process worker"
        );
        (db, runner, Some(shared_store))
    } else {
        let database_url =
            std::env::var("DATABASE_URL").context("DATABASE_URL environment variable required")?;
        let backend = StorageBackend::postgres(&database_url)
            .await
            .context("Failed to connect to database")?;
        tracing::info!("Connected to PostgreSQL database");

        if !config.no_migrations {
            tracing::info!("Running database migrations...");
            let pool = backend.pool().expect("PostgreSQL backend should have pool");
            sqlx::migrate!("./migrations")
                .run(pool)
                .await
                .context("Database migration failed - check migration files and database state")?;
            tracing::info!("Database migrations complete");
        } else {
            tracing::info!("Skipping database migrations (--no-migrations)");
        }

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

    // Spawn seeding in background
    seed::spawn_seed_task(db.clone());

    // Create session SQL database store
    let sqldb_backend = Arc::new(everruns_session_sqldb::InMemorySqlDbBackend::new());
    let sqldb_store: Arc<dyn everruns_core::session_sqldb::SessionSqlDbStore> = Arc::new(
        everruns_session_sqldb::InMemorySqlDbStore::new(sqldb_backend),
    );
    tracing::info!("Session SQL database store initialized (in-memory)");

    // Initialize encryption service
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

    // Load authentication configuration and create backend via factory
    let auth_config = auth::AuthConfig::from_env();
    tracing::info!(
        mode = ?auth_config.mode,
        password_auth = auth_config.password_auth_enabled(),
        oauth = auth_config.oauth_enabled(),
        "Authentication configured"
    );

    let auth_backend = auth_factory(db.clone());
    let auth_state = auth::AuthState::new(auth_config.clone(), auth_backend.clone());

    // Create module-specific states
    let sessions_state =
        api::sessions::AppState::new(db.clone(), runner.clone(), auth_state.clone());
    let messages_state =
        api::messages::AppState::new(db.clone(), runner.clone(), auth_state.clone());
    let tool_results_state =
        api::tool_results::AppState::new(db.clone(), runner.clone(), auth_state.clone());

    // Create event listeners
    let otel_listener: Arc<dyn EventListener> = Arc::new(OtelEventListener::new());
    let usage_listener: Arc<dyn EventListener> =
        Arc::new(services::UsageTrackingListener::new(db.clone()));
    let mut event_listeners: Vec<Arc<dyn EventListener>> = vec![otel_listener, usage_listener];

    if let Some(braintrust_listener) = BraintrustListener::from_env() {
        tracing::info!("Braintrust integration enabled");
        event_listeners.push(Arc::new(braintrust_listener));
    }

    let event_service = Arc::new(services::EventService::with_listeners(
        db.clone(),
        event_listeners,
    ));

    let sse_tracker = Arc::new(crate::api::sse::SseConnectionTracker::new(
        crate::api::sse::SseConnectionLimits::from_env(),
    ));
    // Initialize event notification broadcaster for push-based SSE (requires PostgreSQL)
    let event_broadcaster = if let Some(pool) = db.pool() {
        let broadcaster =
            crate::event_notifications::EventNotificationBroadcaster::new(pool.clone()).await;
        tracing::info!("Event notification broadcaster initialized for push-based SSE");
        Some(Arc::new(broadcaster))
    } else {
        tracing::info!(
            "Event notification broadcaster not available (DEV_MODE), SSE will use polling"
        );
        None
    };

    let events_state = api::events::AppState {
        session_service: Arc::new(services::SessionService::new(db.clone())),
        event_service: event_service.clone(),
        sse_tracker,
        event_broadcaster,
        auth: auth_state.clone(),
    };
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
    let harnesses_state =
        api::harnesses::AppState::new(db.clone(), capability_service.clone(), auth_state.clone());
    let agents_state =
        api::agents::AppState::new(db.clone(), capability_service, auth_state.clone());
    let session_files_state = api::session_files::AppState::new(db.clone(), auth_state.clone());
    let session_storage_state = api::session_storage::AppState::new(db.clone(), auth_state.clone());
    let session_databases_state =
        api::session_databases::AppState::new(sqldb_store.clone(), auth_state.clone());
    let users_state = api::users::UsersState {
        db: db.clone(),
        auth: auth_state.clone(),
    };
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
    let durable_state = api::durable::AppState::new(durable_store.clone());
    let schedules_state = api::schedules::ScheduleAppState::new(durable_store);
    let skills_state = api::skills::AppState::new(db.clone(), auth_state.clone());
    let images_state = api::images::AppState::new(db.clone(), auth_state.clone());
    let organizations_state = api::organizations::AppState::new(db.clone(), auth_state.clone());
    let health_state = HealthState {
        auth_mode: format!("{:?}", auth_config.mode),
    };

    if !config.api_prefix.is_empty() {
        tracing::info!(prefix = %config.api_prefix, "API prefix configured");
    }

    if config.cors_origins.is_empty() {
        tracing::info!("CORS not configured (same-origin requests only)");
    } else {
        tracing::info!(origins = ?config.cors_origins, "CORS origins configured");
    }

    // Build API routes
    let mut api_routes = Router::new()
        .merge(api::agents::routes(agents_state))
        .merge(api::harnesses::routes(harnesses_state))
        .merge(api::sessions::routes(sessions_state))
        .merge(api::messages::routes(messages_state))
        .merge(api::tool_results::routes(tool_results_state))
        .merge(api::events::routes(events_state))
        .merge(api::llm_models::routes(llm_models_state))
        .merge(api::llm_providers::routes(llm_providers_state))
        .merge(api::mcp_servers::routes(mcp_servers_state))
        .merge(api::capabilities::routes(capabilities_state))
        .merge(api::session_files::routes(session_files_state))
        .merge(api::session_storage::routes(session_storage_state))
        .merge(api::session_databases::routes(session_databases_state))
        .merge(api::users::routes(users_state))
        .merge(api::durable::routes(durable_state))
        .merge(api::schedules::routes(schedules_state))
        .merge(api::images::routes(images_state))
        .merge(api::skills::routes(skills_state))
        .merge(api::organizations::routes(organizations_state));

    // Add auth-specific routes if the backend provides them
    if let Some(auth_routes) = auth_backend.auth_routes() {
        api_routes = api_routes.merge(auth_routes);
    }

    // Merge extra routes (SaaS billing, etc.)
    if let Some(extra) = extra_routes {
        api_routes = api_routes.merge(extra);
    }

    // Build main router
    let mut app = Router::new().route("/health", get(health).with_state(health_state));
    app = app.merge(build_router_with_prefix(api_routes, &config.api_prefix));
    let app =
        app.merge(SwaggerUi::new("/swagger-ui").url("/api-doc/openapi.json", ApiDoc::openapi()));

    // CORS
    let app = if !config.cors_origins.is_empty() {
        app.layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(config.cors_origins))
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

    let app = app.layer(TraceLayer::new_for_http());

    // Start gRPC server and background tasks (production) or in-process worker (dev)
    if !config.dev_mode {
        let grpc_db = db.clone();
        let grpc_encryption = encryption.clone();
        let grpc_event_service = event_service.clone();
        let grpc_addr = config.grpc_addr.clone();

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
            let mut grpc_svc = grpc_service::WorkerServiceImpl::new(
                (*grpc_event_service).clone(),
                grpc_db,
                grpc_encryption,
            );

            if let Some(broadcaster) = task_broadcaster {
                grpc_svc.set_task_broadcaster(broadcaster);
            }

            // THREAT[TM-DURABLE-002]: gRPC unauthenticated access
            // Mitigation: Bearer token auth via GRPC_AUTH_TOKEN env var
            let auth_interceptor =
                grpc_service::GrpcAuthInterceptor::new(grpc_service::grpc_auth_token_from_env());

            let addr = grpc_addr.parse().expect("Invalid GRPC_ADDR");
            tracing::info!("gRPC server listening on {}", addr);
            if let Err(e) = tonic::transport::Server::builder()
                .layer(tonic::service::interceptor::InterceptorLayer::new(
                    auth_interceptor,
                ))
                .add_service(grpc_svc.into_server())
                .serve(addr)
                .await
            {
                tracing::error!("gRPC server error: {}", e);
            }
        });

        // Stale task reclamation
        {
            use std::time::Duration;

            if let Some(pool) = db.pool() {
                let pool = pool.clone();
                let stale_threshold = Duration::from_secs(30);
                let reclaim_interval = Duration::from_secs(10);

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

        // Model sync
        {
            use std::time::Duration;

            let sync_interval_hours: u64 = std::env::var("MODEL_SYNC_INTERVAL_HOURS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(24);

            if sync_interval_hours > 0 {
                let sync_service = Arc::new(services::ModelSyncService::new(
                    db.clone(),
                    driver_registry.clone(),
                ));
                let sync_interval = Duration::from_secs(sync_interval_hours * 3600);

                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(sync_interval);
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

        // Event retention (archive old events)
        if let Some(pool) = db.pool() {
            let retention_days = crate::event_retention::retention_days_from_env();
            crate::event_retention::spawn_retention_task(pool.clone(), retention_days);
        }
    } else {
        // DEV MODE: Start in-process task worker
        if let Some(shared_store) = shared_durable_store {
            tracing::info!("DEV MODE: Starting task worker for in-process execution");

            let llm_resolver = Arc::new(services::LlmResolverService::new(
                db.clone(),
                encryption.clone(),
            ));
            let mcp_server_service = Arc::new(services::McpServerService::new(
                db.clone(),
                encryption.clone(),
            ));
            let capability_registry = CapabilityRegistry::with_builtins();

            let session_storage_store: Arc<dyn everruns_core::traits::SessionStorageStore> =
                match db.as_ref() {
                    crate::storage::StorageBackend::Postgres(database) => {
                        if let Some(enc) = &encryption {
                            Arc::new(crate::storage::create_db_session_storage_store(
                                database.clone(),
                                enc.as_ref().clone(),
                            ))
                        } else {
                            Arc::new(
                                crate::storage::create_db_session_storage_store_without_encryption(
                                    database.clone(),
                                ),
                            )
                        }
                    }
                    crate::storage::StorageBackend::InMemory(mem_db) => mem_db.clone(),
                };

            let adapters = DirectWorkerAdapters::new(
                db.clone(),
                event_service.clone(),
                llm_resolver,
                mcp_server_service,
                capability_registry,
            )
            .with_sqldb_store(sqldb_store.clone())
            .with_storage_store(session_storage_store);

            let worker_config = TaskWorkerConfig::dev_mode();
            tokio::spawn(async move {
                let mut worker = TaskWorker::new(worker_config, shared_store, adapters);

                if let Err(e) = worker.run().await {
                    tracing::error!("Task worker error: {}", e);
                }
            });

            tracing::info!("DEV MODE: Task worker started - server is fully functional");
        } else {
            tracing::info!("DEV MODE: gRPC server disabled, no task worker available");
        }
    }

    // Start HTTP server
    let listener = tokio::net::TcpListener::bind(&config.addr)
        .await
        .context("Failed to bind to address")?;
    tracing::info!("HTTP server listening on {}", config.addr);

    axum::serve(listener, app).await.context("Server error")?;

    Ok(())
}

/// Build router with optional API prefix
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

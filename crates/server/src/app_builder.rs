// Server app builder for composable server configurations
//
// Decision: Builder pattern lets downstream crates compose custom server setups
//   by adding routes, event listeners, migrations, and auth backends.
// Decision: The old `run()` function is kept as a thin wrapper for backward compat.
// Decision: ServerContext exposes shared infrastructure for background tasks.
// Decision: Uses hyper_util auto builder instead of axum::serve to configure
//   HTTP/2 flow control windows. Default 65KB per-stream window exhausts under
//   high SSE concurrency (50+ streams over single HTTP/2 connection). We set
//   2MB stream windows, 16MB connection windows, and enable adaptive flow control.

use crate::auth::{self, AuthBackend};
use crate::direct_worker_adapters::DirectWorkerAdapters;
use crate::grpc_service;
use crate::openapi::ApiDoc;
use crate::server::{ServerConfig, build_router_with_prefix};
use crate::storage::{EncryptionService, StorageBackend};
use crate::task_notifications::TaskNotificationBroadcaster;
use crate::{api, seed, services};

use anyhow::{Context, Result};
use axum::http::{Method, header};
use axum::{Json, Router, extract::State, routing::get};
use everruns_core::CapabilityRegistry;
use everruns_core::{BraintrustListener, EventListener, OtelEventListener};
use everruns_durable::{
    InMemoryWorkflowEventStore, PostgresWorkflowEventStore, WorkflowEventStore,
};
use everruns_worker::{
    AgentRunner, RunnerBackend, TaskWorker, TaskWorkerConfig, create_driver_registry,
    create_runner_with_backend,
};
use serde::Serialize;
use sqlx::PgPool;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;

// =========================================================================
// Types
// =========================================================================

type AuthFactoryFn = Box<dyn FnOnce(Arc<StorageBackend>) -> Arc<dyn AuthBackend> + Send>;

type MigrationFn =
    Box<dyn FnOnce(PgPool) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send>;

type BackgroundTaskFn =
    Box<dyn FnOnce(ServerContext) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

// =========================================================================
// ServerContext
// =========================================================================

/// Shared infrastructure context passed to custom background tasks.
///
/// Provides access to storage, services, and configuration so extension code
/// can schedule work, emit events, or query the database.
#[derive(Clone)]
pub struct ServerContext {
    pub db: Arc<StorageBackend>,
    pub event_service: Arc<services::EventService>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub runner: Arc<dyn AgentRunner>,
    pub driver_registry: Arc<everruns_core::DriverRegistry>,
}

// =========================================================================
// Health endpoint (moved from server.rs)
// =========================================================================

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

// =========================================================================
// ServerAppBuilder
// =========================================================================

/// Builder for composing server applications.
///
/// Allows downstream crates to extend the server with custom auth backends,
/// API routes, event listeners, database migrations, and background tasks.
///
/// # Example
///
/// ```rust,ignore
/// use everruns_server::app_builder::ServerAppBuilder;
/// use everruns_server::server::ServerConfig;
/// use everruns_server::auth::{AuthConfig, BuiltinAuthBackend};
///
/// let auth_config = AuthConfig::from_env();
/// ServerAppBuilder::new(ServerConfig::from_env())
///     .auth(move |db| Arc::new(BuiltinAuthBackend::new(auth_config, db)))
///     .routes(my_billing_routes())
///     .event_listener(Arc::new(MyAnalyticsListener))
///     .migration(|pool| async move {
///         sqlx::migrate!("./my_migrations").run(&pool).await?;
///         Ok(())
///     })
///     .run()
///     .await?;
/// ```
pub struct ServerAppBuilder {
    config: ServerConfig,
    auth_factory: Option<AuthFactoryFn>,
    extra_routes: Vec<Router>,
    event_listeners: Vec<Arc<dyn EventListener>>,
    migrations: Vec<MigrationFn>,
    background_tasks: Vec<BackgroundTaskFn>,
}

impl ServerAppBuilder {
    /// Create a new server app builder with the given configuration.
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            auth_factory: None,
            extra_routes: Vec::new(),
            event_listeners: Vec::new(),
            migrations: Vec::new(),
            background_tasks: Vec::new(),
        }
    }

    /// Set the authentication backend factory.
    ///
    /// The factory receives the storage backend and returns an auth backend.
    /// If not set, the built-in auth backend is used.
    pub fn auth(
        mut self,
        factory: impl FnOnce(Arc<StorageBackend>) -> Arc<dyn AuthBackend> + Send + 'static,
    ) -> Self {
        self.auth_factory = Some(Box::new(factory));
        self
    }

    /// Add extra API routes merged into the main router.
    ///
    /// Call multiple times to add routes from different modules.
    pub fn routes(mut self, routes: Router) -> Self {
        self.extra_routes.push(routes);
        self
    }

    /// Add an event listener that receives all domain events.
    ///
    /// Built-in listeners (OTel, usage tracking) are always included.
    /// Custom listeners are appended after the built-in ones.
    pub fn event_listener(mut self, listener: Arc<dyn EventListener>) -> Self {
        self.event_listeners.push(listener);
        self
    }

    /// Add a database migration callback that runs after built-in migrations.
    ///
    /// Only runs in production mode (PostgreSQL). Skipped in DEV_MODE.
    /// Migrations run before the server starts accepting requests.
    pub fn migration<F, Fut>(mut self, f: F) -> Self
    where
        F: FnOnce(PgPool) -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.migrations
            .push(Box::new(move |pool| Box::pin(f(pool))));
        self
    }

    /// Add a custom background task that runs after server initialization.
    ///
    /// The task receives a `ServerContext` with access to shared infrastructure.
    /// Tasks are spawned as tokio tasks and run concurrently.
    pub fn background_task<F, Fut>(mut self, f: F) -> Self
    where
        F: FnOnce(ServerContext) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.background_tasks
            .push(Box::new(move |ctx| Box::pin(f(ctx))));
        self
    }

    /// Build and run the server. Blocks until shutdown.
    pub async fn run(self) -> Result<()> {
        tracing::info!("everrun-api starting...");

        // =====================================================================
        // Phase 1: Storage backend & runner
        // =====================================================================
        let (db, runner, shared_durable_store) = if self.config.dev_mode {
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
            let database_url = std::env::var("DATABASE_URL")
                .context("DATABASE_URL environment variable required")?;

            // TM-NEW: Warn when DATABASE_URL lacks TLS in production
            if !database_url.contains("sslmode=") {
                tracing::warn!(
                    "DATABASE_URL does not specify sslmode. \
                     For production, use sslmode=require or sslmode=verify-full \
                     to encrypt database connections."
                );
            } else if database_url.contains("sslmode=disable") {
                tracing::warn!(
                    "DATABASE_URL has sslmode=disable — database connections are unencrypted. \
                     For production, use sslmode=require or sslmode=verify-full."
                );
            }

            let backend = StorageBackend::postgres(&database_url)
                .await
                .context("Failed to connect to database")?;
            tracing::info!("Connected to PostgreSQL database");

            if !self.config.no_migrations {
                tracing::info!("Running database migrations...");
                let pool = backend.pool().expect("PostgreSQL backend should have pool");
                sqlx::migrate!("./migrations").run(pool).await.context(
                    "Database migration failed - check migration files and database state",
                )?;
                tracing::info!("Database migrations complete");

                // Run custom migrations
                for migration_fn in self.migrations {
                    migration_fn(pool.clone()).await?;
                }
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

        // =====================================================================
        // Phase 2: Seed & infrastructure services
        // =====================================================================
        seed::spawn_seed_task(db.clone());

        let sqldb_backend = Arc::new(everruns_session_sqldb::InMemorySqlDbBackend::new());
        let sqldb_store: Arc<dyn everruns_core::session_sqldb::SessionSqlDbStore> = Arc::new(
            everruns_session_sqldb::InMemorySqlDbStore::new(sqldb_backend),
        );
        tracing::info!("Session SQL database store initialized (in-memory)");

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

        // =====================================================================
        // Phase 3: Valkey + Authentication
        // =====================================================================
        let valkey_client = crate::valkey::from_env()
            .await
            .context("Failed to initialize Valkey client")?;

        let auth_config = auth::AuthConfig::from_env();
        tracing::info!(
            mode = ?auth_config.mode,
            password_auth = auth_config.password_auth_enabled(),
            oauth = auth_config.oauth_enabled(),
            distributed_rate_limiting = valkey_client.is_some(),
            "Authentication configured"
        );

        let auth_backend = match self.auth_factory {
            Some(factory) => factory(db.clone()),
            None => {
                let cfg = auth_config.clone();
                if let Some(valkey) = valkey_client {
                    Arc::new(auth::BuiltinAuthBackend::with_valkey(
                        cfg,
                        db.clone(),
                        valkey,
                    ))
                } else {
                    Arc::new(auth::BuiltinAuthBackend::new(cfg, db.clone()))
                }
            }
        };
        let auth_state = auth::AuthState::new(auth_config.clone(), auth_backend.clone());

        // =====================================================================
        // Phase 4: Event listeners & services
        // =====================================================================
        let otel_listener: Arc<dyn EventListener> = Arc::new(OtelEventListener::new());
        let usage_listener: Arc<dyn EventListener> =
            Arc::new(services::UsageTrackingListener::new(db.clone()));
        let mut event_listeners: Vec<Arc<dyn EventListener>> = vec![otel_listener, usage_listener];

        if let Some(braintrust_listener) = BraintrustListener::from_env() {
            tracing::info!("Braintrust integration enabled");
            event_listeners.push(Arc::new(braintrust_listener));
        }

        // Append custom event listeners
        event_listeners.extend(self.event_listeners);

        let event_service = Arc::new(services::EventService::with_listeners(
            db.clone(),
            event_listeners,
        ));

        let sse_tracker = Arc::new(crate::api::sse::SseConnectionTracker::new(
            crate::api::sse::SseConnectionLimits::from_env(),
        ));

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

        // =====================================================================
        // Phase 5: API state construction
        // =====================================================================
        let sessions_state =
            api::sessions::AppState::new(db.clone(), runner.clone(), auth_state.clone());
        let messages_state =
            api::messages::AppState::new(db.clone(), runner.clone(), auth_state.clone());
        let tool_results_state =
            api::tool_results::AppState::new(db.clone(), runner.clone(), auth_state.clone());
        // Slack delivery dispatcher: event-driven message posting (replaces 120s polling).
        // Must be created before events_state takes ownership of event_broadcaster.
        let slack_dispatcher = if let Some(ref broadcaster) = event_broadcaster {
            let rx = broadcaster.subscribe();
            let dispatcher = crate::slack_delivery::SlackDeliveryDispatcher::start(db.clone(), rx);
            tracing::info!("Slack delivery dispatcher started (event-driven)");
            Some(dispatcher)
        } else {
            tracing::info!(
                "Slack delivery dispatcher not available (DEV_MODE), using polling fallback"
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
        let llm_resolver = Arc::new(services::LlmResolverService::new(
            db.clone(),
            encryption.clone(),
        ));
        let llm_providers_state = api::llm_providers::AppState::new(
            db.clone(),
            encryption.clone(),
            driver_registry.clone(),
            auth_state.clone(),
            Some(llm_resolver.clone()),
        );
        let llm_models_state = api::llm_models::AppState::new(
            db.clone(),
            auth_state.clone(),
            Some(llm_resolver.clone()),
        );
        let mcp_servers_state =
            api::mcp_servers::AppState::new(db.clone(), encryption.clone(), auth_state.clone());
        let capability_service = Arc::new(services::CapabilityService::new(
            db.clone(),
            encryption.clone(),
        ));
        let capabilities_state =
            api::capabilities::AppState::new(capability_service.clone(), auth_state.clone());
        let harnesses_state = api::harnesses::AppState::new(
            db.clone(),
            capability_service.clone(),
            auth_state.clone(),
        );
        let commands_state =
            api::commands::AppState::new(capability_service.clone(), auth_state.clone());
        let agents_state =
            api::agents::AppState::new(db.clone(), capability_service, auth_state.clone());
        let apps_state = api::apps::AppState::new(db.clone(), auth_state.clone());
        let slack_state = api::slack_events::SlackState::new(
            db.clone(),
            runner.clone(),
            slack_dispatcher.clone(),
        );
        let session_files_state = api::session_files::AppState::new(db.clone(), auth_state.clone());
        let session_storage_state =
            api::session_storage::AppState::new(db.clone(), auth_state.clone());
        let session_databases_state = api::session_databases::AppState::new(
            sqldb_store.clone(),
            db.clone(),
            auth_state.clone(),
        );
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
        // TM-DURABLE-010: All durable endpoints require admin role
        let durable_state = api::durable::AppState::new(durable_store.clone(), auth_state.clone());
        durable_state.spawn_metrics_sampler();
        let scheduler_store = durable_store.clone();
        let schedules_state =
            api::schedules::routes(api::schedules::ScheduleAppState::new(durable_store));
        let skills_state = api::skills::AppState::new(db.clone(), auth_state.clone());
        let images_state = api::images::AppState::new(db.clone(), auth_state.clone());
        let organizations_state = api::organizations::AppState::new(db.clone(), auth_state.clone());
        let audit_logs_state = api::audit_logs::AppState::new(db.clone(), auth_state.clone());
        let user_connections_state = api::user_connections::AppState {
            db: db.clone(),
            encryption: encryption.clone(),
            auth: auth_state.clone(),
            auth_config: auth_config.clone(),
        };
        let session_schedule_service =
            Arc::new(crate::services::SessionScheduleService::new(db.clone()));
        let session_schedules_state = api::session_schedules::AppState::new(
            session_schedule_service.clone(),
            auth_state.clone(),
        );

        let health_state = HealthState {
            auth_mode: format!("{:?}", auth_config.mode),
        };

        // Feature flags (computed from env vars + deployment grade)
        let feature_flags =
            everruns_core::FeatureFlags::from_env(&everruns_core::DeploymentGrade::from_env());
        tracing::info!(?feature_flags, "Feature flags computed");
        let feature_flags_state = api::feature_flags::AppState {
            flags: feature_flags,
        };

        if !self.config.api_prefix.is_empty() {
            tracing::info!(
                prefix = %self.config.api_prefix,
                "API prefix configured"
            );
        }
        if self.config.cors_origins.is_empty() {
            tracing::info!("CORS not configured (same-origin requests only)");
        } else {
            tracing::info!(
                origins = ?self.config.cors_origins,
                "CORS origins configured"
            );
        }

        // =====================================================================
        // Phase 6: Build API router
        // =====================================================================
        let mut api_routes = Router::new()
            .merge(api::agents::routes(agents_state))
            .merge(api::apps::routes(apps_state))
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
            .merge(schedules_state)
            .merge(api::images::routes(images_state))
            .merge(api::skills::routes(skills_state))
            .merge(api::organizations::routes(organizations_state))
            .merge(api::user_connections::routes(user_connections_state))
            .merge(api::session_schedules::routes(session_schedules_state))
            .merge(api::audit_logs::routes(audit_logs_state))
            .merge(api::commands::routes(commands_state))
            .merge(api::slack_events::routes(slack_state))
            .merge(api::feature_flags::routes(feature_flags_state));

        // Auth-specific routes
        if let Some(auth_routes) = auth_backend.auth_routes() {
            api_routes = api_routes.merge(auth_routes);
        }

        // Extra routes (SaaS billing, custom endpoints, etc.)
        for routes in self.extra_routes {
            api_routes = api_routes.merge(routes);
        }

        // Main router
        let app = Router::new()
            .route("/health", get(health).with_state(health_state))
            .route(
                "/api-doc/openapi.json",
                get(|| async { Json(ApiDoc::openapi()) }),
            )
            .merge(build_router_with_prefix(
                api_routes,
                &self.config.api_prefix,
            ));

        // CORS
        let app = if !self.config.cors_origins.is_empty() {
            app.layer(
                CorsLayer::new()
                    .allow_origin(AllowOrigin::list(self.config.cors_origins.clone()))
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
                axum::http::HeaderValue::from_static(
                    "camera=(), microphone=(), geolocation=()",
                ),
            ))
            .layer(SetResponseHeaderLayer::if_not_present(
                axum::http::header::HeaderName::from_static("content-security-policy"),
                axum::http::HeaderValue::from_static(
                    "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
                ),
            ));

        let app = app.layer(TraceLayer::new_for_http());

        // =====================================================================
        // Phase 7: Background tasks
        // =====================================================================
        let server_context = ServerContext {
            db: db.clone(),
            event_service: event_service.clone(),
            encryption: encryption.clone(),
            runner: runner.clone(),
            driver_registry: driver_registry.clone(),
        };

        if !self.config.dev_mode {
            // -- gRPC server --
            let grpc_db = db.clone();
            let grpc_encryption = encryption.clone();
            let grpc_event_service = event_service.clone();
            let grpc_runner = runner.clone();
            let grpc_addr = self.config.grpc_addr.clone();

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
                    Some(grpc_runner),
                );
                if let Some(broadcaster) = task_broadcaster {
                    grpc_svc.set_task_broadcaster(broadcaster);
                }
                // THREAT[TM-DURABLE-002]: gRPC unauthenticated access
                // Mitigation: Bearer token auth + optional mTLS
                let grpc_token = grpc_service::require_grpc_auth_token();
                let auth_interceptor = grpc_service::GrpcAuthInterceptor::new(Some(grpc_token));
                let tls_config = grpc_service::grpc_server_tls_from_env();
                let addr = grpc_addr.parse().expect("Invalid WORKER_GRPC_ADDR");
                tracing::info!("gRPC server listening on {}", addr);

                let mut builder = tonic::transport::Server::builder();
                if let Some(tls) = tls_config {
                    builder = builder
                        .tls_config(tls)
                        .expect("Invalid gRPC TLS configuration");
                }
                if let Err(e) = builder
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

            // -- Stale task reclamation --
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

            // -- Model sync --
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
                        encryption.clone(),
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
                    tracing::info!(
                        "Model sync background task disabled (MODEL_SYNC_INTERVAL_HOURS=0)"
                    );
                }
            }

            // -- Event retention --
            if let Some(pool) = db.pool() {
                let retention_days = crate::event_retention::retention_days_from_env();
                crate::event_retention::spawn_retention_task(pool.clone(), retention_days);
            }
        } else {
            // DEV MODE: Start in-process task worker
            if let Some(shared_store) = shared_durable_store {
                tracing::info!("DEV MODE: Starting task worker for in-process execution");

                // Reuse the shared llm_resolver from Phase 5
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

                let mut adapters = DirectWorkerAdapters::new(
                    db.clone(),
                    event_service.clone(),
                    llm_resolver,
                    mcp_server_service,
                    capability_registry,
                    sqldb_store.clone(),
                )
                .with_storage_store(session_storage_store)
                .with_runner(runner.clone());

                // Wire lazy connection resolver (requires encryption for token decryption)
                if let Some(ref enc) = encryption {
                    let github_app_minter = auth_config.github_connection.as_ref().map(|config| {
                        crate::storage::GitHubAppTokenMinter::new(
                            config.app_id.clone(),
                            config.private_key.clone(),
                        )
                    });
                    let resolver = Arc::new(crate::storage::DbConnectionResolver::new(
                        db.as_ref().clone(),
                        enc.as_ref().clone(),
                        github_app_minter,
                    ));
                    adapters = adapters.with_connection_resolver(resolver);
                }

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

        // -- Slack delivery recovery (re-register active Slack sessions after restart) --
        if let Some(ref dispatcher) = slack_dispatcher {
            let dispatcher = dispatcher.clone();
            let recovery_db = db.clone();
            tokio::spawn(async move {
                let app_service = crate::services::AppService::new(recovery_db);
                dispatcher.recover(&app_service).await;
            });
        }

        // -- Durable task scheduler (both prod and dev) --
        if let Some(store) = scheduler_store {
            let scheduler = everruns_durable::DurableScheduler::with_defaults(
                store,
                format!("scheduler-{}", uuid::Uuid::now_v7()),
            );
            let _scheduler_shutdown = scheduler.spawn();
            tracing::info!("Durable task scheduler started");
        }

        // -- Session schedule poller (both prod and dev) --
        crate::session_scheduler::spawn_session_scheduler(
            db.clone(),
            session_schedule_service,
            event_service,
            runner,
            std::time::Duration::from_secs(15),
        );

        // -- Custom background tasks --
        for task_fn in self.background_tasks {
            let ctx = server_context.clone();
            tokio::spawn(task_fn(ctx));
        }

        // =====================================================================
        // Phase 8: Start HTTP server
        // =====================================================================
        //
        // Uses hyper_util auto builder instead of axum::serve to configure HTTP/2
        // flow control. Default 65KB per-stream window exhausts under high SSE
        // concurrency → streams block → timeout → cascade failure.
        //
        // Tunable via env vars for production sizing:
        //   HTTP2_STREAM_WINDOW_SIZE    per-stream window (default: 2MB)
        //   HTTP2_CONNECTION_WINDOW_SIZE connection window (default: 16MB)
        //   HTTP2_MAX_CONCURRENT_STREAMS max streams per connection (default: 256)
        let listener = tokio::net::TcpListener::bind(&self.config.addr)
            .await
            .context("Failed to bind to address")?;
        tracing::info!("HTTP server listening on {}", self.config.addr);

        let h2_config = Http2FlowConfig::from_env();
        h2_config.log();

        use hyper_util::rt::{TokioExecutor, TokioIo};
        use hyper_util::server::conn::auto::Builder as AutoBuilder;

        let mut builder = AutoBuilder::new(TokioExecutor::new());
        builder
            .http2()
            .initial_stream_window_size(h2_config.stream_window)
            .initial_connection_window_size(h2_config.connection_window)
            .adaptive_window(true)
            .max_concurrent_streams(h2_config.max_concurrent_streams)
            .keep_alive_interval(Some(std::time::Duration::from_secs(20)))
            .keep_alive_timeout(std::time::Duration::from_secs(20));

        loop {
            let (tcp, _addr) = listener
                .accept()
                .await
                .context("Failed to accept connection")?;
            let _ = tcp.set_nodelay(true);

            let app = app.clone();
            let builder = builder.clone();

            tokio::spawn(async move {
                let socket = TokioIo::new(tcp);
                let service = hyper::service::service_fn(
                    move |req: hyper::Request<hyper::body::Incoming>| {
                        let app = app.clone();
                        async move {
                            let req = req.map(axum::body::Body::new);
                            let mut app = app;
                            tower::Service::call(&mut app, req).await
                        }
                    },
                );

                if let Err(err) = builder
                    .serve_connection_with_upgrades(socket, service)
                    .await
                {
                    // Don't log normal connection closures (client disconnect, reset)
                    let msg = err.to_string();
                    if !msg.contains("connection closed")
                        && !msg.contains("reset by peer")
                        && !msg.contains("broken pipe")
                    {
                        tracing::debug!(error = %err, "HTTP connection error");
                    }
                }
            });
        }
    }
}

// =========================================================================
// HTTP/2 Flow Control Configuration
// =========================================================================

/// HTTP/2 flow control settings tuned for high-concurrency SSE streaming.
///
/// Default HTTP/2 per-stream window is 65KB. With 50+ concurrent SSE streams
/// over a single connection, windows exhaust when server produces events faster
/// than clients read. This causes streams to block, timeout, and cascade.
///
/// Defaults: 2MB stream window, 16MB connection window, 256 max streams.
/// Override via environment variables for production sizing.
struct Http2FlowConfig {
    stream_window: u32,
    connection_window: u32,
    max_concurrent_streams: u32,
}

impl Http2FlowConfig {
    const DEFAULT_STREAM_WINDOW: u32 = 2 * 1024 * 1024; // 2 MB
    const DEFAULT_CONNECTION_WINDOW: u32 = 16 * 1024 * 1024; // 16 MB
    const DEFAULT_MAX_CONCURRENT_STREAMS: u32 = 256;

    fn from_env() -> Self {
        Self::from_values(
            std::env::var("HTTP2_STREAM_WINDOW_SIZE").ok().as_deref(),
            std::env::var("HTTP2_CONNECTION_WINDOW_SIZE")
                .ok()
                .as_deref(),
            std::env::var("HTTP2_MAX_CONCURRENT_STREAMS")
                .ok()
                .as_deref(),
        )
    }

    fn from_values(
        stream_window: Option<&str>,
        connection_window: Option<&str>,
        max_concurrent_streams: Option<&str>,
    ) -> Self {
        Self {
            stream_window: stream_window
                .and_then(|v| v.parse().ok())
                .unwrap_or(Self::DEFAULT_STREAM_WINDOW),
            connection_window: connection_window
                .and_then(|v| v.parse().ok())
                .unwrap_or(Self::DEFAULT_CONNECTION_WINDOW),
            max_concurrent_streams: max_concurrent_streams
                .and_then(|v| v.parse().ok())
                .unwrap_or(Self::DEFAULT_MAX_CONCURRENT_STREAMS),
        }
    }

    fn log(&self) {
        tracing::info!(
            h2_stream_window_kb = self.stream_window / 1024,
            h2_conn_window_kb = self.connection_window / 1024,
            h2_max_streams = self.max_concurrent_streams,
            "HTTP/2 flow control configured"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_h2_config_defaults() {
        let config = Http2FlowConfig::from_values(None, None, None);
        assert_eq!(config.stream_window, 2 * 1024 * 1024); // 2 MB
        assert_eq!(config.connection_window, 16 * 1024 * 1024); // 16 MB
        assert_eq!(config.max_concurrent_streams, 256);
    }

    #[test]
    fn test_h2_config_from_env() {
        let config = Http2FlowConfig::from_values(Some("4194304"), Some("33554432"), Some("512"));
        assert_eq!(config.stream_window, 4 * 1024 * 1024);
        assert_eq!(config.connection_window, 32 * 1024 * 1024);
        assert_eq!(config.max_concurrent_streams, 512);
    }

    #[test]
    fn test_h2_config_invalid_env_uses_defaults() {
        let config = Http2FlowConfig::from_values(Some("not_a_number"), Some(""), None);
        assert_eq!(config.stream_window, 2 * 1024 * 1024); // falls back to default
        assert_eq!(config.connection_window, 16 * 1024 * 1024); // falls back to default
        assert_eq!(config.max_concurrent_streams, 256); // not set, default
    }
}

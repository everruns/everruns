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
use crate::event_delivery::EventDelivery;
use crate::grpc_service;
use crate::openapi::ApiDoc;
use crate::pg_listener_config::resolve_pg_listener_database_url;
use crate::server::{ServerConfig, build_router_with_prefix};
use crate::storage::{EncryptionService, StorageBackend};
use crate::{api, seed, services};

use crate::middleware::RequestIdLayer;
use crate::middleware::request_id::RequestId;
use anyhow::{Context, Result};
use async_trait::async_trait;
use axum::http::{Method, header};
use axum::{Json, Router, extract::State, routing::get};
use everruns_core::{
    BraintrustListener, ErrorReport, ErrorReporter, ErrorScope, EventListener, NoopErrorReporter,
    OtelEventListener, PlatformDefinition, SharedErrorReporter,
};
use everruns_durable::{
    InMemoryWorkflowEventStore, PostgresWorkflowEventStore, WorkflowEventStore,
};
use everruns_worker::{
    AgentRunner, DurableTaskNotifier, RunnerBackend, TaskWorker, TaskWorkerConfig,
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

type AuthFactoryFn =
    Box<dyn FnOnce(Arc<StorageBackend>, Arc<PlatformDefinition>) -> Arc<dyn AuthBackend> + Send>;

type MigrationFn =
    Box<dyn FnOnce(PgPool) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send>;

type BackgroundTaskFn =
    Box<dyn FnOnce(ServerContext) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

type PersonalAccessTokenRoutesWrapFn = Box<dyn FnOnce(Router) -> Router + Send>;

struct ServerTaskNotifier {
    broadcaster: Arc<crate::task_notifications::TaskBroadcaster>,
}

#[async_trait]
impl DurableTaskNotifier for ServerTaskNotifier {
    async fn notify_task_available(&self, activity_type: &str) {
        self.broadcaster.notify_task_available(activity_type).await;
    }
}

/// Apply the optional personal-access-token-routes wrap closure to the router.
/// Identity when no wrap is configured.
fn apply_personal_access_token_routes_wrap(
    wrap: Option<PersonalAccessTokenRoutesWrapFn>,
    router: Router,
) -> Router {
    match wrap {
        Some(wrap) => wrap(router),
        None => router,
    }
}

fn permissions_policy_header_value(voice_enabled: bool) -> axum::http::HeaderValue {
    let value = format!(
        "camera=(), {}, geolocation=()",
        api::voice::microphone_permissions_policy_directive(voice_enabled)
    );
    axum::http::HeaderValue::from_str(&value)
        .expect("permissions policy value is assembled from static directives")
}

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
    pub event_delivery: crate::event_delivery::EventDelivery,
    pub encryption: Option<Arc<EncryptionService>>,
    pub runner: Arc<dyn AgentRunner>,
    pub driver_registry: Arc<everruns_core::DriverRegistry>,
    pub platform_definition: Arc<PlatformDefinition>,
    /// System-wide email sender from the platform profile.
    pub email_sender: Arc<dyn everruns_core::EmailSender>,
    /// System-wide outbound egress service from the platform profile.
    pub egress_service: Arc<dyn everruns_core::EgressService>,
    /// System-wide utility LLM service from the platform profile.
    pub utility_llm_service: Arc<dyn everruns_core::UtilityLlmService>,
    /// Vendor-neutral embedder-provided error reporter. Always present;
    /// defaults to a no-op when no embedder has installed one.
    pub error_reporter: SharedErrorReporter,
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
///     .auth(move |db, platform| {
///         Arc::new(BuiltinAuthBackend::new(auth_config, db, platform))
///     })
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
    platform_definition: Option<PlatformDefinition>,
    extra_routes: Vec<Router>,
    event_listeners: Vec<Arc<dyn EventListener>>,
    error_reporter: Option<SharedErrorReporter>,
    migrations: Vec<MigrationFn>,
    background_tasks: Vec<BackgroundTaskFn>,
    personal_access_token_routes_wrap: Option<PersonalAccessTokenRoutesWrapFn>,
}

impl ServerAppBuilder {
    /// Create a new server app builder with the given configuration.
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            auth_factory: None,
            platform_definition: None,
            extra_routes: Vec::new(),
            event_listeners: Vec::new(),
            error_reporter: None,
            migrations: Vec::new(),
            background_tasks: Vec::new(),
            personal_access_token_routes_wrap: None,
        }
    }

    /// Set the authentication backend factory.
    ///
    /// The factory receives the storage backend and the active platform
    /// definition, and returns an auth backend. If not set, the built-in
    /// auth backend is used.
    ///
    /// The platform definition is forwarded so auth handlers can enforce
    /// operator-configured defaults (e.g. the signup harness-seed safety
    /// net in `BuiltinAuthBackend` — see EVE-390).
    pub fn auth(
        mut self,
        factory: impl FnOnce(Arc<StorageBackend>, Arc<PlatformDefinition>) -> Arc<dyn AuthBackend>
        + Send
        + 'static,
    ) -> Self {
        self.auth_factory = Some(Box::new(factory));
        self
    }

    /// Replace the default OSS runtime surface with an explicit platform definition.
    pub fn platform_definition(mut self, platform_definition: PlatformDefinition) -> Self {
        self.platform_definition = Some(platform_definition);
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

    /// Install a vendor-neutral error reporter.
    ///
    /// Wrappers that integrate Sentry, Datadog, Rollbar, etc. implement
    /// `ErrorReporter` and install it here. OSS never imports vendor SDKs;
    /// the reporter is the only contact surface. See `specs/embedding.md`.
    pub fn error_reporter(mut self, reporter: Arc<dyn ErrorReporter>) -> Self {
        self.error_reporter = Some(reporter);
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

    /// Wrap the auto-mounted personal access token CRUD router
    /// (`/v1/auth/personal-access-tokens*`) before it is merged into the main API surface.
    ///
    /// Lets embedders apply route-specific layers — for example, a stricter rate
    /// limiter — without re-mounting the same paths and duplicating routes.
    /// The closure receives the API-key router and must return a router with
    /// the same paths.
    pub fn wrap_personal_access_token_routes<F>(mut self, wrap: F) -> Self
    where
        F: FnOnce(Router) -> Router + Send + 'static,
    {
        self.personal_access_token_routes_wrap = Some(Box::new(wrap));
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
        let platform_definition = Arc::new(
            self.platform_definition
                .clone()
                .unwrap_or_else(crate::platform::oss_platform_definition),
        );

        // =====================================================================
        // Phase 1: Storage backend & runner
        // =====================================================================
        let database_unpooled_url = std::env::var("DATABASE_UNPOOLED_URL").ok();
        let task_broadcaster: Option<Arc<crate::task_notifications::TaskBroadcaster>>;
        let (db, runner, shared_durable_store, database_url) = if self.config.dev_mode {
            tracing::info!("Starting in DEV MODE (in-memory storage, no PostgreSQL required)");

            let db = Arc::new(StorageBackend::in_memory());
            let shared_store = Arc::new(InMemoryWorkflowEventStore::new());
            let runner =
                create_runner_with_backend(RunnerBackend::SharedInMemory(shared_store.clone()))
                    .await
                    .context("Failed to create in-memory agent runner")?;
            task_broadcaster = None;

            tracing::info!(
                "Using in-memory storage and durable execution engine with in-process worker"
            );
            (db, runner, Some(shared_store), None)
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
                if let Err(e) = sqlx::migrate!("./migrations").run(pool).await {
                    tracing::error!(
                        error = %e,
                        "Database migration failed - check migration files and database state"
                    );
                    return Err(e).context(
                        "Database migration failed - check migration files and database state",
                    );
                }
                tracing::info!("Database migrations complete");

                // Run custom migrations
                for migration_fn in self.migrations {
                    if let Err(e) = migration_fn(pool.clone()).await {
                        tracing::error!(error = %e, "Custom database migration failed");
                        return Err(e);
                    }
                }
            } else {
                tracing::info!("Skipping database migrations (--no-migrations)");
            }

            let pool = backend
                .pool()
                .expect("PostgreSQL backend should have pool")
                .clone();
            task_broadcaster = crate::task_notifications::TaskBroadcaster::from_env(
                Some(database_url.as_str()),
                database_unpooled_url.as_deref(),
            )
            .await
            .map(Arc::new);
            let runner_backend = if let Some(broadcaster) = task_broadcaster.clone() {
                RunnerBackend::PostgresWithNotifier {
                    pool,
                    task_notifier: Arc::new(ServerTaskNotifier { broadcaster }),
                }
            } else {
                RunnerBackend::Postgres(pool)
            };
            let runner = create_runner_with_backend(runner_backend)
                .await
                .context("Failed to create agent runner")?;

            tracing::info!("Using Durable execution engine runner (PostgreSQL-backed)");
            (Arc::new(backend), runner, None, Some(database_url))
        };

        // =====================================================================
        // Phase 2: Seed & infrastructure services
        // =====================================================================
        let auth_config = auth::AuthConfig::from_env();
        seed::spawn_seed_task_with_platform_definition(
            db.clone(),
            seed::SeedAuthContext {
                mode: auth_config.mode.clone(),
                admin: auth_config.admin.clone(),
            },
            platform_definition.as_ref().clone(),
        );

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

        tracing::info!(
            mode = ?auth_config.mode,
            password_auth = auth_config.password_auth_enabled(),
            oauth = auth_config.oauth_enabled(),
            distributed_rate_limiting = valkey_client.is_some(),
            "Authentication configured"
        );

        // Reused later by the per-channel public-ingress rate limiters
        // (AG-UI, A2A) so distributed deployments share counters across
        // instances. Each kind gets its own `ChannelRateLimiter` keyed on a
        // distinct namespace.
        let valkey_for_channel_rate_limits = valkey_client.clone();
        // Cloned separately so API and org rate limiters can be initialized
        // before valkey_client is moved into the auth backend below.
        let valkey_for_api_rate_limits = valkey_client.clone();
        let org_rate_limiter =
            crate::auth::rate_limit::OrgRateLimiter::from_env_with_valkey(valkey_client.clone());

        let auth_backend = match self.auth_factory {
            Some(factory) => factory(db.clone(), platform_definition.clone()),
            None => {
                let cfg = auth_config.clone();
                if let Some(valkey) = valkey_client {
                    Arc::new(auth::BuiltinAuthBackend::with_valkey(
                        cfg,
                        db.clone(),
                        platform_definition.clone(),
                        valkey,
                    ))
                } else {
                    Arc::new(auth::BuiltinAuthBackend::new(
                        cfg,
                        db.clone(),
                        platform_definition.clone(),
                    ))
                }
            }
        };
        let deployment_grade = everruns_core::DeploymentGrade::from_env();
        let feature_flags = everruns_core::FeatureFlags::from_env(&deployment_grade);
        let auth_state = auth::AuthState::new(auth_config.clone(), auth_backend.clone())
            .with_db(db.clone())
            .with_system_feature_flags(feature_flags.clone());
        let internal_feature_flags = everruns_core::InternalFeatureFlags::from_env();
        let notifications_enabled = feature_flags.notifications;
        tracing::info!(?feature_flags, "Feature flags computed");

        // Prometheus metrics
        let prometheus_config = api::prometheus::PrometheusConfig::from_env();
        let prometheus_handle = if prometheus_config.enabled {
            match api::prometheus::install_prometheus_recorder() {
                Some(handle) => {
                    if let Some(ref addr) = prometheus_config.metrics_addr {
                        tracing::info!(addr = %addr, "Prometheus metrics on dedicated internal server");
                    } else {
                        tracing::info!("Prometheus /metrics endpoint on main server");
                    }
                    Some(handle)
                }
                None => None,
            }
        } else {
            tracing::info!("Prometheus metrics disabled");
            None
        };

        // =====================================================================
        // Phase 4: Event listeners & domain/infra helpers
        // =====================================================================
        let otel_listener: Arc<dyn EventListener> = Arc::new(OtelEventListener::new());
        let usage_listener: Arc<dyn EventListener> =
            Arc::new(services::UsageTrackingListener::new(db.clone()));
        let budget_service = Arc::new(crate::domains::budgets::BudgetService::new(db.clone()));
        let budget_listener: Arc<dyn EventListener> = budget_service.clone();
        let mut event_listeners: Vec<Arc<dyn EventListener>> =
            vec![otel_listener, usage_listener, budget_listener];
        let session_sandbox_service: Option<
            Arc<crate::domains::session_sandbox::SessionSandboxService>,
        > = if internal_feature_flags.session_sandbox {
            match db.as_ref() {
                crate::storage::StorageBackend::Postgres(database) => match &encryption {
                    Some(enc) => {
                        let storage_store: Arc<dyn everruns_core::traits::SessionStorageStore> =
                            Arc::new(crate::storage::create_db_session_storage_store(
                                database.clone(),
                                enc.as_ref().clone(),
                            ));
                        let github_app_minter =
                            auth_config.github_connection.as_ref().map(|config| {
                                crate::storage::GitHubAppTokenMinter::new(
                                    config.app_id.clone(),
                                    config.private_key.clone(),
                                )
                            });
                        let connection_resolver =
                            Some(Arc::new(crate::storage::DbConnectionResolver::new(
                                db.as_ref().clone(),
                                enc.as_ref().clone(),
                                github_app_minter,
                            ))
                                as Arc<dyn everruns_core::traits::UserConnectionResolver>);
                        let service =
                            Arc::new(crate::domains::session_sandbox::SessionSandboxService::new(
                                db.clone(),
                                storage_store,
                                connection_resolver,
                            ));
                        event_listeners.push(Arc::new(
                            crate::domains::session_sandbox::SessionSandboxEventListener::new(
                                service.clone(),
                            ),
                        ));
                        Some(service)
                    }
                    None => {
                        tracing::warn!(
                            "FEATURE_SESSION_SANDBOX is enabled but encryption is not configured; session_sandbox lifecycle is disabled"
                        );
                        None
                    }
                },
                crate::storage::StorageBackend::InMemory(mem_db) => {
                    let github_app_minter = auth_config.github_connection.as_ref().map(|config| {
                        crate::storage::GitHubAppTokenMinter::new(
                            config.app_id.clone(),
                            config.private_key.clone(),
                        )
                    });
                    let connection_resolver = encryption.as_ref().map(|enc| {
                        Arc::new(crate::storage::DbConnectionResolver::new(
                            db.as_ref().clone(),
                            enc.as_ref().clone(),
                            github_app_minter,
                        ))
                            as Arc<dyn everruns_core::traits::UserConnectionResolver>
                    });
                    let service =
                        Arc::new(crate::domains::session_sandbox::SessionSandboxService::new(
                            db.clone(),
                            mem_db.clone(),
                            connection_resolver,
                        ));
                    event_listeners.push(Arc::new(
                        crate::domains::session_sandbox::SessionSandboxEventListener::new(
                            service.clone(),
                        ),
                    ));
                    Some(service)
                }
            }
        } else {
            None
        };
        let notification_service: Option<Arc<crate::domains::notifications::NotificationService>> =
            if notifications_enabled {
                let service = Arc::new(crate::domains::notifications::NotificationService::new(
                    db.clone(),
                ));
                let notification_listener: Arc<dyn EventListener> = Arc::new(
                    crate::domains::notifications::NotificationEventListener::new(service.clone()),
                );
                event_listeners.push(notification_listener);
                Some(service)
            } else {
                tracing::info!("Notifications disabled via feature flag");
                None
            };

        if prometheus_handle.is_some() {
            event_listeners.push(
                Arc::new(api::prometheus::PrometheusMetricsListener) as Arc<dyn EventListener>
            );
        }

        if let Some(braintrust_listener) = BraintrustListener::from_env() {
            tracing::info!("Braintrust integration enabled");
            event_listeners.push(Arc::new(braintrust_listener));
        }

        // Append custom event listeners
        event_listeners.extend(self.event_listeners);

        // =====================================================================
        // Event delivery (NATS JetStream / in-memory)
        // =====================================================================
        let event_delivery = if self.config.dev_mode {
            crate::event_delivery::EventDelivery::in_memory()
        } else {
            crate::event_delivery::EventDelivery::from_env().await
        };
        let resolve_listener_database_url = || {
            database_url
                .as_deref()
                .map(|database_url| {
                    resolve_pg_listener_database_url(database_url, database_unpooled_url.as_deref())
                })
                .transpose()
        };

        let event_service = Arc::new(services::EventService::with_listeners(
            db.clone(),
            event_delivery.clone(),
            event_listeners,
        ));

        let sse_tracker = Arc::new(crate::api::sse::SseConnectionTracker::new(
            crate::api::sse::SseConnectionLimits::from_env(),
        ));

        let event_broadcaster = if matches!(event_delivery, EventDelivery::Nats(_)) {
            tracing::info!(
                "Skipping legacy PostgreSQL event listener; NATS event delivery is active"
            );
            None
        } else if let Some(database_url) = resolve_listener_database_url()?.as_ref() {
            let broadcaster =
                crate::event_notifications::EventNotificationBroadcaster::new(database_url.clone())
                    .await;
            tracing::info!("Event notification broadcaster initialized for push-based SSE");
            Some(Arc::new(broadcaster))
        } else {
            tracing::info!(
                "Event notification broadcaster not available (DEV_MODE), SSE will use polling"
            );
            None
        };
        let notification_broadcaster = if notifications_enabled {
            if let Some(database_url) = resolve_listener_database_url()?.as_ref() {
                let broadcaster =
                    crate::notification_notifications::NotificationNotificationBroadcaster::new(
                        database_url.clone(),
                    )
                    .await;
                tracing::info!("Notification broadcaster initialized for push-based SSE");
                Some(Arc::new(broadcaster))
            } else {
                tracing::info!(
                    "Notification broadcaster not available (DEV_MODE), notification SSE will use polling"
                );
                None
            }
        } else {
            tracing::info!("Notification broadcaster disabled via feature flag");
            None
        };

        // =====================================================================
        // Phase 5: API state construction
        // =====================================================================

        // Shared virtual mount registry — serves capability-mounted content
        // from memory without DB writes. Threaded into all file-reading paths.
        let virtual_registry = Arc::new(
            crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry::new(),
        );

        let mut sessions_state = api::sessions::AppState::with_platform_definition(
            db.clone(),
            runner.clone(),
            auth_state.clone(),
            platform_definition.as_ref(),
            event_delivery.clone(),
        );
        let mut session_service = crate::domains::sessions::SessionService::with_registry(
            db.clone(),
            platform_definition.capability_registry().clone(),
        )
        .with_virtual_registry(virtual_registry.clone());
        if let Some(service) = &session_sandbox_service {
            session_service = session_service.with_session_sandbox_service(service.clone());
        }
        sessions_state.session_service = Arc::new(session_service);
        sessions_state.org_rate_limiter = org_rate_limiter.clone();
        let session_sandbox_state = session_sandbox_service.as_ref().map(|sandbox_service| {
            api::session_sandbox::AppState::new(
                db.clone(),
                sessions_state.session_service.clone(),
                sandbox_service.clone(),
                auth_state.clone(),
            )
        });
        let messages_state = api::messages::AppState::new(
            db.clone(),
            runner.clone(),
            auth_state.clone(),
            notifications_enabled,
            event_delivery.clone(),
        );
        let tool_results_state = api::tool_results::AppState::new(
            db.clone(),
            runner.clone(),
            auth_state.clone(),
            event_delivery.clone(),
        );
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
            db: db.clone(),
            session_service: Arc::new(
                crate::domains::sessions::SessionService::with_registry(
                    db.clone(),
                    platform_definition.capability_registry().clone(),
                )
                .with_virtual_registry(virtual_registry.clone()),
            ),
            event_service: event_service.clone(),
            sse_tracker: sse_tracker.clone(),
            event_broadcaster,
            auth: auth_state.clone(),
        };
        let notifications_state = notification_service.as_ref().map(|notification_service| {
            api::notifications::AppState {
                db: db.clone(),
                notification_service: notification_service.clone(),
                sse_tracker: sse_tracker.clone(),
                notification_broadcaster,
                auth: auth_state.clone(),
            }
        });
        let driver_registry = Arc::new(platform_definition.driver_registry().clone());
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
        let voice_state = api::voice::AppState::new(
            db.clone(),
            auth_state.clone(),
            feature_flags.clone(),
            api::voice::AppDependencies {
                runner: runner.clone(),
                message_service: messages_state.message_service.clone(),
                llm_resolver: llm_resolver.clone(),
                event_delivery: event_delivery.clone(),
            },
            platform_definition.as_ref(),
        );
        let capability_service = Arc::new(
            services::CapabilityService::with_registry(
                db.clone(),
                encryption.clone(),
                platform_definition.capability_registry().clone(),
            )
            .with_mcp_egress_service(platform_definition.egress_service()),
        );
        let mcp_server_service = Arc::new(
            crate::domains::mcp_servers::McpServerService::with_egress_service(
                db.clone(),
                encryption.clone(),
                platform_definition.egress_service(),
            ),
        );
        let command_service = Arc::new(
            crate::domains::session_commands::SessionCommandService::new(
                db.clone(),
                event_service.clone(),
                llm_resolver.clone(),
                mcp_server_service.clone(),
                platform_definition.capability_registry().clone(),
                driver_registry.as_ref().clone(),
                sqldb_store.clone(),
            )
            .with_virtual_registry(virtual_registry.clone()),
        );
        let mcp_servers_state = api::mcp_servers::AppState::new(
            db.clone(),
            encryption.clone(),
            capability_service.clone(),
            auth_state.clone(),
        );
        let capabilities_state = api::capabilities::AppState::new(
            db.clone(),
            capability_service.clone(),
            auth_state.clone(),
        );
        let grade_for_harnesses = everruns_core::DeploymentGrade::from_env();
        let harnesses_state = api::harnesses::AppState::new(
            db.clone(),
            capability_service.clone(),
            auth_state.clone(),
            grade_for_harnesses,
            platform_definition.clone(),
        );
        let harness_examples_state = api::harness_examples::AppState {
            auth: auth_state.clone(),
            grade: grade_for_harnesses,
            platform_definition: platform_definition.clone(),
        };
        let commands_state = api::commands::AppState::new(
            capability_service.clone(),
            command_service,
            auth_state.clone(),
        );
        let grade = everruns_core::DeploymentGrade::from_env();
        let agent_examples_state = api::agent_examples::AppState {
            auth: auth_state.clone(),
            grade,
            platform_definition: platform_definition.clone(),
        };
        let agents_state = api::agents::AppState::new(
            db.clone(),
            capability_service.clone(),
            auth_state.clone(),
            grade,
            platform_definition.clone(),
        );
        let agent_identities_state = api::agent_identities::AppState::new(
            db.clone(),
            capability_service.clone(),
            auth_state.clone(),
        );
        let agent_identity_connections_state = api::agent_identity_connections::AppState::new(
            db.clone(),
            encryption.clone(),
            auth_state.clone(),
            platform_definition.connection_providers().clone(),
        );
        let eval_run_ctx = Arc::new(crate::domains::evals::runner::EvalRunContext {
            db: db.clone(),
            session_service: Arc::new(
                crate::domains::sessions::SessionService::new(db.clone())
                    .with_virtual_registry(virtual_registry.clone()),
            ),
            message_service: Arc::new(crate::domains::messages::MessageService::new(
                db.clone(),
                runner.clone(),
                notifications_enabled,
                event_delivery.clone(),
            )),
        });
        let evals_state = api::evals::AppState::new(db.clone(), auth_state.clone())
            .with_run_context(eval_run_ctx);
        let slack_state = api::slack_events::SlackState::new(
            db.clone(),
            encryption.clone(),
            runner.clone(),
            slack_dispatcher.clone(),
            notifications_enabled,
            event_delivery.clone(),
        );
        let app_webhooks_state = api::app_webhooks::AppWebhookState::new(
            db.clone(),
            encryption.clone(),
            runner.clone(),
            notifications_enabled,
            event_delivery.clone(),
        );
        let ag_ui_rate_limiter = match valkey_for_channel_rate_limits.clone() {
            Some(client) => {
                api::channel_rate_limit::ChannelRateLimiter::with_valkey("agui", client)
            }
            None => api::channel_rate_limit::ChannelRateLimiter::in_memory("agui"),
        };
        let a2a_rate_limiter = match valkey_for_channel_rate_limits.clone() {
            Some(client) => api::channel_rate_limit::ChannelRateLimiter::with_valkey("a2a", client),
            None => api::channel_rate_limit::ChannelRateLimiter::in_memory("a2a"),
        };
        let a2a_replay_store = match valkey_for_channel_rate_limits.clone() {
            Some(client) => api::a2a_signing::A2aReplayStore::with_valkey(client),
            None => api::a2a_signing::A2aReplayStore::in_memory(),
        };
        // FCP gets its own rate limiter namespace so the per-channel cap can
        // never be shared with — or exhausted by — AG-UI/A2A traffic.
        let fcp_rate_limiter = match valkey_for_channel_rate_limits.clone() {
            Some(client) => api::channel_rate_limit::ChannelRateLimiter::with_valkey("fcp", client),
            None => api::channel_rate_limit::ChannelRateLimiter::in_memory("fcp"),
        };
        let app_a2a_state = api::app_a2a::AppA2aState::new(
            db.clone(),
            encryption.clone(),
            runner.clone(),
            notifications_enabled,
            event_delivery.clone(),
            sse_tracker.clone(),
            a2a_rate_limiter,
            a2a_replay_store,
        );
        let ag_ui_state = api::ag_ui::AgUiState::new(
            db.clone(),
            encryption.clone(),
            runner.clone(),
            notifications_enabled,
            event_delivery.clone(),
            sse_tracker.clone(),
            ag_ui_rate_limiter,
        );
        let fcp_state = api::fcp::FcpState::new(
            db.clone(),
            encryption.clone(),
            runner.clone(),
            notifications_enabled,
            event_delivery.clone(),
            fcp_rate_limiter,
        );
        let session_files_state = api::session_files::AppState::new(
            db.clone(),
            event_service.clone(),
            auth_state.clone(),
        )
        .with_virtual_registry(virtual_registry.clone());
        let session_git_state = api::session_git::AppState::new(db.clone(), auth_state.clone());
        let session_storage_state =
            api::session_storage::AppState::new(db.clone(), encryption.clone(), auth_state.clone());
        let session_databases_state = api::session_databases::AppState::new(
            sqldb_store.clone(),
            db.clone(),
            auth_state.clone(),
        );
        let users_state = api::users::UsersState {
            db: db.clone(),
            auth: auth_state.clone(),
        };
        let resolver_state = api::resolver::AppState {
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
        let durable_state = api::durable::AppState::new(
            durable_store.clone(),
            auth_state.clone(),
            task_broadcaster.clone(),
            event_delivery.backend_name().to_string(),
        );
        durable_state.spawn_metrics_sampler();

        // Bridge durable MetricsCollector gauges to Prometheus
        if prometheus_handle.is_some() {
            api::prometheus::spawn_gauge_bridge(durable_state.metrics_collector().clone());
        }
        let scheduler_store = durable_store.clone();
        let apps_state = api::apps::AppState::new(
            db.clone(),
            encryption.clone(),
            scheduler_store.clone(),
            capability_service.clone(),
            auth_state.clone(),
        )
        .with_invocation_services(
            messages_state.session_service.clone(),
            messages_state.message_service.clone(),
        );
        let schedules_state = api::schedules::routes(
            api::schedules::ScheduleAppState::new(
                db.clone(),
                durable_store.clone(),
                auth_state.clone(),
            )
            .with_org_rate_limiter(org_rate_limiter.clone()),
        );
        let skills_state =
            api::skills::AppState::new(db.clone(), capability_service.clone(), auth_state.clone());
        let images_state = api::images::AppState::new(db.clone(), auth_state.clone());
        let mut organizations_state = api::organizations::AppState::with_harnesses(
            db.clone(),
            auth_state.clone(),
            platform_definition.built_in_harnesses().to_vec(),
        );
        organizations_state.org_rate_limiter = org_rate_limiter.clone();
        let volumes_state = api::volumes::AppState::new(db.clone(), auth_state.clone());
        let memory_stores_state = api::memory_stores::AppState::new(db.clone(), auth_state.clone());
        let knowledge_bases_state =
            api::knowledge_bases::AppState::new(db.clone(), auth_state.clone());
        let payments_state =
            api::payments::AppState::new(db.clone(), encryption.clone(), auth_state.clone());
        let reporting_state = api::reporting::AppState::new(db.clone(), auth_state.clone());
        let audit_logs_state = api::audit_logs::AppState::new(
            db.clone(),
            capability_service.clone(),
            auth_state.clone(),
        );
        let user_connections_state = api::user_connections::AppState::new(
            db.clone(),
            encryption.clone(),
            auth_state.clone(),
            auth_config.clone(),
            platform_definition.connection_providers().clone(),
            mcp_server_service,
        );
        let session_schedule_service =
            Arc::new(crate::domains::session_schedules::SessionScheduleService::new(db.clone()));
        let session_schedules_state = api::session_schedules::AppState::new(
            session_schedule_service.clone(),
            auth_state.clone(),
        );
        let session_resources_state =
            api::session_resources::AppState::new(db.clone(), auth_state.clone());

        // MCP endpoint: derive the protected-resource metadata URL from
        // auth_config.base_url. Path-derived per RFC 9728 §3.1 for resource
        // `{root}/mcp` → `{root}/.well-known/oauth-protected-resource/mcp`.
        let mcp_resource_metadata_url = format!(
            "{}/.well-known/oauth-protected-resource/mcp",
            auth::builtin::root_url_from_api_base(&auth_config.base_url)
        );
        let mcp_endpoint_state = api::mcp_endpoint::AppState::new(
            db.clone(),
            runner.clone(),
            auth_state.clone(),
            platform_definition.as_ref(),
            notifications_enabled,
            event_delivery.clone(),
            encryption.clone(),
            scheduler_store.clone(),
            capability_service.clone(),
            Some(sqldb_store.clone()),
        )
        .with_virtual_registry(virtual_registry.clone())
        .with_resource_metadata_url(mcp_resource_metadata_url);
        let mcp_endpoint_state = if let Some(service) = &session_sandbox_service {
            mcp_endpoint_state.with_session_sandbox_service(service.clone())
        } else {
            mcp_endpoint_state
        };

        let health_state = HealthState {
            auth_mode: format!("{:?}", auth_config.mode),
        };

        let feature_flags_state = api::feature_flags::AppState {
            flags: feature_flags.clone(),
        };

        let http_signing_keys_state = api::http_signing_keys::AppState::from_env();

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
            .merge(api::agent_examples::routes(agent_examples_state))
            .merge(api::agents::routes(agents_state))
            .merge(api::agent_identities::routes(agent_identities_state))
            .merge(api::agent_identity_connections::routes(
                agent_identity_connections_state,
            ))
            .merge(api::apps::routes(apps_state))
            // Evals routes are conditionally merged below based on feature flag.
            .merge(api::harness_examples::routes(harness_examples_state))
            .merge(api::harnesses::routes(harnesses_state))
            .merge(api::sessions::routes(sessions_state))
            .merge(api::messages::routes(messages_state))
            .merge(api::voice::routes(voice_state))
            .merge(api::tool_results::routes(tool_results_state))
            .merge(api::events::routes(events_state))
            .merge(api::llm_models::routes(llm_models_state))
            .merge(api::llm_providers::routes(llm_providers_state))
            .merge(api::mcp_servers::routes(mcp_servers_state))
            .merge(api::capabilities::routes(capabilities_state))
            .merge(api::session_files::routes(session_files_state))
            .merge(api::session_git::routes(session_git_state))
            .merge(api::session_resources::routes(session_resources_state))
            .merge(api::session_storage::routes(session_storage_state))
            .merge(api::session_databases::routes(session_databases_state))
            .merge(api::users::routes(users_state))
            .merge(api::resolver::routes(resolver_state))
            .merge(api::durable::routes(durable_state))
            .merge(schedules_state)
            .merge(api::images::routes(images_state))
            .merge({
                // Only mount presigned image routes when WORKER_GRPC_AUTH_TOKEN is set.
                // Without a signing secret, presigned URLs would be trivially forgeable.
                match std::env::var("WORKER_GRPC_AUTH_TOKEN")
                    .ok()
                    .filter(|s| !s.is_empty())
                {
                    Some(signing_secret) => {
                        api::internal_images::routes(api::internal_images::AppState {
                            db: db.clone(),
                            signing_secret,
                        })
                    }
                    None => Router::new(),
                }
            })
            .merge(api::skills::routes(skills_state))
            .merge(api::organizations::routes(organizations_state))
            .merge(api::org_feature_flags::routes(
                api::org_feature_flags::AppState::new(
                    db.clone(),
                    auth_state.clone(),
                    feature_flags.clone(),
                ),
            ))
            .merge(api::volumes::routes(volumes_state))
            .merge(api::memory_stores::routes(memory_stores_state))
            .merge(api::knowledge_bases::routes(knowledge_bases_state))
            .merge(api::payments::routes(payments_state))
            .merge(api::reporting::routes(reporting_state))
            .merge(api::user_connections::routes(user_connections_state))
            .merge(api::session_schedules::routes(session_schedules_state))
            .merge(api::audit_logs::routes(audit_logs_state))
            .merge(api::commands::routes(commands_state))
            .merge(api::slack_events::routes(slack_state))
            .merge(api::app_webhooks::routes(app_webhooks_state))
            .merge(api::app_a2a::routes(app_a2a_state))
            .merge(api::ag_ui::routes(ag_ui_state))
            .merge(api::fcp::routes(fcp_state))
            .merge(api::feature_flags::routes(feature_flags_state))
            .merge(api::budgets::routes(api::budgets::AppState::new(
                db.clone(),
                budget_service.clone(),
                auth_state.clone(),
            )));

        if let Some(notifications_state) = notifications_state {
            api_routes = api_routes.merge(api::notifications::routes(notifications_state));
        }

        if feature_flags.evals {
            api_routes = api_routes.merge(api::evals::routes(evals_state));
        } else {
            tracing::info!("Evals disabled via feature flag");
        }

        if let Some(session_sandbox_state) = session_sandbox_state {
            api_routes = api_routes.merge(api::session_sandbox::routes(session_sandbox_state));
        }

        // Personal access token CRUD — auth-provider-agnostic, always mounted.
        // Embedders can wrap this router via `wrap_personal_access_token_routes()` to attach
        // route-specific middleware (e.g. stricter rate limits) without
        // re-mounting and duplicating the paths.
        let pat_router =
            crate::auth::personal_access_token_routes(crate::auth::PersonalAccessTokenState {
                db: db.clone(),
                auth: auth_state.clone(),
                resource_limits: crate::server::ResourceLimitsConfig::from_env(),
            });
        let pat_router = apply_personal_access_token_routes_wrap(
            self.personal_access_token_routes_wrap,
            pat_router,
        );
        api_routes = api_routes.merge(pat_router);

        // Auth-specific routes (login, register, OAuth — provider-dependent)
        if let Some(auth_routes) = auth_backend.auth_routes() {
            api_routes = api_routes.merge(auth_routes);
        }

        // Extra routes (SaaS billing, custom endpoints, etc.)
        for routes in self.extra_routes {
            api_routes = api_routes.merge(routes);
        }

        // TM-DOS: Global per-IP API rate limiting (applied to API routes only,
        // not /health or /metrics). Set RATE_LIMIT_API_REQUESTS_PER_MINUTE=0 to disable.
        let link_builder = api::common::UrlBuilder::from_auth_config(&auth_state.config);
        let pagination_builder = link_builder.clone();
        let api_routes = api_routes.layer(axum::middleware::from_fn(move |req, next| {
            let link_builder = link_builder.clone();
            api::common::decorate_json_response_links(link_builder, req, next)
        }));

        // AI-friendly: add `next_url` / `prev_url` to paginated list responses.
        // Layered after entity-link decoration so both can run; only mutates
        // objects shaped like PaginatedResponse.
        let api_routes = api_routes.layer(axum::middleware::from_fn(move |req, next| {
            let builder = pagination_builder.clone();
            api::common::decorate_pagination_links(builder, req, next)
        }));

        // RFC 9457: rewrite Content-Type on JSON error responses (4xx/5xx) to
        // `application/problem+json`. Runs after link decoration, which only
        // touches success responses.
        let api_routes = api_routes.layer(axum::middleware::from_fn(
            api::common::problem_json_content_type,
        ));

        let api_rate_limiter = crate::auth::rate_limit::ApiRateLimiter::from_env_with_valkey(
            valkey_for_api_rate_limits,
        );
        let api_routes = if crate::auth::rate_limit::ApiRateLimiter::is_disabled() {
            tracing::info!("API rate limiting disabled via RATE_LIMIT_API_REQUESTS_PER_MINUTE=0");
            api_routes
        } else {
            let limiter = api_rate_limiter.clone();
            api_routes.layer(axum::middleware::from_fn(move |req, next| {
                let limiter = limiter.clone();
                crate::auth::rate_limit::api_rate_limit_middleware(limiter, req, next)
            }))
        };

        let mut root_routes = Router::new();

        if feature_flags.mcp_endpoint {
            root_routes = root_routes.merge(api::mcp_endpoint::routes(mcp_endpoint_state));
        } else {
            tracing::info!("MCP endpoint disabled via feature flag");
        }

        if let Some(public_routes) = auth_backend.public_routes() {
            root_routes = root_routes.merge(public_routes);
        }

        // TM-DOS: Apply the same per-IP rate limiting to root routes (MCP, OAuth)
        // as to API routes, to prevent brute-force and DoS on unthrottled endpoints.
        let root_routes = if crate::auth::rate_limit::ApiRateLimiter::is_disabled() {
            root_routes
        } else {
            let limiter = api_rate_limiter;
            root_routes.layer(axum::middleware::from_fn(move |req, next| {
                let limiter = limiter.clone();
                crate::auth::rate_limit::api_rate_limit_middleware(limiter, req, next)
            }))
        };

        // Main router
        let mut app = Router::new()
            .route("/health", get(health).with_state(health_state))
            .route(
                "/api-doc/openapi.json",
                get(|| async { Json(ApiDoc::openapi()) }),
            )
            .merge(api::http_signing_keys::routes(http_signing_keys_state))
            .merge(root_routes)
            .merge(build_router_with_prefix(
                api_routes,
                &self.config.api_prefix,
            ));

        // Mount /metrics endpoint:
        //  - METRICS_ADDR set → dedicated internal-only server (recommended)
        //  - METRICS_ADDR unset + METRICS_PUBLIC_ON_MAIN=true → main router (dev/local)
        //  - otherwise → do not expose /metrics HTTP endpoint
        if let Some(ref handle) = prometheus_handle {
            if let Some(ref addr) = prometheus_config.metrics_addr {
                api::prometheus::spawn_metrics_server(handle.clone(), addr.clone());
            } else if prometheus_config.public_on_main {
                app = app.merge(api::prometheus::route(handle.clone()));
            } else {
                tracing::info!(
                    "Prometheus metrics HTTP endpoint not exposed on main API server; set METRICS_ADDR or METRICS_PUBLIC_ON_MAIN=true"
                );
            }
        }

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
                permissions_policy_header_value(feature_flags.voice),
            ))
            .layer(SetResponseHeaderLayer::if_not_present(
                axum::http::header::HeaderName::from_static("content-security-policy"),
                axum::http::HeaderValue::from_static(
                    "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
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

        // RequestIdLayer must be outer (run first) so TraceLayer can read the ID when
        // creating the span above. See specs/correlation-ids.md.
        let app = app.layer(RequestIdLayer);

        // Per-request access log: applied as route_layer so axum's MatchedPath
        // extractor is available for low-cardinality `route` labels. Emits one
        // tracing event per request with method, route, status, latency_ms,
        // and request_id (DEBUG for /health and /metrics, WARN for 5xx,
        // INFO otherwise). See EVE-399 / specs/correlation-ids.md.
        let app = app.route_layer(axum::middleware::from_fn(
            crate::middleware::http_access_log_layer,
        ));

        // HTTP request duration histogram: applied as route_layer so axum's
        // MatchedPath extractor is available for low-cardinality path labels.
        let app = if prometheus_handle.is_some() {
            app.route_layer(axum::middleware::from_fn(
                api::prometheus::http_metrics_layer,
            ))
        } else {
            app
        };

        // =====================================================================
        // Phase 7: Background tasks
        // =====================================================================
        let error_reporter: SharedErrorReporter = self
            .error_reporter
            .clone()
            .unwrap_or_else(|| Arc::new(NoopErrorReporter));
        if self.error_reporter.is_some() {
            tracing::info!("Embedder error reporter installed");
        }

        let server_context = ServerContext {
            db: db.clone(),
            event_service: event_service.clone(),
            event_delivery: event_delivery.clone(),
            encryption: encryption.clone(),
            runner: runner.clone(),
            driver_registry: driver_registry.clone(),
            platform_definition: platform_definition.clone(),
            email_sender: platform_definition.email_sender(),
            egress_service: platform_definition.egress_service(),
            utility_llm_service: platform_definition.utility_llm_service(),
            error_reporter: error_reporter.clone(),
        };

        if !self.config.dev_mode {
            // -- gRPC server --
            let grpc_db = db.clone();
            let grpc_encryption = encryption.clone();
            let grpc_event_service = event_service.clone();
            let grpc_runner = runner.clone();
            let grpc_addr = self.config.grpc_addr.clone();
            let grpc_platform_definition = platform_definition.clone();
            let grpc_llm_resolver = llm_resolver.clone();
            let grpc_permission_resolver = auth_state.permission_resolver.clone();

            let grpc_task_broadcaster = task_broadcaster.clone();
            let grpc_virtual_registry = virtual_registry.clone();

            tokio::spawn(async move {
                let mut grpc_svc = grpc_service::WorkerServiceImpl::with_virtual_registry(
                    (*grpc_event_service).clone(),
                    grpc_db,
                    grpc_encryption,
                    Some(grpc_runner),
                    grpc_platform_definition.as_ref().clone(),
                    Some(grpc_virtual_registry),
                    Some(grpc_llm_resolver),
                );
                if let Some(broadcaster) = grpc_task_broadcaster {
                    grpc_svc.set_task_broadcaster(broadcaster);
                }
                grpc_svc.set_permission_resolver(grpc_permission_resolver);
                // THREAT[TM-DURABLE-002]: gRPC unauthenticated access
                // Mitigation: Bearer token auth + optional mTLS
                let grpc_token = grpc_service::require_grpc_auth_token();
                let auth_interceptor = grpc_service::GrpcAuthInterceptor::new(Some(grpc_token));
                let tls_config = grpc_service::grpc_server_tls_from_env();
                let addr = grpc_addr
                    .parse()
                    .expect("Invalid SERVER_GRPC_BIND_ADDR/WORKER_GRPC_ADDR");
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
                    let reclaim_error_reporter = error_reporter.clone();

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
                                Ok(result) => {
                                    if !result.reclaimed_ids.is_empty() {
                                        tracing::info!(
                                            count = result.reclaimed_ids.len(),
                                            task_ids = ?result.reclaimed_ids,
                                            "Reclaimed stale tasks"
                                        );
                                    }

                                    // Notify workflows about dead tasks so they can
                                    // transition to failed instead of staying stuck.
                                    for dead in &result.dead_tasks {
                                        let error_msg = dead.last_error.clone().unwrap_or_else(|| {
                                            "Worker became unresponsive after exhausting all retry attempts".to_string()
                                        });
                                        tracing::info!(
                                            task_id = %dead.task_id,
                                            workflow_id = ?dead.workflow_id,
                                            activity_id = %dead.activity_id,
                                            "Notifying workflow of dead task"
                                        );
                                        everruns_durable::record_activity_failed(
                                            &store,
                                            dead.workflow_id,
                                            dead.activity_id.clone(),
                                            error_msg,
                                            false,
                                        )
                                        .await;
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Failed to reclaim stale tasks: {}", e);
                                    // Detach reporter call so a slow vendor reporter cannot stall
                                    // the reclamation loop during an upstream outage.
                                    let reporter = reclaim_error_reporter.clone();
                                    let err_msg = e.to_string();
                                    tokio::spawn(async move {
                                        reporter
                                            .report(
                                                ErrorReport::error(
                                                    "server.stale_task_reclaim",
                                                    err_msg,
                                                )
                                                .with_scope(
                                                    ErrorScope::new()
                                                        .with_component("stale_task_reclaim"),
                                                ),
                                            )
                                            .await;
                                    });
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
                let mcp_server_service = Arc::new(
                    crate::domains::mcp_servers::McpServerService::with_egress_service(
                        db.clone(),
                        encryption.clone(),
                        platform_definition.egress_service(),
                    ),
                );
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
                    platform_definition.capability_registry().clone(),
                    platform_definition.driver_registry().clone(),
                    sqldb_store.clone(),
                )
                .with_budget_service(budget_service.clone())
                .with_encryption(encryption.clone())
                .with_workflow_store(durable_store.clone())
                .with_permission_resolver(auth_state.permission_resolver.clone())
                .with_utility_llm_service(platform_definition.utility_llm_service())
                .with_egress_service(platform_definition.egress_service())
                .with_virtual_registry(virtual_registry.clone())
                .with_storage_store(session_storage_store)
                .with_runner(runner.clone())
                .with_org_rate_limiter(Arc::new(org_rate_limiter.clone()));

                // Wire lazy connection resolver (requires encryption for token decryption).
                // Without encryption (e.g. DEV_MODE without SECRETS_ENCRYPTION_KEY) we cannot
                // decrypt stored tokens, so install a no-op resolver instead of leaving the
                // slot empty — `runtime_host::connection_resolver()` always calls into the
                // adapter at runtime and would otherwise panic.
                let connection_resolver: Arc<dyn everruns_core::traits::UserConnectionResolver> =
                    if let Some(ref enc) = encryption {
                        let github_app_minter =
                            auth_config.github_connection.as_ref().map(|config| {
                                crate::storage::GitHubAppTokenMinter::new(
                                    config.app_id.clone(),
                                    config.private_key.clone(),
                                )
                            });
                        Arc::new(crate::storage::DbConnectionResolver::new(
                            db.as_ref().clone(),
                            enc.as_ref().clone(),
                            github_app_minter,
                        ))
                    } else {
                        Arc::new(crate::storage::NoopConnectionResolver)
                    };
                adapters = adapters.with_connection_resolver(connection_resolver);

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
            let recovery_enc = encryption.clone();
            tokio::spawn(async move {
                dispatcher.recover(recovery_enc.as_ref()).await;
            });
        }

        // -- Durable task scheduler (both prod and dev) --
        if let Some(store) = scheduler_store {
            if let Err(e) =
                crate::leased_resource_scheduler::ensure_leased_resource_cleanup_schedule(
                    store.clone(),
                )
                .await
            {
                tracing::error!(error = %e, "Failed to bootstrap leased-resource cleanup schedule");
            }
            let scheduler = everruns_durable::DurableScheduler::with_defaults(
                store,
                format!("scheduler-{}", uuid::Uuid::now_v7()),
            );
            let _scheduler_shutdown = scheduler.spawn();
            tracing::info!("Durable task scheduler started");
        }

        // -- Tool result timeout sweep (both prod and dev) --
        crate::tool_result_timeout::spawn_tool_result_timeout_sweep(
            db.clone(),
            runner.clone(),
            event_delivery.clone(),
        );

        // -- Session schedule poller (both prod and dev) --
        crate::session_scheduler::spawn_session_scheduler(
            db.clone(),
            session_schedule_service,
            event_service,
            runner,
            std::time::Duration::from_secs(15),
        );

        // -- Source-backed workspace volume sync (both prod and dev) --
        let volume_connection_resolver = encryption.as_ref().map(|enc| {
            let github_app_minter = auth_config.github_connection.as_ref().map(|config| {
                crate::storage::GitHubAppTokenMinter::new(
                    config.app_id.clone(),
                    config.private_key.clone(),
                )
            });
            Arc::new(crate::storage::DbConnectionResolver::new(
                db.as_ref().clone(),
                enc.as_ref().clone(),
                github_app_minter,
            )) as Arc<dyn everruns_core::traits::UserConnectionResolver>
        });
        crate::domains::volumes::source_sync::spawn_volume_source_sync_task(
            db.clone(),
            volume_connection_resolver,
        );

        // -- Reporting projection and missing-work reconciliation (both prod and dev) --
        crate::domains::reporting::background::spawn_reporting_background_task(db.clone());

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
            let (tcp, addr) = listener
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
                        let addr = addr;
                        async move {
                            let mut req = req.map(axum::body::Body::new);
                            req.extensions_mut()
                                .insert(axum::extract::ConnectInfo(addr));
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

    #[test]
    fn permissions_policy_denies_microphone_by_default() {
        assert_eq!(
            permissions_policy_header_value(false).to_str().unwrap(),
            "camera=(), microphone=(), geolocation=()"
        );
    }

    #[test]
    fn permissions_policy_allows_microphone_when_voice_is_enabled() {
        assert_eq!(
            permissions_policy_header_value(true).to_str().unwrap(),
            "camera=(), microphone=(self), geolocation=()"
        );
    }

    // EVE-401: embedders can layer route-specific middleware on the auto-mounted
    // personal access token CRUD router via `wrap_personal_access_token_routes()` without re-mounting.
    #[tokio::test]
    async fn wrap_personal_access_token_routes_applies_custom_layer() {
        use axum::body::Body;
        use axum::http::{HeaderName, HeaderValue, Request, StatusCode};
        use axum::routing::get;
        use tower::ServiceExt;
        use tower_http::set_header::SetResponseHeaderLayer;

        let base = Router::new().route("/v1/auth/personal-access-tokens", get(|| async { "ok" }));
        let wrap: PersonalAccessTokenRoutesWrapFn = Box::new(|r: Router| {
            r.layer(SetResponseHeaderLayer::if_not_present(
                HeaderName::from_static("x-eve-401-marker"),
                HeaderValue::from_static("applied"),
            ))
        });

        let router = apply_personal_access_token_routes_wrap(Some(wrap), base);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/v1/auth/personal-access-tokens")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("x-eve-401-marker").unwrap(),
            "applied"
        );
    }

    #[tokio::test]
    async fn wrap_personal_access_token_routes_passthrough_when_none() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use axum::routing::get;
        use tower::ServiceExt;

        let base = Router::new().route("/v1/auth/personal-access-tokens", get(|| async { "ok" }));
        let router = apply_personal_access_token_routes_wrap(None, base);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/v1/auth/personal-access-tokens")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("x-eve-401-marker").is_none());
    }
}

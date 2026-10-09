mod background;
mod channels;
mod http_layers;
mod integrations;
mod listeners;
mod serve;

// Server app builder for composable server configurations
//
// Decision: Builder pattern lets downstream crates compose custom server setups
//   by adding routes, event listeners, migrations, and auth backends.
// Decision: ServerContext exposes shared infrastructure for background tasks.
// Decision: `run()` reads top to bottom as the startup phases. Each phase with
//   more than a screen of wiring lives in its own file:
//   4 event listeners `listeners.rs`; 5 channel states `channels.rs`;
//   6 middleware `http_layers.rs`; 7 worker link and background loops
//   `background.rs` and `health.rs`; 8 the HTTP serve loop `serve.rs`.

use crate::api::sse::{SseConnectionLimits, SseConnectionTracker};
use crate::auth::{self, AuthBackend};
use crate::event_delivery::EventDelivery;
use crate::openapi::ApiDoc;
use crate::pg_listener_config::resolve_pg_listener_database_url;
use crate::server::{ServerConfig, build_router_with_prefix};
use crate::storage::{EncryptionService, StorageBackend};
use crate::supervised_task::TaskSupervisor;
use crate::{api, org_init, seed, services};
use everruns_core::host::HostComposition;

mod health;

use anyhow::{Context, Result};
use axum::{Json, Router, routing::get};
use everruns_core::{ErrorReporter, EventListener, NoopErrorReporter, SharedErrorReporter};
use everruns_durable::{PostgresWorkflowEventStore, WorkflowEventStore};
use sqlx::PgPool;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use utoipa::OpenApi;

// =========================================================================
// Types
// =========================================================================

type AuthFactoryFn =
    Box<dyn FnOnce(Arc<StorageBackend>, Arc<HostComposition>) -> Arc<dyn AuthBackend> + Send>;

pub(crate) type MigrationFn =
    Box<dyn FnOnce(PgPool) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send>;

type BackgroundTaskFn =
    Box<dyn FnOnce(ServerContext) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

type PersonalAccessTokenRoutesWrapFn = Box<dyn FnOnce(Router) -> Router + Send>;

fn github_app_token_minter(
    auth_config: &auth::AuthConfig,
) -> Option<crate::storage::GitHubAppTokenMinter> {
    auth_config.github_connection.as_ref().map(|config| {
        crate::storage::GitHubAppTokenMinter::new(config.app_id.clone(), config.private_key.clone())
    })
}

pub(super) fn build_connection_resolver(
    db: &Arc<StorageBackend>,
    encryption: &Arc<EncryptionService>,
    auth_config: &auth::AuthConfig,
    egress: Arc<dyn everruns_core::EgressService>,
) -> Arc<dyn everruns_core::connection_services::UserConnectionResolver> {
    Arc::new(crate::storage::DbConnectionResolver::new(
        db.as_ref().clone(),
        encryption.as_ref().clone(),
        github_app_token_minter(auth_config),
        egress,
    ))
}

fn optional_connection_resolver(
    db: &Arc<StorageBackend>,
    encryption: &Option<Arc<EncryptionService>>,
    auth_config: &auth::AuthConfig,
    egress: Arc<dyn everruns_core::EgressService>,
) -> Option<Arc<dyn everruns_core::connection_services::UserConnectionResolver>> {
    encryption
        .as_ref()
        .map(|enc| build_connection_resolver(db, enc, auth_config, egress))
}

fn spawn_background_tasks(
    supervisor: &mut TaskSupervisor,
    server_context: &ServerContext,
    background_tasks: Vec<BackgroundTaskFn>,
) {
    crate::agents_api_lifecycle::track(supervisor, server_context);
    for task_fn in background_tasks {
        let ctx = server_context.clone();
        supervisor.track("custom_background_task", tokio::spawn(task_fn(ctx)));
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
    pub runner: Arc<dyn everruns_core::host::TurnBackend>,
    pub driver_registry: Arc<everruns_contracts::driver_registry::DriverRegistry>,
    pub host_composition: Arc<HostComposition>,
    /// System-wide email sender from the platform profile.
    pub email_sender: Arc<dyn crate::records::email::EmailSender>,
    /// System-wide outbound egress service from the platform profile.
    pub egress_service: Arc<dyn everruns_core::EgressService>,
    /// System-wide utility LLM service from the platform profile.
    pub utility_llm_service: Arc<dyn everruns_core::UtilityLlmService>,
    /// Vendor-neutral embedder-provided error reporter. Always present;
    /// defaults to a no-op when no embedder has installed one.
    pub error_reporter: SharedErrorReporter,
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
    host_composition: Option<HostComposition>,
    built_in_harnesses: Option<Vec<crate::records::BuiltInHarnessDefinition>>,
    connector_registry: Option<everruns_contracts::connector::ConnectorRegistry>,
    email_sender: Option<Arc<dyn crate::records::email::EmailSender>>,
    slack_app_provisioner: Option<Arc<dyn crate::records::slack_provisioning::SlackAppProvisioner>>,
    extra_routes: Vec<Router>,
    event_listeners: Vec<Arc<dyn EventListener>>,
    error_reporter: Option<SharedErrorReporter>,
    migrations: Vec<MigrationFn>,
    background_tasks: Vec<BackgroundTaskFn>,
    personal_access_token_routes_wrap: Option<PersonalAccessTokenRoutesWrapFn>,
    org_create_policy: Option<Arc<dyn api::organizations::OrgCreatePolicy>>,
    org_initializers: Vec<Arc<dyn org_init::OrgInitializer>>,
}

impl ServerAppBuilder {
    /// Create a new server app builder with the given configuration.
    pub fn new(config: ServerConfig) -> Self {
        Self {
            config,
            auth_factory: None,
            host_composition: None,
            built_in_harnesses: None,
            connector_registry: None,
            email_sender: None,
            slack_app_provisioner: None,
            extra_routes: Vec::new(),
            event_listeners: Vec::new(),
            error_reporter: None,
            migrations: Vec::new(),
            background_tasks: Vec::new(),
            personal_access_token_routes_wrap: None,
            org_create_policy: None,
            org_initializers: Vec::new(),
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
        factory: impl FnOnce(Arc<StorageBackend>, Arc<HostComposition>) -> Arc<dyn AuthBackend>
        + Send
        + 'static,
    ) -> Self {
        self.auth_factory = Some(Box::new(factory));
        self
    }

    /// Replace the default OSS runtime surface with an explicit platform definition.
    pub fn host_composition(mut self, host_composition: HostComposition) -> Self {
        self.host_composition = Some(host_composition);
        self
    }

    /// Replace the built-in harness provisioning templates (EVE-881).
    ///
    /// Defaults to the OSS preset (`crate::platform::oss_built_in_harnesses`).
    /// Product provisioning templates are server composition, not part of the
    /// shared `HostComposition` runtime surface.
    pub fn built_in_harnesses(
        mut self,
        harnesses: Vec<crate::records::BuiltInHarnessDefinition>,
    ) -> Self {
        self.built_in_harnesses = Some(harnesses);
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
    /// the reporter is the only contact surface.
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

    /// Register a pre-create policy for organization creation (EVE-607). It runs in the
    /// OSS `POST /v1/orgs` handler before any row is written, sees the user and requested
    /// name, and a rejection fails creation closed with a UI-facing status/body. Lets
    /// wrappers (e.g. SaaS) gate creation on product policy (verified email, limits)
    /// without forking the handler. Unset keeps OSS behavior.
    pub fn org_create_policy(
        mut self,
        policy: Arc<dyn api::organizations::OrgCreatePolicy>,
    ) -> Self {
        self.org_create_policy = Some(policy);
        self
    }

    /// Register a post-create organization initializer (EVE-811). It runs in the OSS
    /// `POST /v1/orgs` handler after built-in harnesses and the default marketplace are
    /// provisioned, so a wrapper provisions per-org resources (managed provider, budget,
    /// tenant record) in creation rather than via a reconciler. Initializers run in
    /// registration order; a required one that fails rolls the org back, an optional one
    /// only logs. None registered keeps OSS behavior.
    pub fn org_initializer(mut self, initializer: Arc<dyn org_init::OrgInitializer>) -> Self {
        self.org_initializers.push(initializer);
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
        let host_composition = Arc::new(
            self.host_composition
                .clone()
                .unwrap_or_else(crate::platform::oss_host_composition),
        );
        // Built-in harness provisioning templates are server composition
        // (EVE-881): resolved here and threaded to seeding, auth safety nets,
        // and role-based fallbacks.
        let built_in_harnesses = Arc::new(
            self.built_in_harnesses
                .clone()
                .unwrap_or_else(crate::platform::oss_built_in_harnesses),
        );
        // Connector registry and system email sender are hosted control-plane
        // services (EVE-879): composed here, not on `HostComposition`.
        let connector_registry = self
            .connector_registry
            .clone()
            .unwrap_or_else(crate::platform::oss_connector_registry);
        let email_sender = self
            .email_sender
            .clone()
            .unwrap_or_else(crate::platform::system_email_sender);
        let mut supervisor = TaskSupervisor::new();

        // =====================================================================
        // Phase 1: Storage backend & runner
        // =====================================================================
        let migrations = self.migrations;
        let crate::storage_init::StorageInit {
            db,
            runner,
            background_runner,
            shared_durable_store,
            database_url,
            database_unpooled_url,
            task_broadcaster,
        } = crate::storage_init::init_storage(&self.config, migrations).await?;

        // Background loops draw from a pool of their own (EVE-1081). Prod had
        // the durable scheduler, the observer scoring worker and the sweeps
        // around them all report `pool timed out while waiting for an open
        // connection` within two seconds of each other, on the same trace as
        // user-facing 500s, because every one of them was queued on the single
        // request pool. Handing them this backend routes their whole call graph
        // onto the reserved pool without changing a query.
        let background_db = Arc::new(db.for_background());

        // =====================================================================
        // Phase 2: Seed & infrastructure services
        // =====================================================================
        let auth_config = auth::AuthConfig::from_env();

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
        let slack_provisioning = crate::slack_provisioning::configure(
            &mut supervisor,
            db.clone(),
            encryption.clone(),
            self.slack_app_provisioner.clone(),
        )?;
        let slack_provisioner = slack_provisioning.provisioner.clone();

        // Seed must run after encryption is resolved: single-tenant/dev seeding materializes
        // DEFAULT_*_API_KEY env vars into the default org's (encrypted) provider rows.
        supervisor.track(
            "seed",
            seed::prepare_seed_task(
                db.clone(),
                &auth_config,
                host_composition.as_ref().clone(),
                built_in_harnesses.as_ref().clone(),
                encryption.clone(),
            )
            .await?,
        );

        let sqldb_backend = Arc::new(crate::session_sqldb::InMemorySqlDbBackend::new());
        let sqldb_store: Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore> =
            Arc::new(crate::session_sqldb::InMemorySqlDbStore::new(sqldb_backend));
        tracing::info!("Session SQL database store initialized (in-memory)");

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
            Some(factory) => factory(db.clone(), host_composition.clone()),
            None => {
                let cfg = auth_config.clone();
                if let Some(valkey) = valkey_client {
                    Arc::new(
                        auth::BuiltinAuthBackend::with_valkey(
                            cfg,
                            db.clone(),
                            host_composition.clone(),
                            valkey,
                        )
                        .with_built_in_harnesses(built_in_harnesses.clone())
                        .with_email_sender(email_sender.clone()),
                    )
                } else {
                    Arc::new(
                        auth::BuiltinAuthBackend::new(cfg, db.clone(), host_composition.clone())
                            .with_built_in_harnesses(built_in_harnesses.clone())
                            .with_email_sender(email_sender.clone()),
                    )
                }
            }
        };
        let feature_flag_policy = crate::records::FeatureFlagPolicy::current();
        let feature_flags = feature_flag_policy.deployment_flags();
        let auth_state = auth::AuthState::new(auth_config.clone(), auth_backend.clone())
            .with_db(db.clone())
            .with_feature_flag_policy(feature_flag_policy.clone());
        let notifications_enabled = feature_flags.notifications;
        tracing::info!(?feature_flags, "Feature flags computed");

        // Prometheus metrics
        let prometheus_config = api::prometheus::PrometheusConfig::from_env();
        let prometheus_handle = if prometheus_config.enabled {
            let handle = api::prometheus::install_prometheus_recorder();
            if handle.is_some() {
                let addr = prometheus_config.metrics_addr.as_deref();
                tracing::info!(addr = addr.unwrap_or("main"), "Prometheus metrics on");
            }
            handle
        } else {
            tracing::info!("Prometheus metrics disabled");
            None
        };

        // =====================================================================
        // Phase 4: Event listeners & domain/infra helpers
        // =====================================================================
        let listeners::Listeners {
            event_listeners,
            budget_service,
            mcp_events,
            mcp_event_triggers,
            thread_turns,
            session_sandbox_service,
            notification_service,
            observer_wake,
        } = listeners::build(
            listeners::ListenerDeps {
                db: db.clone(),
                encryption: encryption.clone(),
                host_composition: host_composition.clone(),
                auth_state: auth_state.clone(),
                auth_config: auth_config.clone(),
                notifications_enabled,
                observers_enabled: feature_flags.observers,
                prometheus_enabled: prometheus_handle.is_some(),
            },
            self.event_listeners,
        );

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
            event_listeners.clone(),
        ));
        thread_turns.bind_registry(&db, &event_service, &runner);
        let background_event_service = Arc::new(services::EventService::with_listeners(
            background_db.clone(),
            event_delivery.clone(),
            event_listeners,
        ));

        let sse_tracker = Arc::new(SseConnectionTracker::new(SseConnectionLimits::from_env()));

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

        let mut sessions_state = api::sessions::AppState::with_host_composition(
            db.clone(),
            runner.clone(),
            auth_state.clone(),
            host_composition.as_ref(),
            &built_in_harnesses,
            event_delivery.clone(),
        );
        let mut session_service = crate::domains::sessions::SessionService::with_registry(
            db.clone(),
            (*host_composition.capability_registry()).clone(),
        )
        .with_virtual_registry(virtual_registry.clone());
        if let Some(service) = &session_sandbox_service {
            session_service = session_service.with_session_sandbox_service(service.clone());
        }
        sessions_state.session_service = Arc::new(session_service);
        sessions_state.org_rate_limiter = org_rate_limiter.clone();
        // Captured for the stale-task reclaim loop so it can surface sealed turns
        // (forward-progress guard, EVE-534). `sessions_state` is moved into the
        // router later, so grab a clone of the service now.
        let reclaim_session_service = sessions_state.session_service.clone();
        let sandbox_templates_state = api::sandbox_templates::AppState::new(
            db.clone(),
            sessions_state.session_service.clone(),
            auth_state.clone(),
        );
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
            sse_tracker.clone(),
        );
        let tool_results_state = api::tool_results::AppState::new(
            db.clone(),
            runner.clone(),
            auth_state.clone(),
            event_delivery.clone(),
        );
        // Slack delivery dispatcher, always on: without the PostgreSQL listener
        // (NATS) it polls sessions (EVERRUNS-2B) and takes bus deltas (EVE-1211).
        let slack_wake = match event_broadcaster {
            Some(ref broadcaster) => broadcaster.subscribe().into(),
            None => crate::slack_delivery::DeliveryWake::Poll,
        };
        let slack_dispatcher = crate::slack_delivery::SlackDeliveryDispatcher::start(
            db.clone(),
            slack_wake,
            auth_config.frontend_url.clone(),
        );
        slack_dispatcher.feed_live_deltas(&event_delivery);
        let events_state = api::events::AppState {
            db: db.clone(),
            session_service: Arc::new(
                crate::domains::sessions::SessionService::with_registry(
                    db.clone(),
                    (*host_composition.capability_registry()).clone(),
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
        let driver_registry = Arc::new(host_composition.driver_registry().clone());
        let provider_resolver = Arc::new(
            services::ProviderResolverService::new(db.clone(), encryption.clone())
                .with_driver_registry((*driver_registry).clone()),
        );

        // Now that the LLM stack exists, spawn the observer scoring worker with
        // an LLM-judge client backed by the org's own configured providers.
        if let Some(observer_wake) = observer_wake {
            let judge: Arc<dyn crate::domains::observers::JudgeClient> =
                Arc::new(crate::domains::observers::LlmJudgeClient::new(
                    background_db.clone(),
                    driver_registry.clone(),
                    provider_resolver.clone(),
                ));
            supervisor.track(
                "observer_scoring",
                crate::domains::observers::spawn_observer_worker(
                    crate::domains::observers::ObserverWorkerDeps {
                        db: background_db.clone(),
                        judge: Some(judge),
                    },
                    observer_wake,
                    crate::domains::observers::ObserverWorkerConfig::default(),
                ),
            );
            tracing::info!("Observers enabled: scoring worker started");
        }

        let providers_state = api::providers::AppState::new(
            db.clone(),
            encryption.clone(),
            driver_registry.clone(),
            auth_state.clone(),
            Some(provider_resolver.clone()),
        );
        let models_state = api::models::AppState::new(
            db.clone(),
            auth_state.clone(),
            Some(provider_resolver.clone()),
        );
        let voice_state = api::voice::AppState::new(
            db.clone(),
            auth_state.clone(),
            feature_flags.clone(),
            api::voice::AppDependencies {
                runner: runner.clone(),
                message_service: messages_state.message_service.clone(),
                provider_resolver: provider_resolver.clone(),
                event_delivery: event_delivery.clone(),
            },
            host_composition.as_ref(),
            &built_in_harnesses,
        );
        let capability_service = Arc::new(
            services::CapabilityService::with_registry(
                db.clone(),
                encryption.clone(),
                (*host_composition.capability_registry()).clone(),
            )
            .with_mcp_egress_service(host_composition.egress_service()),
        );
        let mcp_server_service = Arc::new(
            crate::domains::mcp_servers::McpServerService::with_egress_service(
                db.clone(),
                encryption.clone(),
                host_composition.egress_service(),
            ),
        );
        let command_service = Arc::new(
            crate::domains::session_commands::SessionCommandService::new(
                db.clone(),
                event_service.clone(),
                provider_resolver.clone(),
                mcp_server_service.clone(),
                (*host_composition.capability_registry()).clone(),
                driver_registry.as_ref().clone(),
                sqldb_store.clone(),
            )
            .with_virtual_registry(virtual_registry.clone()),
        );
        let plugins_state =
            api::plugins::AppState::new(db.clone(), capability_service.clone(), auth_state.clone());
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
            host_composition.clone(),
        );
        let harness_examples_state = api::harness_examples::AppState {
            auth: auth_state.clone(),
            grade: grade_for_harnesses,
            host_composition: host_composition.clone(),
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
            host_composition: host_composition.clone(),
        };
        let agents_state = api::agents::AppState::new(
            db.clone(),
            encryption.clone(),
            capability_service.clone(),
            auth_state.clone(),
            grade,
            host_composition.clone(),
            built_in_harnesses.clone(),
        )
        .with_org_rate_limiter(org_rate_limiter.clone())
        .with_slack_provisioner(slack_provisioner.clone());
        let virtual_user_connections_state = api::virtual_user_connections::AppState::new(
            db.clone(),
            encryption.clone(),
            auth_state.clone(),
            connector_registry.clone(),
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
            // Judged scorers (citation_judged) use the org's own configured model,
            // mirroring the observer LLM judge.
            judge: Some(Arc::new(crate::domains::observers::LlmJudgeClient::new(
                db.clone(),
                driver_registry.clone(),
                provider_resolver.clone(),
            ))),
        });
        let evals_state = api::evals::AppState::new(db.clone(), auth_state.clone())
            .with_run_context(eval_run_ctx);
        // Agent health checks are shared by HTTP and MCP (knowledge/evaluation/agent-checks.md).
        // They need a utility LLM to generate/judge cases and a default harness to host sessions.
        let utility_llm = host_composition.utility_llm_service();
        let health_check_service: Option<Arc<crate::domains::agents::AgentHealthCheckService>> =
            if utility_llm.is_configured()
                && let Some(default_harness_name) = crate::records::harness_for_role(
                    &built_in_harnesses,
                    crate::records::BuiltInHarnessRole::Default,
                )
                .map(|h| h.name.clone())
            {
                let health_check_ctx = Arc::new(crate::domains::agents::HealthCheckRunContext {
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
                    utility_llm_service: utility_llm,
                    default_harness_name,
                });
                Some(Arc::new(
                    crate::domains::agents::AgentHealthCheckService::new(
                        health_check_ctx,
                        capability_service.clone(),
                    ),
                ))
            } else {
                None
            };
        let agents_state = match &health_check_service {
            Some(svc) => agents_state.with_health_check_service(svc.clone()),
            None => agents_state,
        };
        let slack_state = api::slack_events::SlackState::new(
            db.clone(),
            encryption.clone(),
            runner.clone(),
            Some(slack_dispatcher.clone()),
            notifications_enabled,
            event_delivery.clone(),
            auth_config.base_url.clone(),
        )
        .with_decisions(&host_composition, &provider_resolver, &budget_service);
        let channel_states = channels::ChannelStates::build(channels::ChannelDeps {
            db: db.clone(),
            encryption: encryption.clone(),
            runner: runner.clone(),
            notifications_enabled,
            event_delivery: event_delivery.clone(),
            sse_tracker: sse_tracker.clone(),
            valkey: valkey_for_channel_rate_limits,
            auth: auth_state.clone(),
            frontend_url: auth_config.frontend_url.clone(),
            public_chat_enabled: feature_flags.public_chat,
            mcp_event_triggers: mcp_event_triggers.clone(),
        });
        let session_files_state = api::session_files::AppState::new(
            db.clone(),
            event_service.clone(),
            auth_state.clone(),
        )
        .with_virtual_registry(virtual_registry.clone());
        let session_git_state = api::session_git::AppState::new(db.clone(), auth_state.clone());
        let session_databases_state = api::session_databases::AppState::new(
            sqldb_store.clone(),
            db.clone(),
            auth_state.clone(),
        );
        let durable_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>> =
            if let Some(ref shared_store) = shared_durable_store {
                Some(shared_store.clone() as Arc<dyn WorkflowEventStore + Send + Sync>)
            } else {
                Some(Arc::new(PostgresWorkflowEventStore::new(db.pool().clone()))
                    as Arc<dyn WorkflowEventStore + Send + Sync>)
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
            api::prometheus::spawn_storage_pool_gauge_bridge(&db);
        }
        let scheduler_store = durable_store.clone();
        // The durable scheduler's own store runs on the background pool
        // (EVE-1081). `claim_due_schedules` is the loop that Sentry
        // EVERRUNS-15/16 caught timing out; the request-path copies above keep
        // the request pool. Dev mode has no pools, so it shares the store.
        let background_scheduler_store: Option<Arc<dyn WorkflowEventStore + Send + Sync>> =
            if shared_durable_store.is_some() {
                durable_store.clone()
            } else {
                Some(Arc::new(PostgresWorkflowEventStore::new(
                    db.background_pool().clone(),
                ))
                    as Arc<dyn WorkflowEventStore + Send + Sync>)
            };
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
        let agent_triggers_state = api::agent_triggers::AppState::new(
            db.clone(),
            encryption.clone(),
            scheduler_store.clone(),
            capability_service.clone(),
            auth_state.clone(),
            messages_state.session_service.clone(),
            messages_state.message_service.clone(),
        )
        .with_mcp_event_triggers(mcp_event_triggers.clone());
        let schedules_state = api::schedules::routes(
            api::schedules::ScheduleAppState::new(
                db.clone(),
                durable_store.clone(),
                auth_state.clone(),
            )
            .with_org_rate_limiter(org_rate_limiter.clone()),
        );
        let mut organizations_state = api::organizations::AppState::with_harnesses(
            db.clone(),
            auth_state.clone(),
            built_in_harnesses.as_ref().clone(),
        );
        organizations_state.org_rate_limiter = org_rate_limiter.clone();
        organizations_state.org_create_policy = self.org_create_policy;
        organizations_state.org_initializers = self.org_initializers;
        let org_invitations_state = api::org_invitations::AppState::new(
            db.clone(),
            auth_state.clone(),
            email_sender.clone(),
            auth_config.frontend_url.clone(),
        );
        let workspace_files_state =
            api::workspace_files::AppState::new(db.clone(), auth_state.clone())
                .with_virtual_registry(virtual_registry.clone());
        let knowledge_indexes_state = api::knowledge_indexes::AppState::new(
            db.clone(),
            auth_state.clone(),
            driver_registry.clone(),
        );
        let reporting_state = api::reporting::AppState::new(db.clone(), auth_state.clone());
        let user_connections_state = api::user_connections::AppState::new(
            db.clone(),
            encryption.clone(),
            auth_state.clone(),
            auth_config.clone(),
            connector_registry.clone(),
            mcp_server_service,
        );
        let background_session_schedule_service = Arc::new(
            crate::domains::session_schedules::SessionScheduleService::new(background_db.clone()),
        );
        let session_schedules_state =
            api::session_schedules::AppState::new(db.clone(), auth_state.clone());
        let session_tasks_state = api::session_tasks::AppState::new(
            db.clone(),
            auth_state.clone(),
            event_service.clone(),
        );

        // MCP endpoint: derive the protected-resource metadata URL from
        // auth_config.base_url. Path-derived per RFC 9728 §3.1 for resource
        // `{root}/mcp` → `{root}/.well-known/oauth-protected-resource/mcp`.
        let mcp_root_url = auth::builtin::root_url_from_api_base(&auth_config.base_url);
        let mcp_resource_metadata_url =
            format!("{mcp_root_url}/.well-known/oauth-protected-resource/mcp");
        // Canonical MCP resource (`{root}/mcp`) that MCP OAuth tokens are bound
        // to (RFC 8707). Must match the `aud` minted in `mcp_oauth.rs` so the
        // audience check in `validate_mcp_token` accepts real MCP tokens (TM-MCP-006).
        let mcp_resource = format!("{mcp_root_url}/mcp");
        let mcp_endpoint_state = api::mcp_endpoint::AppState::new(
            db.clone(),
            runner.clone(),
            auth_state.clone(),
            host_composition.as_ref(),
            &built_in_harnesses,
            notifications_enabled,
            event_delivery.clone(),
            encryption.clone(),
            scheduler_store.clone(),
            capability_service.clone(),
            Some(sqldb_store.clone()),
        )
        .with_connector_registry(connector_registry.clone())
        .with_org_rate_limiter(org_rate_limiter.clone())
        .with_virtual_registry(virtual_registry.clone())
        .with_resource_metadata_url(mcp_resource_metadata_url)
        .with_mcp_resource(mcp_resource)
        // URL mode elicitation pages hang off the same root `/mcp` is served
        // under, so a client that can reach the endpoint can reach the page.
        .with_elicitation_base_url(mcp_root_url.clone())
        .with_mcp_events(mcp_events)
        .with_slack_provisioner(slack_provisioner.clone())
        .with_provider_services((&providers_state).into());
        let mcp_endpoint_state = match &session_sandbox_service {
            Some(service) => mcp_endpoint_state.with_session_sandbox_service(service.clone()),
            None => mcp_endpoint_state,
        };
        let mcp_endpoint_state = match &health_check_service {
            Some(svc) => mcp_endpoint_state.with_health_check_service(svc.clone()),
            None => mcp_endpoint_state,
        };
        let api_state = api::state::ApiState::from_mcp(&mcp_endpoint_state);

        let health_state = health::HealthState {
            auth_mode: format!("{:?}", auth_config.mode),
        };

        let feature_flags_state = api::feature_flags::AppState {
            flags: feature_flags.clone(),
        };

        // Agent discovery: MCP server card + auth.md, both derived from the
        // live auth config so a self-hosted deployment describes itself.
        let agent_discovery_state = api::agent_discovery::AppState::for_deployment(
            mcp_root_url.clone(),
            auth_config.base_url.clone(),
            auth_config.mode.clone(),
        );
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
        // Phase 6: Build API router.
        let mut api_routes = Router::new()
            .merge(api::agent_examples::routes(agent_examples_state))
            .merge(api::agents::routes(agents_state))
            .merge(api::agent_credentials::routes(api_state.clone()))
            .merge(api::runtime_auth::routes(api::runtime_auth::AppState {
                db: db.clone(),
                auth: auth_state.clone(),
                encryption: encryption.clone(),
                verifier: api::channel_auth::ChannelAuthVerifier::new(),
            }))
            .merge(api::virtual_users::routes(api_state.clone()))
            .merge(api::organization_connections::routes(
                virtual_user_connections_state.clone(),
            ))
            .merge(api::virtual_user_connections::routes(
                virtual_user_connections_state,
            ))
            .merge(api::apps::routes(apps_state))
            .merge(api::agent_channels::routes(agent_triggers_state.clone()))
            .merge(api::agent_triggers::routes(agent_triggers_state))
            // Evals routes are conditionally merged below based on feature flag.
            .merge(api::harness_examples::routes(harness_examples_state))
            .merge(api::harnesses::routes(harnesses_state))
            .merge(api::sessions::routes(sessions_state))
            .merge(api::messages::routes(messages_state))
            .merge(api::voice::routes(voice_state))
            .merge(api::tool_results::routes(tool_results_state))
            .merge(api::events::routes(events_state))
            .merge(api::models::routes(models_state))
            .merge(api::providers::routes(providers_state))
            .merge(api::mcp_servers::routes(api_state.clone()))
            .merge(api::plugins::routes(plugins_state))
            .merge(api::capabilities::routes(capabilities_state))
            .merge(api::session_files::routes(session_files_state))
            .merge(api::session_git::routes(session_git_state))
            .merge(api::session_resources::routes(api_state.clone()))
            .merge(api::session_tasks::routes(session_tasks_state))
            .merge(api::session_storage::routes(api_state.clone()))
            .merge(api::session_databases::routes(session_databases_state))
            .merge(api::users::routes(api_state.clone()))
            .merge(api::resolver::routes(api_state.clone()))
            .merge(api::durable::routes(durable_state))
            .merge(schedules_state)
            .merge(api::files::routes(api_state.clone()))
            .merge(api::images::routes(api_state.clone()))
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
            .merge(api::skills::routes(api_state.clone()))
            .merge(api::organizations::routes(organizations_state))
            .merge(api::org_invitations::routes(org_invitations_state))
            .merge(api::task_webhooks::routes(api_state.clone()))
            .merge(api::org_feature_flags::routes(
                api::org_feature_flags::AppState::new(
                    db.clone(),
                    auth_state.clone(),
                    feature_flag_policy.clone(),
                ),
            ))
            .merge(api::health_issues::routes(api_state.clone()))
            .merge(api::memory::routes(api_state.clone()))
            .merge(api::workspaces::routes(api_state.clone()))
            .merge(api::workspace_files::routes(workspace_files_state))
            .merge(api::memory_files::routes(api_state.clone()))
            .merge(api::knowledge_bases::routes(api_state.clone()))
            .merge(api::knowledge_indexes::routes(knowledge_indexes_state))
            .merge(api::payments::routes(
                api_state.clone(),
                feature_flags.machine_payments,
            ))
            .merge(api::reporting::routes(reporting_state))
            .merge(api::user_connections::routes(user_connections_state))
            .merge(api::user_preferences::routes(api_state.clone()))
            .merge(api::session_schedules::routes(session_schedules_state))
            .merge(api::audit_logs::routes(api_state.clone()))
            .merge(api::commands::routes(commands_state))
            .merge(api::command_dispatch::routes(mcp_endpoint_state.clone()))
            .merge(api::slack_events::routes(slack_state.clone()))
            .merge(api::slack_install::routes(
                api::slack_install::SlackInstallState::new(
                    slack_state,
                    auth_state.clone(),
                    auth_config.frontend_url.clone(),
                    slack_provisioning,
                ),
            ))
            .merge(api::channel_webhooks::routes(channel_states.webhooks))
            .merge(api::channel_a2a::routes(channel_states.a2a))
            .merge(api::channel_api::routes(channel_states.api))
            .merge(api::ag_ui::routes(channel_states.ag_ui))
            .merge(api::public_chat::routes(channel_states.public_chat))
            .merge(api::fcp::routes(channel_states.fcp))
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

        if feature_flags.observers {
            api_routes = api_routes.merge(api::observers::routes(api_state.clone()));
        } else {
            tracing::info!("Observers disabled via feature flag");
        }

        // Every Session can resolve a primary Sandbox, including one with no compute.
        // This is a core resource now that new Sessions pin specifications; only the
        // optional provider adapters remain deployment-gated.
        api_routes = api_routes.merge(api::sandbox_templates::routes(sandbox_templates_state));
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

        let api_routes = http_layers::decorate_api_routes(api_routes, &auth_state.config);
        // TM-DOS: Global per-IP API rate limiting (applied to API routes only,
        // not /health or /metrics). Set RATE_LIMIT_API_REQUESTS_PER_MINUTE=0 to disable.
        let api_rate_limiter = http_layers::api_rate_limiter(valkey_for_api_rate_limits);
        let api_routes = http_layers::rate_limit(api_routes, api_rate_limiter.as_ref());

        // The authenticated MCP product surface is always mounted. Access is
        // enforced per request by MCP-specific auth, org resolution, policy,
        // and the root-route rate limiter below (TM-MCP-001, TM-MCP-006).
        // Pages that complete a URL mode elicitation started over `/mcp`. They
        // are browser surfaces (cookie session), not MCP surfaces, which is why
        // they authenticate through the ordinary AuthUser extractor rather than
        // the MCP token path.
        let mcp_elicitation_state = api::mcp_elicitation::AppState::new(
            db.clone(),
            encryption.clone(),
            auth_state.clone(),
            auth_config.base_url.clone(),
        );
        let mut root_routes = Router::new()
            .merge(api::mcp_endpoint::routes(mcp_endpoint_state))
            .merge(api::mcp_elicitation::routes(mcp_elicitation_state));

        if let Some(public_routes) = auth_backend.public_routes() {
            root_routes = root_routes.merge(public_routes);
        }

        // TM-DOS: Apply the same per-IP rate limiting to root routes (MCP, OAuth)
        // as to API routes, to prevent brute-force and DoS on unthrottled endpoints.
        let root_routes = http_layers::rate_limit(root_routes, api_rate_limiter.as_ref());

        // Main router
        let mut app = Router::new()
            .route("/health", get(health::endpoint).with_state(health_state))
            .route(
                "/api-doc/openapi.json",
                get(|| async { Json(ApiDoc::openapi()) }),
            )
            .merge(api::agent_discovery::routes(agent_discovery_state))
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
                supervisor.track(
                    "prometheus_metrics_server",
                    api::prometheus::spawn_metrics_server(handle.clone(), addr.clone()),
                );
            } else if prometheus_config.public_on_main {
                app = app.merge(api::prometheus::route(handle.clone()));
            } else {
                tracing::info!(
                    "Prometheus metrics HTTP endpoint not exposed on main API server; set METRICS_ADDR or METRICS_PUBLIC_ON_MAIN=true"
                );
            }
        }

        let app = http_layers::apply_outer_layers(
            app,
            &self.config.cors_origins,
            &feature_flags,
            prometheus_handle.is_some(),
        );

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
            host_composition: host_composition.clone(),
            email_sender: email_sender.clone(),
            egress_service: host_composition.egress_service(),
            utility_llm_service: host_composition.utility_llm_service(),
            error_reporter: error_reporter.clone(),
        };

        let worker_link = background::WorkerLinkDeps {
            db: db.clone(),
            encryption: encryption.clone(),
            event_service: event_service.clone(),
            runner: runner.clone(),
            host_composition: host_composition.clone(),
            connector_registry: connector_registry.clone(),
            provider_resolver: provider_resolver.clone(),
            permission_resolver: auth_state.permission_resolver.clone(),
            sqldb_store: sqldb_store.clone(),
            org_rate_limiter: org_rate_limiter.clone(),
            virtual_registry: virtual_registry.clone(),
            slack_provisioner: slack_provisioner.clone(),
        };
        if !self.config.dev_mode {
            background::spawn_grpc_server(
                &mut supervisor,
                &self.config.grpc_addr,
                worker_link,
                task_broadcaster.clone(),
            )?;

            // -- Stale task reclamation (everruns_durable::maintenance) --
            crate::durable_reaper::spawn_stale_task_reaper(
                &mut supervisor,
                db.pool().clone(),
                Arc::new(crate::durable_reaper::TurnReapHandler::new(
                    event_service.clone(),
                    reclaim_session_service.clone(),
                    error_reporter.clone(),
                )),
            );

            background::spawn_model_sync(
                &mut supervisor,
                db.clone(),
                driver_registry.clone(),
                encryption.clone(),
            );
        } else if let Some(shared_store) = shared_durable_store {
            background::spawn_dev_task_worker(
                &mut supervisor,
                shared_store,
                worker_link,
                background::DevWorkerExtras {
                    budget_service: budget_service.clone(),
                    durable_store: durable_store.clone(),
                    connection_resolver: optional_connection_resolver(
                        &db,
                        &encryption,
                        &auth_config,
                        host_composition.egress_service(),
                    ),
                },
            );
        } else {
            tracing::info!("DEV MODE: gRPC server disabled, no task worker available");
        }

        // -- Slack delivery recovery (re-register active Slack sessions after restart) --
        {
            let dispatcher = slack_dispatcher.clone();
            let recovery_enc = encryption.clone();
            supervisor.track(
                "slack_delivery_recovery",
                tokio::spawn(async move {
                    dispatcher.recover(recovery_enc.as_ref()).await;
                }),
            );
        }

        // -- Durable task scheduler (both prod and dev) --
        let cluster_jobs_store = background_scheduler_store.clone();
        if let Some(store) = background_scheduler_store {
            crate::system_schedules::ensure_worker_schedules(&store).await;
            let scheduler = everruns_durable::DurableScheduler::with_defaults(
                store,
                format!("scheduler-{}", uuid::Uuid::now_v7()),
            );
            let _scheduler_shutdown = scheduler.spawn();
            tracing::info!("Durable task scheduler started");
        }

        health::start(&mut supervisor, background_db.clone(), encryption.clone()).await;

        // MCP event trigger subscriptions are renewed before their refreshBefore.
        supervisor.track(
            "mcp_event_trigger_refresher",
            mcp_event_triggers.spawn_refresher(),
        );
        background::start_maintenance(
            &mut supervisor,
            background::MaintenanceDeps {
                background_db: background_db.clone(),
                background_pool: db.background_pool().clone(),
                background_runner,
                background_event_service,
                background_session_schedule_service,
                event_delivery: event_delivery.clone(),
                // GitHub connection resolver for the Memory and knowledge index source syncs.
                connection_resolver: optional_connection_resolver(
                    &db,
                    &encryption,
                    &auth_config,
                    host_composition.egress_service(),
                ),
                provider_resolver: provider_resolver.clone(),
                driver_registry: driver_registry.clone(),
                host_composition: host_composition.clone(),
            },
            cluster_jobs_store,
        )
        .await;

        // -- Custom background tasks --
        spawn_background_tasks(&mut supervisor, &server_context, self.background_tasks);

        // =====================================================================
        // Phase 8: Start HTTP server
        // =====================================================================
        serve::serve(&self.config.addr, app).await
    }
}

#[cfg(test)]
mod tests;

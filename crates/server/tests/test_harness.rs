//! Test harness for in-process server testing with PostgreSQL
//!
//! This module provides a TestServer that allows running integration tests
//! against the full API without starting a TCP listener. Uses tower's
//! `oneshot` method for making requests directly to the router.
//!
//! Usage:
//! ```ignore
//! let server = TestServer::new().await;
//! let response = server.post("/v1/agents", json!({"name": "Test"})).await;
//! assert_eq!(response.status(), 201);
//! ```

// Allow unused code - this is a test utility module and not all
// functions/methods are used by all tests
#![allow(dead_code)]

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode},
    routing::get,
};
use http_body_util::BodyExt;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;

use everruns_durable::{
    InMemoryWorkflowEventStore, PostgresWorkflowEventStore, WorkflowEventStore,
};
use everruns_server::{
    api, auth, seed, services,
    storage::{EncryptionService, StorageBackend},
};
use everruns_worker::{AgentRunner, RunnerBackend, create_runner_with_backend};

pub fn extract_cookie(headers: &HeaderMap, name: &str) -> String {
    headers
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|cookie| {
            let prefix = format!("{name}=");
            cookie
                .strip_prefix(&prefix)
                .and_then(|rest| rest.split(';').next())
                .map(|value| format!("{name}={value}"))
        })
        .unwrap_or_else(|| panic!("{name} cookie must be set"))
}

async fn lookup_built_in_harness(db: &Arc<StorageBackend>, name: &str) -> String {
    use everruns_core::DEFAULT_ORG_ID;
    db.get_harness_by_name(DEFAULT_ORG_ID, name)
        .await
        .expect("db error looking up built-in harness")
        .filter(|h| h.is_built_in)
        .map(|h| h.id.to_string())
        .unwrap_or_else(|| panic!("built-in harness '{name}' not provisioned for default org"))
}

/// Get test database URL from environment or use default
pub fn get_database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        let port = std::env::var("DB_PORT").unwrap_or_else(|_| "9332".to_string());
        format!("postgres://everruns:everruns@localhost:{port}/everruns_test")
    })
}

/// Create a PostgreSQL pool for tests
pub async fn create_test_pool() -> PgPool {
    let database_url = get_database_url();
    PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to PostgreSQL. Set DATABASE_URL or ensure postgres is running.")
}

/// Test server for in-process API testing
pub struct TestServer {
    router: Router,
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub pool: PgPool,
    pub virtual_registry:
        Arc<everruns_server::domains::session_files::virtual_mount_registry::VirtualMountRegistry>,
    pub runner: Arc<dyn AgentRunner>,
    /// Public ID of the built-in `base` harness for the default org (resolved
    /// at construction time; no hardcoded UUIDs).
    pub seed_base_harness_id: String,
    /// Public ID of the built-in `generic` harness for the default org.
    pub seed_generic_harness_id: String,
    /// Public ID of the built-in `platform-chat` harness for the default org.
    pub seed_chat_harness_id: String,
}

impl TestServer {
    /// Create a new test server with PostgreSQL backend
    pub async fn new() -> Self {
        Self::with_mode_and_url(TestMode::Postgres, "http://127.0.0.1:0/api".to_string()).await
    }

    /// Create a new test server in dev mode (in-memory storage)
    pub async fn in_memory() -> Self {
        Self::with_mode_and_url(TestMode::InMemory, "http://127.0.0.1:0/api".to_string()).await
    }

    /// In-memory test server with a shrunk ATIF export size cap, so 413
    /// coverage does not need to build a 50 MiB document.
    pub async fn in_memory_with_atif_export_cap(max_bytes: usize) -> Self {
        Self::build(
            TestMode::InMemory,
            "http://127.0.0.1:0/api".to_string(),
            Some(max_bytes),
        )
        .await
    }

    /// Create a test server backed by a real TCP listener.
    ///
    /// Returns `(server, base_url)`. The server is served on a random port
    /// and the MCP endpoint's `execute` tool can make HTTP callbacks to it.
    pub async fn serving() -> (Self, String) {
        Self::serving_with_mode(TestMode::Postgres).await
    }

    /// Create an in-memory test server backed by a real TCP listener.
    ///
    /// Returns `(server, base_url)`. Use this when the code under test needs
    /// to make real HTTP callbacks but does not need PostgreSQL-backed storage.
    pub async fn serving_in_memory() -> (Self, String) {
        Self::serving_with_mode(TestMode::InMemory).await
    }

    async fn serving_with_mode(mode: TestMode) -> (Self, String) {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind TCP listener");
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");

        let server = Self::with_mode_and_url(mode, format!("{base_url}/api")).await;
        let router = server.router.clone();

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async { drop(shutdown_rx.await) })
                .await
                .unwrap();
        });

        // Store shutdown handle so caller can drop it; stash in a leaked box
        // so the server task is cleaned up when the process exits.
        // Also wait briefly for the server to start accepting connections.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Leak the handles — tests are short-lived processes and this avoids
        // adding lifetime complexity to TestServer. The graceful shutdown +
        // oneshot still ensures the server stops promptly on drop.
        std::mem::forget((shutdown_tx, handle));

        (server, base_url)
    }

    fn normalize_uri(uri: &str) -> String {
        if uri.starts_with("/v1/") {
            format!("/api{uri}")
        } else {
            uri.to_string()
        }
    }

    async fn with_mode_and_url(mode: TestMode, api_base_url: String) -> Self {
        Self::build(mode, api_base_url, None).await
    }

    async fn build(
        mode: TestMode,
        api_base_url: String,
        atif_export_max_bytes: Option<usize>,
    ) -> Self {
        // Create storage backend based on mode
        let (db, pool, durable_store) = match mode {
            TestMode::Postgres => {
                let pool = create_test_pool().await;
                let db = Arc::new(StorageBackend::Postgres(
                    everruns_server::storage::Database::new(pool.clone()),
                ));
                let durable_store: Arc<dyn WorkflowEventStore + Send + Sync> =
                    Arc::new(PostgresWorkflowEventStore::new(pool.clone()));
                (db, pool, durable_store)
            }
            TestMode::InMemory => {
                let db = Arc::new(StorageBackend::in_memory());
                let shared_store = Arc::new(InMemoryWorkflowEventStore::new());
                // In-memory tests do not use PostgreSQL; keep a lazy pool only
                // to satisfy the test harness return type.
                let pool = PgPoolOptions::new()
                    .connect_lazy(&get_database_url())
                    .expect("Failed to create lazy PostgreSQL pool");
                (
                    db,
                    pool,
                    shared_store as Arc<dyn WorkflowEventStore + Send + Sync>,
                )
            }
        };

        // Seed default data synchronously (harnesses, agents, providers, etc.)
        let grade = everruns_core::DeploymentGrade::from_env();
        let host_composition = Arc::new(everruns_server::oss_host_composition_for_grade(grade));
        let built_in_harnesses = Arc::new(everruns_server::oss_built_in_harnesses());
        seed::seed_all(&db, grade, &seed::SeedAuthContext::default())
            .await
            .expect("Failed to seed test data");

        // Initialize encryption service (use test key)
        let encryption = Some(Arc::new(
            EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[])
                .expect("Invalid test encryption key"),
        ));

        // Create auth config and backend (no auth for tests)
        let auth_config = auth::AuthConfig {
            base_url: api_base_url.clone(),
            frontend_url: api_base_url
                .trim_end_matches("/api")
                .trim_end_matches('/')
                .to_string(),
            ..auth::AuthConfig::default()
        };
        let auth_backend = auth::BuiltinAuthBackend::new(
            auth_config.clone(),
            db.clone(),
            host_composition.clone(),
        );
        let auth_state = auth::AuthState::new(auth_config.clone(), Arc::new(auth_backend.clone()))
            .with_db(db.clone());

        // Create runner with PostgreSQL backend
        let runner = match mode {
            TestMode::Postgres => create_runner_with_backend(RunnerBackend::Postgres(pool.clone()))
                .await
                .expect("Failed to create agent runner"),
            TestMode::InMemory => {
                // For in-memory tests, use in-memory runner
                let shared_store = Arc::new(InMemoryWorkflowEventStore::new());
                create_runner_with_backend(RunnerBackend::SharedInMemory(shared_store))
                    .await
                    .expect("Failed to create agent runner")
            }
        };

        // Create driver registry
        let driver_registry = Arc::new(host_composition.driver_registry().clone());

        // Shared event delivery — mirrors production wiring where all services
        // publish to the same broadcast backend that SSE subscribers listen on.
        let event_delivery = everruns_server::EventDelivery::in_memory();

        // Create event listeners (minimal for tests)
        let event_service = Arc::new(services::EventService::with_listeners(
            db.clone(),
            event_delivery.clone(),
            vec![],
        ));
        let mut feature_flags = everruns_platform::FeatureFlags::from_env(&grade);
        feature_flags.evals = true;
        feature_flags.observers = true;
        // System side of the experimental gates exercised by integration tests.
        // Org-effective = system && org-opt-in, so both must be on (see the
        // org opt-in seeded just below).
        feature_flags.voice = true;
        feature_flags.agent_versions = true;
        feature_flags.app_budgets = true;
        feature_flags.skills = true;
        feature_flags.memory = true;
        feature_flags.knowledge = true;
        feature_flags.plugins = true;
        feature_flags.agent_delegation = true;

        // Org-effective flags are `system && org-opt-in`, so opt the default
        // test org into the experimental flags whose runtime gates now consult
        // the org-effective flags used by integration tests. Without this those gates reject requests in tests
        // even though the system flags are on. Deliberately scoped to these flags —
        // e.g. `notifications` is left off so tests asserting its default-off
        // org state still hold.
        let org_flag_overrides: std::collections::HashMap<String, bool> = [
            "evals",
            "skills",
            "memory",
            "knowledge",
            "plugins",
            "observers",
            "voice",
            "agent_delegation",
            "agent_versions",
            "app_budgets",
        ]
        .into_iter()
        .map(|name| (name.to_string(), true))
        .collect();
        db.replace_org_feature_flags(everruns_core::DEFAULT_ORG_ID, &org_flag_overrides)
            .await
            .expect("seed org feature flags for test org");

        // The auth middleware resolves each request's org-effective flags as
        // `system && org-opt-in`, taking the system side from
        // `AuthState::system_feature_flags` (defaults to `FeatureFlags::current()`).
        // Point it at the harness's system flags so the gates above see them on.
        let auth_state = auth_state.with_system_feature_flags(feature_flags.clone());

        // Create module-specific states
        let sessions_state = api::sessions::AppState::with_host_composition(
            db.clone(),
            runner.clone(),
            auth_state.clone(),
            &host_composition,
            &built_in_harnesses,
            event_delivery.clone(),
        );
        let mut messages_state = api::messages::AppState::new(
            db.clone(),
            runner.clone(),
            auth_state.clone(),
            feature_flags.notifications,
            event_delivery.clone(),
        );
        if let Some(max_bytes) = atif_export_max_bytes {
            messages_state = messages_state.with_atif_export_max_bytes(max_bytes);
        }
        let sse_tracker = Arc::new(everruns_server::api::sse::SseConnectionTracker::new(
            everruns_server::api::sse::SseConnectionLimits::default(),
        ));
        let events_state = api::events::AppState {
            db: db.clone(),
            session_service: Arc::new(
                everruns_server::domains::sessions::SessionService::with_registry(
                    db.clone(),
                    host_composition.capability_registry().clone(),
                ),
            ),
            event_service: event_service.clone(),
            sse_tracker: sse_tracker.clone(),
            event_broadcaster: None,
            auth: auth_state.clone(),
        };
        let providers_state = api::providers::AppState::new(
            db.clone(),
            encryption.clone(),
            driver_registry.clone(),
            auth_state.clone(),
            None,
        );
        let models_state = api::models::AppState::new(db.clone(), auth_state.clone(), None);
        let knowledge_indexes_state = api::knowledge_indexes::AppState::new(
            db.clone(),
            auth_state.clone(),
            driver_registry.clone(),
        );
        let knowledge_bases_state =
            api::knowledge_bases::AppState::new(db.clone(), auth_state.clone());
        let memory_state = api::memory::AppState::new(db.clone(), auth_state.clone());
        let memory_files_state = api::memory_files::AppState::new(db.clone(), auth_state.clone());
        let virtual_registry = Arc::new(
            everruns_server::domains::session_files::virtual_mount_registry::VirtualMountRegistry::new(),
        );
        // Session SQL database store (in-memory for all test modes)
        let sqldb_backend = Arc::new(everruns_session_sqldb::InMemorySqlDbBackend::new());
        let sqldb_store: Arc<dyn everruns_platform::session_sqldb::SessionSqlDbStore> = Arc::new(
            everruns_session_sqldb::InMemorySqlDbStore::new(sqldb_backend),
        );
        let provider_resolver = Arc::new(services::ProviderResolverService::new(
            db.clone(),
            encryption.clone(),
        ));
        let capability_service = Arc::new(services::CapabilityService::with_registry(
            db.clone(),
            encryption.clone(),
            host_composition.capability_registry().clone(),
        ));
        let mcp_service = Arc::new(
            everruns_server::domains::mcp_servers::McpServerService::new(
                db.clone(),
                encryption.clone(),
            ),
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
        let harnesses_state = api::harnesses::AppState::new(
            db.clone(),
            capability_service.clone(),
            auth_state.clone(),
            grade,
            host_composition.clone(),
        );
        let commands_state = api::commands::AppState::new(
            capability_service.clone(),
            Arc::new(
                everruns_server::domains::session_commands::SessionCommandService::new(
                    db.clone(),
                    event_service.clone(),
                    provider_resolver,
                    mcp_service.clone(),
                    host_composition.capability_registry().clone(),
                    driver_registry.as_ref().clone(),
                    sqldb_store.clone(),
                )
                .with_virtual_registry(virtual_registry.clone()),
            ),
            auth_state.clone(),
        );
        let agents_state = api::agents::AppState::new(
            db.clone(),
            capability_service.clone(),
            auth_state.clone(),
            grade,
            host_composition.clone(),
            built_in_harnesses.clone(),
        );
        let session_files_state = api::session_files::AppState::new(
            db.clone(),
            event_service.clone(),
            auth_state.clone(),
        );
        let workspaces_state = api::workspaces::AppState::new(db.clone(), auth_state.clone());
        let workspace_files_state =
            api::workspace_files::AppState::new(db.clone(), auth_state.clone());
        let session_git_state = api::session_git::AppState::new(db.clone(), auth_state.clone());
        let session_storage_state =
            api::session_storage::AppState::new(db.clone(), encryption.clone(), auth_state.clone());
        let agent_credentials_state = api::agent_credentials::AppState::new(
            db.clone(),
            encryption.clone(),
            auth_state.clone(),
        );
        let session_databases_state = api::session_databases::AppState::new(
            sqldb_store.clone(),
            db.clone(),
            auth_state.clone(),
        );
        let users_state = api::users::UsersState {
            db: db.clone(),
            auth: auth_state.clone(),
        };
        let evals_state = api::evals::AppState::new(db.clone(), auth_state.clone());
        let durable_state = api::durable::AppState::new(
            Some(durable_store.clone()),
            auth_state.clone(),
            None,
            "in_memory".to_string(),
        );
        let schedules_state = api::schedules::ScheduleAppState::new(
            db.clone(),
            Some(durable_store.clone()),
            auth_state.clone(),
        );
        let skills_state =
            api::skills::AppState::new(db.clone(), capability_service.clone(), auth_state.clone());
        let plugins_state =
            api::plugins::AppState::new(db.clone(), capability_service.clone(), auth_state.clone());
        let images_state = api::images::AppState::new(db.clone(), auth_state.clone());
        let organizations_state = api::organizations::AppState::with_harnesses(
            db.clone(),
            auth_state.clone(),
            built_in_harnesses.as_ref().clone(),
        );
        let org_invitations_state = api::org_invitations::AppState::new(
            db.clone(),
            auth_state.clone(),
            everruns_server::platform::system_email_sender(),
            auth_config.frontend_url.clone(),
        );
        let session_schedules_state = api::session_schedules::AppState::new(
            Arc::new(
                everruns_server::domains::session_schedules::SessionScheduleService::new(
                    db.clone(),
                ),
            ),
            auth_state.clone(),
        );
        let notifications_state = if feature_flags.notifications {
            Some(api::notifications::AppState {
                db: db.clone(),
                notification_service: Arc::new(
                    everruns_server::domains::notifications::NotificationService::new(db.clone()),
                ),
                sse_tracker: sse_tracker.clone(),
                notification_broadcaster: None,
                auth: auth_state.clone(),
            })
        } else {
            None
        };
        let feature_flags_state = api::feature_flags::AppState {
            flags: feature_flags.clone(),
        };
        let org_feature_flags_state = api::org_feature_flags::AppState::new(
            db.clone(),
            auth_state.clone(),
            feature_flags.clone(),
        );
        let user_connections_state = api::user_connections::AppState::new(
            db.clone(),
            encryption.clone(),
            auth_state.clone(),
            auth_config.clone(),
            everruns_server::platform::oss_connector_registry(),
            mcp_service,
        );
        let reporting_state = api::reporting::AppState::new(db.clone(), auth_state.clone());

        let agent_identities_state = api::agent_identities::AppState::new(
            db.clone(),
            capability_service.clone(),
            auth_state.clone(),
        );
        let agent_identity_connections_state = api::agent_identity_connections::AppState::new(
            db.clone(),
            encryption.clone(),
            auth_state.clone(),
            everruns_server::platform::oss_connector_registry(),
        );
        let apps_state = api::apps::AppState::new(
            db.clone(),
            encryption.clone(),
            Some(durable_store.clone()),
            capability_service.clone(),
            auth_state.clone(),
        )
        .with_invocation_services(
            messages_state.session_service.clone(),
            messages_state.message_service.clone(),
        );
        let agent_triggers_state = api::agent_triggers::AppState::new(
            db.clone(),
            Some(durable_store.clone()),
            capability_service.clone(),
            auth_state.clone(),
            messages_state.session_service.clone(),
            messages_state.message_service.clone(),
        );
        let slack_state = api::slack_events::SlackState::new(
            db.clone(),
            encryption.clone(),
            runner.clone(),
            None, // No delivery dispatcher in tests
            feature_flags.notifications,
            event_delivery.clone(),
        );
        let app_webhooks_state = api::app_webhooks::AppWebhookState::new(
            db.clone(),
            encryption.clone(),
            runner.clone(),
            feature_flags.notifications,
            event_delivery.clone(),
            api::channel_rate_limit::ChannelRateLimiter::in_memory("webhook"),
        );
        let app_a2a_state = api::app_a2a::AppA2aState::new(
            db.clone(),
            encryption.clone(),
            runner.clone(),
            feature_flags.notifications,
            event_delivery.clone(),
            sse_tracker.clone(),
            api::channel_rate_limit::ChannelRateLimiter::in_memory("a2a"),
            api::a2a_signing::A2aReplayStore::in_memory(),
        );
        let app_api_state = api::app_api::AppApiState::new(
            db.clone(),
            encryption.clone(),
            runner.clone(),
            feature_flags.notifications,
            event_delivery.clone(),
            api::channel_rate_limit::ChannelRateLimiter::in_memory("apikey"),
        );
        let ag_ui_state = api::ag_ui::AgUiState::new(
            db.clone(),
            encryption.clone(),
            runner.clone(),
            feature_flags.notifications,
            event_delivery.clone(),
            sse_tracker.clone(),
            api::channel_rate_limit::ChannelRateLimiter::in_memory("agui"),
        );
        let fcp_state = api::fcp::FcpState::new(
            db.clone(),
            encryption.clone(),
            runner.clone(),
            feature_flags.notifications,
            event_delivery.clone(),
            api::channel_rate_limit::ChannelRateLimiter::in_memory("fcp"),
        );
        let mcp_endpoint_state = api::mcp_endpoint::AppState::new(
            db.clone(),
            runner.clone(),
            auth_state.clone(),
            &host_composition,
            &built_in_harnesses,
            feature_flags.notifications,
            event_delivery.clone(),
            encryption.clone(),
            Some(durable_store.clone()),
            capability_service.clone(),
            Some(sqldb_store.clone()),
        )
        .with_virtual_registry(virtual_registry.clone());

        // Build API routes
        let mut api_routes = Router::new()
            .merge(api::agents::routes(agents_state))
            .merge(api::agent_credentials::routes(agent_credentials_state))
            .merge(api::agent_identities::routes(agent_identities_state))
            .merge(api::agent_identity_connections::routes(
                agent_identity_connections_state,
            ))
            .merge(api::apps::routes(apps_state))
            .merge(api::agent_triggers::routes(agent_triggers_state))
            .merge(api::harnesses::routes(harnesses_state))
            .merge(api::sessions::routes(sessions_state))
            .merge(api::messages::routes(messages_state))
            .merge(api::events::routes(events_state))
            .merge(api::models::routes(models_state))
            .merge(api::knowledge_indexes::routes(knowledge_indexes_state))
            .merge(api::knowledge_bases::routes(knowledge_bases_state))
            .merge(api::memory::routes(memory_state))
            .merge(api::memory_files::routes(memory_files_state))
            .merge(api::providers::routes(providers_state))
            .merge(api::mcp_servers::routes(mcp_servers_state))
            .merge(api::capabilities::routes(capabilities_state))
            .merge(api::commands::routes(commands_state))
            .merge(api::session_files::routes(
                session_files_state.with_virtual_registry(virtual_registry.clone()),
            ))
            .merge(api::workspaces::routes(workspaces_state))
            .merge(api::workspace_files::routes(
                workspace_files_state.with_virtual_registry(virtual_registry.clone()),
            ))
            .merge(api::session_git::routes(session_git_state))
            .merge(api::session_storage::routes(session_storage_state))
            .merge(api::session_databases::routes(session_databases_state))
            .merge(api::users::routes(users_state))
            .merge(api::durable::routes(durable_state))
            .merge(api::schedules::routes(schedules_state))
            .merge(api::skills::routes(skills_state))
            .merge(api::plugins::routes(plugins_state))
            .merge(api::images::routes(images_state))
            .merge(api::organizations::routes(organizations_state))
            .merge(api::org_invitations::routes(org_invitations_state))
            .merge(api::session_schedules::routes(session_schedules_state))
            .merge(api::feature_flags::routes(feature_flags_state))
            .merge(api::org_feature_flags::routes(org_feature_flags_state))
            .merge(api::user_connections::routes(user_connections_state))
            .merge(api::reporting::routes(reporting_state))
            .merge(api::ag_ui::routes(ag_ui_state))
            .merge(api::fcp::routes(fcp_state))
            .merge(api::slack_events::routes(slack_state))
            .merge(api::app_webhooks::routes(app_webhooks_state))
            .merge(api::app_a2a::routes(app_a2a_state))
            .merge(api::app_api::routes(app_api_state))
            .merge(auth::routes(auth_backend.clone()))
            .merge(auth::cli_auth::cli_auth_routes(
                auth::cli_auth::CliAuthState {
                    db: db.clone(),
                    auth: auth_state.clone(),
                    frontend_url: auth_config.frontend_url.clone(),
                    login_origin: auth_config.login_origin.clone(),
                    base_url: auth_config.base_url.clone(),
                },
            ));

        if let Some(notifications_state) = notifications_state {
            api_routes = api_routes.merge(api::notifications::routes(notifications_state));
        }
        if feature_flags.evals {
            api_routes = api_routes.merge(api::evals::routes(evals_state));
        }
        if feature_flags.observers {
            let observers_state = api::observers::AppState::new(db.clone(), auth_state.clone());
            api_routes = api_routes.merge(api::observers::routes(observers_state));
        }

        api_routes = api_routes.merge(auth::personal_access_token_routes(
            auth::PersonalAccessTokenState {
                db: db.clone(),
                auth: auth_state.clone(),
                resource_limits: everruns_server::server::ResourceLimitsConfig::default(),
            },
        ));

        let root_routes = Router::new()
            .merge(api::mcp_endpoint::routes(mcp_endpoint_state))
            .merge(auth::cli_auth::cli_auth_public_routes(
                auth::cli_auth::CliAuthState {
                    db: db.clone(),
                    auth: auth_state.clone(),
                    frontend_url: auth_config.frontend_url.clone(),
                    login_origin: auth_config.login_origin.clone(),
                    base_url: auth_config.base_url.clone(),
                },
            ))
            .merge(auth::mcp_oauth::mcp_oauth_routes(
                auth::mcp_oauth::McpOAuthState {
                    db: db.clone(),
                    auth: auth_state.clone(),
                    jwt_service: auth_backend.jwt_service.clone(),
                    issuer_url: auth_config.frontend_url.clone(),
                    frontend_url: auth_config.frontend_url.clone(),
                    login_origin: auth_config.login_origin.clone(),
                    rate_limiter: auth_backend.rate_limiter.clone(),
                },
            ));

        // Build main router with health endpoint
        let router = Router::new()
            .route(
                "/health",
                get(|| async { axum::Json(serde_json::json!({"status": "ok"})) }),
            )
            .merge(root_routes)
            .nest("/api", api_routes);

        let seed_base_harness_id = lookup_built_in_harness(&db, "base").await;
        let seed_generic_harness_id = lookup_built_in_harness(&db, "generic").await;
        let seed_chat_harness_id = lookup_built_in_harness(&db, "platform-chat").await;

        Self {
            router,
            db,
            encryption,
            pool,
            virtual_registry,
            runner,
            seed_base_harness_id,
            seed_generic_harness_id,
            seed_chat_harness_id,
        }
    }

    /// Make a GET request
    pub async fn get(&self, uri: &str) -> TestResponse {
        self.request(Method::GET, uri, None::<()>).await
    }

    /// Make a POST request with JSON body
    pub async fn post<T: Serialize>(&self, uri: &str, body: T) -> TestResponse {
        self.request(Method::POST, uri, Some(body)).await
    }

    /// Make a PUT request with JSON body
    pub async fn put<T: Serialize>(&self, uri: &str, body: T) -> TestResponse {
        self.request(Method::PUT, uri, Some(body)).await
    }

    /// Make a PATCH request with JSON body
    pub async fn patch<T: Serialize>(&self, uri: &str, body: T) -> TestResponse {
        self.request(Method::PATCH, uri, Some(body)).await
    }

    /// Make a DELETE request
    pub async fn delete(&self, uri: &str) -> TestResponse {
        self.request(Method::DELETE, uri, None::<()>).await
    }

    /// Make a raw request with custom headers and body bytes (for Slack webhook testing)
    pub async fn request_raw(
        &self,
        method: Method,
        uri: &str,
        headers: Vec<(&str, &str)>,
        body: Vec<u8>,
    ) -> TestResponse {
        let normalized_uri = Self::normalize_uri(uri);
        let mut builder = Request::builder().method(method).uri(&normalized_uri);
        for (key, value) in headers {
            builder = builder.header(key, value);
        }
        let request = builder
            .body(Body::from(body))
            .expect("Failed to build request");

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("Request failed");

        let status = response.status();
        let headers = response.headers().clone();
        let body_bytes = response
            .into_body()
            .collect()
            .await
            .expect("Failed to read body")
            .to_bytes();

        TestResponse {
            status,
            headers,
            body: body_bytes.to_vec(),
        }
    }

    /// Make a raw request and return once the response headers are available,
    /// without collecting a streaming response body.
    pub async fn request_raw_without_collecting_body(
        &self,
        method: Method,
        uri: &str,
        headers: Vec<(&str, &str)>,
        body: Vec<u8>,
    ) -> TestResponse {
        let normalized_uri = Self::normalize_uri(uri);
        let mut builder = Request::builder().method(method).uri(&normalized_uri);
        for (key, value) in headers {
            builder = builder.header(key, value);
        }
        let request = builder
            .body(Body::from(body))
            .expect("Failed to build request");

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("Request failed");

        TestResponse {
            status: response.status(),
            headers: response.headers().clone(),
            body: Vec::new(),
        }
    }

    /// Make a request with custom method and optional body
    async fn request<T: Serialize>(
        &self,
        method: Method,
        uri: &str,
        body: Option<T>,
    ) -> TestResponse {
        let normalized_uri = Self::normalize_uri(uri);
        let request_builder = Request::builder()
            .method(method)
            .uri(&normalized_uri)
            .header("content-type", "application/json");

        let request = if let Some(body) = body {
            let json = serde_json::to_string(&body).expect("Failed to serialize body");
            request_builder
                .body(Body::from(json))
                .expect("Failed to build request")
        } else {
            request_builder
                .body(Body::empty())
                .expect("Failed to build request")
        };

        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("Request failed");

        let status = response.status();
        let headers = response.headers().clone();
        let body_bytes = response
            .into_body()
            .collect()
            .await
            .expect("Failed to read body")
            .to_bytes();

        TestResponse {
            status,
            headers,
            body: body_bytes.to_vec(),
        }
    }
}

/// Test mode for the server
#[derive(Clone, Copy)]
pub enum TestMode {
    Postgres,
    InMemory,
}

/// Response from a test request
pub struct TestResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl TestResponse {
    /// Get the status code
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Get the response headers
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get the body as a string
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    /// Get the raw response body bytes
    pub fn bytes(&self) -> &[u8] {
        &self.body
    }

    /// Parse the body as JSON
    pub fn json<T: DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("Failed to parse JSON: {}. Body: {}", e, self.text()))
    }

    /// Parse the body as a JSON Value
    pub fn json_value(&self) -> Value {
        self.json()
    }

    /// Assert status and return self for chaining
    pub fn assert_status(self, expected: StatusCode) -> Self {
        assert_eq!(
            self.status,
            expected,
            "Expected status {}, got {}. Body: {}",
            expected,
            self.status,
            self.text()
        );
        self
    }

    /// Assert success (2xx) status
    pub fn assert_success(self) -> Self {
        assert!(
            self.status.is_success(),
            "Expected success status, got {}. Body: {}",
            self.status,
            self.text()
        );
        self
    }
}

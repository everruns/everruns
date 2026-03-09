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
    http::{Method, Request, StatusCode},
    routing::get,
};
use http_body_util::BodyExt;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

use everruns_durable::{
    InMemoryWorkflowEventStore, PostgresWorkflowEventStore, WorkflowEventStore,
};
use everruns_server::{
    api, auth, seed, services,
    storage::{EncryptionService, StorageBackend},
};
use everruns_worker::{RunnerBackend, create_driver_registry, create_runner_with_backend};

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
    pub pool: PgPool,
}

impl TestServer {
    /// Create a new test server with PostgreSQL backend
    pub async fn new() -> Self {
        Self::with_mode(TestMode::Postgres).await
    }

    /// Create a new test server in dev mode (in-memory storage)
    pub async fn in_memory() -> Self {
        Self::with_mode(TestMode::InMemory).await
    }

    async fn with_mode(mode: TestMode) -> Self {
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
                // For in-memory mode, we still need a pool for the return type
                // but we won't use it - create a dummy connection
                let pool = create_test_pool().await;
                (
                    db,
                    pool,
                    shared_store as Arc<dyn WorkflowEventStore + Send + Sync>,
                )
            }
        };

        // Seed default data synchronously (harnesses, agents, providers, etc.)
        let grade = everruns_core::DeploymentGrade::from_env();
        seed::seed_all(&db, grade)
            .await
            .expect("Failed to seed test data");

        // Initialize encryption service (use test key)
        let encryption = Some(Arc::new(
            EncryptionService::new("kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=", &[])
                .expect("Invalid test encryption key"),
        ));

        // Create auth config and backend (no auth for tests)
        let auth_config = auth::AuthConfig::default(); // mode: None is the default
        let auth_backend = auth::BuiltinAuthBackend::new(auth_config.clone(), db.clone());
        let auth_state = auth::AuthState::new(auth_config.clone(), Arc::new(auth_backend.clone()));

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
        let driver_registry = Arc::new(create_driver_registry());

        // Create event listeners (minimal for tests)
        let event_service = Arc::new(services::EventService::with_listeners(db.clone(), vec![]));

        // Create module-specific states
        let sessions_state =
            api::sessions::AppState::new(db.clone(), runner.clone(), auth_state.clone());
        let messages_state =
            api::messages::AppState::new(db.clone(), runner.clone(), auth_state.clone());
        let sse_tracker = Arc::new(everruns_server::api::sse::SseConnectionTracker::new(
            everruns_server::api::sse::SseConnectionLimits::default(),
        ));
        let events_state = api::events::AppState {
            session_service: Arc::new(services::SessionService::new(db.clone())),
            event_service: event_service.clone(),
            sse_tracker,
            event_broadcaster: None,
            auth: auth_state.clone(),
        };
        let llm_providers_state = api::llm_providers::AppState::new(
            db.clone(),
            encryption.clone(),
            driver_registry.clone(),
            auth_state.clone(),
            None,
        );
        let llm_models_state = api::llm_models::AppState::new(db.clone(), auth_state.clone(), None);
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
        let agents_state =
            api::agents::AppState::new(db.clone(), capability_service, auth_state.clone());
        let session_files_state = api::session_files::AppState::new(db.clone(), auth_state.clone());
        let session_storage_state =
            api::session_storage::AppState::new(db.clone(), auth_state.clone());
        // Session SQL database store (in-memory for all test modes)
        let sqldb_backend = Arc::new(everruns_session_sqldb::InMemorySqlDbBackend::new());
        let sqldb_store: Arc<dyn everruns_core::session_sqldb::SessionSqlDbStore> = Arc::new(
            everruns_session_sqldb::InMemorySqlDbStore::new(sqldb_backend),
        );
        let session_databases_state =
            api::session_databases::AppState::new(sqldb_store, db.clone(), auth_state.clone());
        let users_state = api::users::UsersState {
            db: db.clone(),
            auth: auth_state.clone(),
        };
        let durable_state =
            api::durable::AppState::new(Some(durable_store.clone()), auth_state.clone());
        let schedules_state =
            api::schedules::ScheduleAppState::new(Some(durable_store), auth_state.clone());
        let skills_state = api::skills::AppState::new(db.clone(), auth_state.clone());
        let images_state = api::images::AppState::new(db.clone(), auth_state.clone());
        let organizations_state = api::organizations::AppState::new(db.clone(), auth_state.clone());
        let feature_flags = everruns_core::FeatureFlags::from_env(&grade);
        let feature_flags_state = api::feature_flags::AppState {
            flags: feature_flags,
        };

        // Build API routes
        let api_routes = Router::new()
            .merge(api::agents::routes(agents_state))
            .merge(api::harnesses::routes(harnesses_state))
            .merge(api::sessions::routes(sessions_state))
            .merge(api::messages::routes(messages_state))
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
            .merge(api::skills::routes(skills_state))
            .merge(api::images::routes(images_state))
            .merge(api::organizations::routes(organizations_state))
            .merge(api::feature_flags::routes(feature_flags_state))
            .merge(auth::routes(auth_backend));

        // Build main router with health endpoint
        let router = Router::new()
            .route(
                "/health",
                get(|| async { axum::Json(serde_json::json!({"status": "ok"})) }),
            )
            .merge(api_routes);

        Self { router, db, pool }
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

    /// Make a request with custom method and optional body
    async fn request<T: Serialize>(
        &self,
        method: Method,
        uri: &str,
        body: Option<T>,
    ) -> TestResponse {
        let request_builder = Request::builder()
            .method(method)
            .uri(uri)
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
        let body_bytes = response
            .into_body()
            .collect()
            .await
            .expect("Failed to read body")
            .to_bytes();

        TestResponse {
            status,
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
    body: Vec<u8>,
}

impl TestResponse {
    /// Get the status code
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Get the body as a string
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
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

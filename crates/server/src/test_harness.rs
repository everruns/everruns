// Test harness for integration tests
//
// Decision: In-process server setup for fast, isolated integration tests
// Decision: No external process dependencies - everything runs in-memory
// Decision: Worker runs in background task, spawned automatically
//
// Usage:
//   let harness = TestHarness::new().await;
//   let response = harness.get("/v1/agents").await;

use axum::Router;
use everruns_core::CapabilityRegistry;
use everruns_durable::InMemoryWorkflowEventStore;
use everruns_worker::{TaskWorker, TaskWorkerConfig, create_driver_registry};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::DirectWorkerAdapters;
use crate::api;
use crate::auth::{self, AuthConfig};
use crate::services;
use crate::storage::StorageBackend;

/// Test harness for running integration tests with an in-process server
pub struct TestHarness {
    /// Base URL of the test server (e.g., "http://127.0.0.1:12345")
    pub base_url: String,
    /// HTTP client for making requests
    pub client: reqwest::Client,
    /// Shutdown signal sender - drop to stop the server
    _shutdown_tx: oneshot::Sender<()>,
}

impl TestHarness {
    /// Create a new test harness with an in-process server and worker
    pub async fn new() -> Self {
        let (app, shared_store, db) = create_test_app().await;

        // Spawn the in-process worker
        spawn_test_worker(shared_store, db);

        // Spawn the server and get its URL
        let (base_url, shutdown_tx) = spawn_test_server(app).await;

        Self {
            base_url,
            client: reqwest::Client::new(),
            _shutdown_tx: shutdown_tx,
        }
    }

    /// Make a GET request to the given path
    pub async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{}", self.base_url, path))
            .send()
            .await
            .expect("Failed to send GET request")
    }

    /// Make a POST request with JSON body
    pub async fn post(&self, path: &str, body: &serde_json::Value) -> reqwest::Response {
        self.client
            .post(format!("{}{}", self.base_url, path))
            .json(body)
            .send()
            .await
            .expect("Failed to send POST request")
    }

    /// Make a PUT request with JSON body
    pub async fn put(&self, path: &str, body: &serde_json::Value) -> reqwest::Response {
        self.client
            .put(format!("{}{}", self.base_url, path))
            .json(body)
            .send()
            .await
            .expect("Failed to send PUT request")
    }

    /// Make a PATCH request with JSON body
    pub async fn patch(&self, path: &str, body: &serde_json::Value) -> reqwest::Response {
        self.client
            .patch(format!("{}{}", self.base_url, path))
            .json(body)
            .send()
            .await
            .expect("Failed to send PATCH request")
    }

    /// Make a DELETE request
    pub async fn delete(&self, path: &str) -> reqwest::Response {
        self.client
            .delete(format!("{}{}", self.base_url, path))
            .send()
            .await
            .expect("Failed to send DELETE request")
    }

    /// Get the base URL for the test server
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Get SSE headers for Accept
    pub fn sse_headers() -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            "text/event-stream".parse().unwrap(),
        );
        headers
    }
}

/// Create the test application router with all routes configured
async fn create_test_app() -> (Router, Arc<InMemoryWorkflowEventStore>, Arc<StorageBackend>) {
    // Create in-memory storage
    let db = Arc::new(StorageBackend::in_memory());

    // Create shared in-memory durable store
    let shared_store = Arc::new(InMemoryWorkflowEventStore::new());

    // Create runner with shared store
    let runner = everruns_worker::create_runner_with_backend(
        everruns_worker::RunnerBackend::SharedInMemory(shared_store.clone()),
    )
    .await
    .expect("Failed to create in-memory runner");

    // No encryption service for tests
    let encryption = None;

    // Create auth config with no authentication (default is AuthMode::None)
    let auth_config = AuthConfig::default();
    let auth_state = auth::AuthState::new(auth_config, db.clone());

    // Create module-specific states
    let sessions_state =
        api::sessions::AppState::new(db.clone(), runner.clone(), auth_state.clone());
    let messages_state =
        api::messages::AppState::new(db.clone(), runner.clone(), auth_state.clone());

    // Create event listeners (empty for tests)
    let event_listeners: Vec<Arc<dyn everruns_core::EventListener>> = vec![];
    let event_service = Arc::new(services::EventService::with_listeners(
        db.clone(),
        event_listeners,
    ));

    let events_state = api::events::AppState {
        session_service: Arc::new(services::SessionService::new(db.clone())),
        event_service: event_service.clone(),
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
    let agents_state =
        api::agents::AppState::new(db.clone(), capability_service, auth_state.clone());
    let session_files_state = api::session_files::AppState::new(db.clone(), auth_state.clone());
    let session_storage_state = api::session_storage::AppState::new(db.clone(), auth_state.clone());
    let users_state = api::users::UsersState {
        db: db.clone(),
        auth: auth_state.clone(),
    };

    // Use in-memory durable store
    let durable_store: Option<Arc<dyn everruns_durable::WorkflowEventStore + Send + Sync>> =
        Some(shared_store.clone() as Arc<dyn everruns_durable::WorkflowEventStore + Send + Sync>);
    let durable_state = api::durable::AppState::new(durable_store);
    let images_state = api::images::AppState::new(db.clone(), auth_state.clone());
    let organizations_state = api::organizations::AppState::new(db.clone(), auth_state.clone());

    // Build API routes
    let api_routes = Router::new()
        .merge(api::agents::routes(agents_state))
        .merge(api::sessions::routes(sessions_state))
        .merge(api::messages::routes(messages_state))
        .merge(api::events::routes(events_state))
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

    // Build main router with health endpoint
    let app = Router::new()
        .route("/health", axum::routing::get(health))
        .merge(api_routes);

    (app, shared_store, db)
}

/// Health endpoint for test server
async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "auth_mode": "None"
    }))
}

/// Spawn the test server and return its base URL and shutdown channel
async fn spawn_test_server(app: Router) -> (String, oneshot::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test server");
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}", addr);

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });

    // Give the server a moment to start
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    (url, shutdown_tx)
}

/// Spawn the in-process task worker
fn spawn_test_worker(shared_store: Arc<InMemoryWorkflowEventStore>, db: Arc<StorageBackend>) {
    // Create event service for worker (no listeners for tests)
    let event_service = Arc::new(services::EventService::new(db.clone()));

    // Create LLM resolver service
    let llm_resolver = Arc::new(services::LlmResolverService::new(db.clone(), None));

    // Create MCP server service
    let mcp_server_service = Arc::new(services::McpServerService::new(db.clone(), None));

    // Create capability registry
    let capability_registry = CapabilityRegistry::with_builtins();

    // Create direct adapters
    let adapters = DirectWorkerAdapters::new(
        db,
        event_service,
        llm_resolver,
        mcp_server_service,
        capability_registry,
    );

    // Create and spawn the task worker
    let worker_config = TaskWorkerConfig::dev_mode();
    tokio::spawn(async move {
        let mut worker = TaskWorker::new(worker_config, shared_store, adapters);
        if let Err(e) = worker.run().await {
            tracing::error!("Test worker error: {}", e);
        }
    });
}

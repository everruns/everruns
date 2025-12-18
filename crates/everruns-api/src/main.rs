// Everruns API server
// Decision: Flexible auth with support for no-auth, admin-only, and full auth modes
// M2: Agent/Session/Messages model with Events as SSE notifications

mod agents;
mod auth;
mod capabilities;
mod llm_models;
mod llm_providers;
mod services;
mod sessions;

use anyhow::{Context, Result};
use axum::{extract::State, routing::get, Json, Router};
use everruns_contracts::*;
use everruns_storage::{Database, EncryptionService};
use everruns_worker::{create_runner, RunnerConfig, RunnerMode};
use serde::Serialize;
use std::sync::Arc;
use axum::http::{header, HeaderValue, Method};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
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
    runner_mode: String,
    auth_mode: String,
}

async fn health(State(state): State<HealthState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        runner_mode: state.runner_mode.clone(),
        auth_mode: state.auth_mode.clone(),
    })
}

/// State for health endpoint
#[derive(Clone)]
struct HealthState {
    runner_mode: String,
    auth_mode: String,
}

/// OpenAPI documentation
#[derive(OpenApi)]
#[openapi(
    paths(
        agents::create_agent,
        agents::list_agents,
        agents::get_agent,
        agents::update_agent,
        agents::delete_agent,
        sessions::create_session,
        sessions::list_sessions,
        sessions::get_session,
        sessions::update_session,
        sessions::delete_session,
        sessions::create_message,
        sessions::list_messages,
        sessions::stream_events,
        llm_providers::create_provider,
        llm_providers::list_providers,
        llm_providers::get_provider,
        llm_providers::update_provider,
        llm_providers::delete_provider,
        llm_models::create_model,
        llm_models::list_provider_models,
        llm_models::list_all_models,
        llm_models::get_model,
        llm_models::update_model,
        llm_models::delete_model,
        capabilities::list_capabilities,
        capabilities::get_capability,
        capabilities::get_agent_capabilities,
        capabilities::set_agent_capabilities,
    ),
    components(
        schemas(
            Agent, AgentStatus,
            Session, SessionStatus, Message, MessageRole, Event,
            CreateAgentRequest, UpdateAgentRequest,
            CreateSessionRequest, UpdateSessionRequest,
            CreateMessageRequest,
            ListResponse<Agent>,
            ListResponse<Session>,
            ListResponse<Message>,
            ListResponse<Event>,
            LlmProvider, LlmProviderType, LlmProviderStatus,
            LlmModel, LlmModelWithProvider, LlmModelStatus,
            llm_providers::CreateLlmProviderRequest,
            llm_providers::UpdateLlmProviderRequest,
            llm_models::CreateLlmModelRequest,
            llm_models::UpdateLlmModelRequest,
            Capability, CapabilityId, CapabilityStatus,
            AgentCapability, UpdateAgentCapabilitiesRequest,
            ListResponse<Capability>,
            ListResponse<AgentCapability>,
        )
    ),
    tags(
        (name = "agents", description = "Agent management endpoints"),
        (name = "sessions", description = "Session management endpoints"),
        (name = "messages", description = "Message management endpoints"),
        (name = "events", description = "Event streaming endpoints (SSE)"),
        (name = "llm-providers", description = "LLM Provider management endpoints"),
        (name = "llm-models", description = "LLM Model management endpoints"),
        (name = "capabilities", description = "Capability management endpoints")
    ),
    info(
        title = "Everruns API",
        version = "0.2.0",
        description = "API for managing AI agents, sessions, messages, and events",
        license(name = "MIT", url = "https://opensource.org/licenses/MIT")
    )
)]
struct ApiDoc;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "everruns_api=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

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
    tracing::info!(
        mode = ?runner_config.mode,
        "Agent runner mode configured"
    );

    // Create the agent runner based on configuration
    let runner = create_runner(&runner_config, db.clone())
        .await
        .context("Failed to create agent runner")?;

    match runner_config.mode {
        RunnerMode::InProcess => {
            tracing::info!("Using in-process agent runner (default)");
        }
        RunnerMode::Temporal => {
            tracing::info!(
                address = %runner_config.temporal_address(),
                namespace = %runner_config.temporal_namespace(),
                task_queue = %runner_config.temporal_task_queue(),
                "Using Temporal agent runner"
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
    let agents_state = agents::AppState::new(db.clone());
    let sessions_state = sessions::AppState::new(db.clone(), runner.clone());
    let llm_providers_state = llm_providers::AppState {
        db: db.clone(),
        encryption: encryption.clone(),
    };
    let llm_models_state = llm_models::AppState { db: db.clone() };
    let capabilities_state = capabilities::AppState::new(db.clone());
    let health_state = HealthState {
        runner_mode: format!("{:?}", runner_config.mode),
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
        .map(|s| {
            s.split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect()
        })
        .unwrap_or_default();

    if cors_origins.is_empty() {
        tracing::info!("CORS not configured (same-origin requests only)");
    } else {
        tracing::info!(origins = ?cors_origins, "CORS origins configured");
    }

    // Build API routes
    // Note: llm_models routes must be merged BEFORE llm_providers
    // because /v1/llm-providers/{provider_id}/models is more specific
    // than /v1/llm-providers/{id}
    let api_routes = Router::new()
        .merge(agents::routes(agents_state))
        .merge(sessions::routes(sessions_state))
        .merge(llm_models::routes(llm_models_state))
        .merge(llm_providers::routes(llm_providers_state))
        .merge(capabilities::routes(capabilities_state));

    // Build main router with health, auth (not prefixed), and prefixed API routes
    let mut app = Router::new()
        .route("/health", get(health).with_state(health_state))
        .merge(auth::routes(auth_state));

    // Apply API prefix if configured
    app = app.merge(build_router_with_prefix(api_routes, &api_prefix));

    // Add Swagger UI
    let app = app.merge(SwaggerUi::new("/swagger-ui").url("/api-doc/openapi.json", ApiDoc::openapi()));

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

    // Start server
    let addr = "0.0.0.0:9000";
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("Failed to bind to address")?;
    tracing::info!("Listening on {}", addr);

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

// Everruns API server
// Decision: Auth will be added later via OAuth (dashboard login)

mod agents;
mod agui;
mod runs;
mod threads;

use anyhow::{Context, Result};
use axum::{routing::get, Json, Router};
use everruns_contracts::*;
use everruns_storage::Database;
use everruns_worker::WorkflowExecutor;
use serde::Serialize;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
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
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// OpenAPI documentation
#[derive(OpenApi)]
#[openapi(
    paths(
        agents::create_agent,
        agents::list_agents,
        agents::get_agent,
        agents::update_agent,
        threads::create_thread,
        threads::get_thread,
        threads::create_message,
        threads::list_messages,
        runs::list_runs,
        runs::create_run,
        runs::get_run,
        runs::cancel_run,
    ),
    components(
        schemas(
            Agent, AgentStatus,
            Thread, Message,
            Run, RunStatus,
            Action, ActionKind,
            User,
            agents::CreateAgentRequest,
            agents::UpdateAgentRequest,
            threads::CreateThreadRequest,
            threads::CreateMessageRequest,
            runs::CreateRunRequest,
            runs::ListRunsParams,
        )
    ),
    tags(
        (name = "agents", description = "Agent management endpoints"),
        (name = "threads", description = "Thread and message management endpoints"),
        (name = "runs", description = "Run execution endpoints")
    ),
    info(
        title = "Everruns API",
        version = "0.1.0",
        description = "API for managing AI agents, threads, and runs",
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

    // Create workflow executor (M4: in-process execution)
    let executor = Arc::new(WorkflowExecutor::new(db.clone()));
    tracing::info!("Workflow executor initialized");

    // Create app state
    let state = AppState { db: Arc::new(db) };

    // Create module-specific states
    let agents_state = agents::AppState {
        db: state.db.clone(),
    };
    let threads_state = threads::AppState {
        db: state.db.clone(),
    };
    let runs_state = runs::AppState {
        db: state.db.clone(),
        executor: executor.clone(),
    };
    let agui_state = agui::AppState {
        db: state.db.clone(),
        executor: executor.clone(),
    };

    // Build router
    let app = Router::new()
        .route("/health", get(health))
        .merge(agents::routes(agents_state))
        .merge(threads::routes(threads_state))
        .merge(runs::routes(runs_state))
        .merge(agui::routes(agui_state))
        .merge(SwaggerUi::new("/swagger-ui").url("/api-doc/openapi.json", ApiDoc::openapi()))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http());

    // Start server
    let addr = "0.0.0.0:9000";
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("Failed to bind to address")?;
    tracing::info!("Listening on {}", addr);

    axum::serve(listener, app).await.context("Server error")?;

    Ok(())
}

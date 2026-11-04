// Agent Examples API — read-only catalogue of built-in examples.
//
// Decision: Examples live in code (SEED_AGENTS), not in DB.
// Decision: Import is handled by POST /v1/agents/import?from-example={name}
// Decision: Examples are identified by their name
// Decision: Guided templates (crate::agent_templates) are examples with a
// `setup` the UI walks through after import; same list, same import path.
// Decision: the catalogue, its filter and adoption live in
// `domains::agents::examples` as commands, so the CLI and MCP see the same set.

use crate::auth::{AuthState, ResolvedOrg};
pub use crate::domains::agents::examples::{AgentExample, AgentExampleSetting, AgentExampleSetup};
use axum::{Json, Router, extract::State, routing::get};
use everruns_core::DeploymentGrade;
use everruns_core::host::HostComposition;
use std::sync::Arc;

use super::common::impl_auth_state;

/// App state for agent example routes
#[derive(Clone)]
pub struct AppState {
    pub auth: AuthState,
    pub grade: DeploymentGrade,
    pub host_composition: Arc<HostComposition>,
}

impl_auth_state!(AppState);

/// Create agent example routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/agent-examples", get(list_examples))
        .with_state(state)
}

/// GET /v1/agent-examples — list all available examples
#[utoipa::path(
    operation_id = "list_agent_examples",
    get,
    path = "/v1/agent-examples",
    responses(
        (status = 200, description = "List of agent examples", body = Vec<AgentExample>),
    ),
    tag = "agent-examples"
)]
pub async fn list_examples(
    _org: ResolvedOrg,
    State(state): State<AppState>,
) -> Json<Vec<AgentExample>> {
    let registry = state.host_composition.capability_registry();
    Json(crate::domains::agents::examples::offered_examples(
        state.grade.experimental_features_enabled(),
        &|id| registry.has(id),
    ))
}

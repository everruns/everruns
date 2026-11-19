// Harness Examples API — read-only catalogue of adoptable harness templates.
//
// Decision: Examples live in code (see `crate::harnesses::examples`), not in DB.
// Decision: Adoption is handled by `POST /v1/harnesses/import?from-example={name}`
// Decision: Examples are identified by their `name` (slug).
// Decision: the catalogue, its filter and adoption live in
// `domains::harnesses::examples` as commands, so the CLI and MCP see the same
// set; this route only serves it over REST.

use crate::auth::{AuthState, ResolvedOrg};
pub use crate::domains::harnesses::HarnessExample;
use axum::{Json, Router, extract::State, routing::get};
use everruns_core::DeploymentGrade;
use everruns_core::host::HostComposition;
use std::sync::Arc;

use super::common::impl_auth_state;

/// App state for harness example routes.
#[derive(Clone)]
pub struct AppState {
    pub auth: AuthState,
    pub grade: DeploymentGrade,
    pub host_composition: Arc<HostComposition>,
}

impl_auth_state!(AppState);

/// Create harness example routes.
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/harness-examples", get(list_examples))
        .with_state(state)
}

/// GET /v1/harness-examples — list all available harness examples.
#[utoipa::path(
    get,
    path = "/v1/harness-examples",
    responses(
        (status = 200, description = "List of harness examples", body = Vec<HarnessExample>),
    ),
    tag = "harness-examples"
)]
pub async fn list_examples(
    _org: ResolvedOrg,
    State(state): State<AppState>,
) -> Json<Vec<HarnessExample>> {
    let registry = state.host_composition.capability_registry();
    Json(
        crate::domains::harnesses::examples::offered_harness_examples(
            state.grade.experimental_features_enabled(),
            &|id| registry.has(id),
        ),
    )
}

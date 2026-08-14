// Harness Examples API — read-only catalogue of adoptable harness templates.
//
// Decision: Examples live in code (see `crate::harnesses::examples`), not in DB.
// Decision: Adoption is handled by `POST /v1/harnesses/import?from-example={name}`
// Decision: Examples are identified by their `name` (slug).
// Decision: Examples are filtered at request time by capability registration so
//   that capability-gated harnesses (e.g. `coding-container`) only appear when
//   the corresponding capability plugin is registered for the deployment.

use crate::auth::{AuthState, ResolvedOrg};
use crate::harnesses::{HarnessExampleDef, harness_examples};
use axum::{Json, Router, extract::State, routing::get};
use everruns_core::DeploymentGrade;
use everruns_host::HostComposition;
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::impl_auth_state;

/// A read-only harness example defined in code.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HarnessExample {
    /// Unique slug (e.g. `data-analyst`).
    pub name: String,
    /// Human-readable display name (e.g. `Data Analyst`).
    pub display_name: String,
    /// Short description.
    pub description: String,
    /// Tags for categorization.
    pub tags: Vec<String>,
    /// Name of the parent harness this example inherits from when adopted
    /// (e.g. `generic`). Resolved per-org at import time — no UUIDs are
    /// hardcoded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_name: Option<String>,
    /// Capabilities the example will assign with their per-harness config.
    #[schema(value_type = Vec<everruns_platform::CapabilityRefSchema>)]
    pub capabilities: Vec<everruns_capability::CapabilityRef>,
    /// Whether this example is only available when experimental features are on.
    pub dev_only: bool,
}

fn example_to_dto(ex: &HarnessExampleDef) -> HarnessExample {
    HarnessExample {
        name: ex.definition.name.clone(),
        display_name: ex.definition.display_name.clone(),
        description: ex.definition.description.clone(),
        tags: ex.definition.tags.clone(),
        parent_name: ex.definition.parent_name.clone(),
        capabilities: ex
            .definition
            .capabilities
            .iter()
            .map(|cap| {
                everruns_capability::CapabilityRef::with_config(
                    cap.typed_id().clone(),
                    cap.config_value().clone(),
                )
            })
            .collect(),
        dev_only: ex.dev_only,
    }
}

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
    let include_dev = state.grade.experimental_features_enabled();
    let platform = &state.host_composition;

    let examples: Vec<HarnessExample> = harness_examples()
        .iter()
        .filter(|ex| {
            if ex.dev_only && !include_dev {
                return false;
            }
            // Only show examples whose capabilities are all registered.
            ex.definition
                .capabilities
                .iter()
                .all(|cap| platform.capability_registry().has(cap.capability_id()))
        })
        .map(example_to_dto)
        .collect();

    Json(examples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dto_round_trips_capabilities() {
        let examples = harness_examples();
        let dto = example_to_dto(&examples[0]);
        assert!(!dto.capabilities.is_empty());
        assert_eq!(dto.parent_name.as_deref(), Some("generic"));
    }

    #[test]
    fn known_example_present_in_catalog() {
        let names: Vec<String> = harness_examples()
            .iter()
            .map(|e| e.definition.name.clone())
            .collect();
        for expected in ["coding-daytona", "coding-container", "data-analyst"] {
            assert!(
                names.iter().any(|n| n == expected),
                "expected {expected} in harness examples catalogue"
            );
        }
    }
}

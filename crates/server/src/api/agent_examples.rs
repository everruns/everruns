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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_templates::{REPOSITORY_PLACEHOLDER, agent_examples};
    use crate::domains::agents::examples::seed_to_example;
    use crate::setup::seed::SEED_AGENTS;

    #[test]
    fn templates_carry_their_setup_and_plain_examples_do_not() {
        let examples: Vec<AgentExample> = agent_examples()
            .map(|(seed, setup)| seed_to_example(seed, setup))
            .collect();
        let reviewer = examples.iter().find(|e| e.name == "pr-reviewer").unwrap();
        let setup = reviewer.setup.as_ref().expect("guided setup");
        assert!(setup.connect_github);
        assert_eq!(setup.trigger["trigger_type"], "github");
        assert_eq!(setup.repository_placeholder, REPOSITORY_PLACEHOLDER);

        let scanner = examples
            .iter()
            .find(|e| e.name == "security-scanner")
            .unwrap();
        let fix = scanner.setup.as_ref().unwrap().settings.iter();
        let fix = fix
            .clone()
            .find(|s| s.key == "open_fix_pull_requests")
            .unwrap();
        assert!(!fix.default);

        let jokes = examples
            .iter()
            .find(|e| e.name == "dad-jokes-agent")
            .unwrap();
        assert!(jokes.setup.is_none());
        assert_eq!(jokes.harness_name, "conversation");
        let json = serde_json::to_value(jokes).unwrap();
        assert!(
            json.get("setup").is_none(),
            "plain examples keep their shape"
        );
    }

    #[test]
    fn test_all_seeds_have_unique_names() {
        let names: Vec<&str> = SEED_AGENTS.iter().map(|s| s.name).collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(names.len(), unique.len(), "Duplicate example names");
    }

    #[test]
    fn test_known_seed_exists_by_name() {
        let seed = SEED_AGENTS.iter().find(|s| s.name == "dad-jokes-agent");
        assert!(seed.is_some());
        assert_eq!(seed.unwrap().display_name, "Dad Jokes Agent");
    }

    #[test]
    fn test_unknown_name_returns_none() {
        let seed = SEED_AGENTS.iter().find(|s| s.name == "nonexistent");
        assert!(seed.is_none());
    }
}

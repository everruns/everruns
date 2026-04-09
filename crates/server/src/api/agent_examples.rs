// Agent Examples API — read-only catalogue of built-in examples.
//
// Decision: Examples live in code (SEED_AGENTS), not in DB.
// Decision: Import is handled by POST /v1/agents/import?from-example={name}
// Decision: Examples are identified by their name

use crate::auth::{AuthState, ResolvedOrg};
use crate::seed::{SEED_AGENTS, SeedAgent};
use axum::{Json, Router, extract::State, routing::get};
use everruns_core::{AgentCapabilityConfig, DeploymentGrade, PlatformDefinition};
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::impl_auth_state;

/// A read-only agent example defined in code
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentExample {
    /// Name (e.g. "dad-jokes-agent")
    pub name: String,
    /// Human-readable display name (e.g. "Dad Jokes Agent")
    pub display_name: String,
    /// Short description
    pub description: String,
    /// Tags for categorization
    pub tags: Vec<String>,
    /// Capability IDs this example uses
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Whether this example requires dev/experimental mode
    pub dev_only: bool,
}

fn seed_to_example(seed: &SeedAgent) -> AgentExample {
    AgentExample {
        name: seed.name.to_string(),
        display_name: seed.display_name.to_string(),
        description: seed.description.to_string(),
        tags: seed.tags.iter().map(|s| s.to_string()).collect(),
        capabilities: seed
            .capabilities
            .iter()
            .map(|cap| {
                let config = cap.config.map_or_else(|| serde_json::json!({}), |f| f());
                AgentCapabilityConfig::with_config(cap.id.to_string(), config)
            })
            .collect(),
        dev_only: seed.dev_only,
    }
}

/// App state for agent example routes
#[derive(Clone)]
pub struct AppState {
    pub auth: AuthState,
    pub grade: DeploymentGrade,
    pub platform_definition: Arc<PlatformDefinition>,
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
    let include_dev = state.grade.experimental_features_enabled();
    let platform = &state.platform_definition;

    let examples: Vec<AgentExample> = SEED_AGENTS
        .iter()
        .filter(|s| {
            if s.dev_only && !include_dev {
                return false;
            }
            // Only show examples whose capabilities are all registered
            s.capabilities
                .iter()
                .all(|cap| platform.capability_registry().has(cap.id))
        })
        .map(seed_to_example)
        .collect();

    Json(examples)
}

#[cfg(test)]
mod tests {
    use super::*;

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

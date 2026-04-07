// Agent Examples API — read-only examples defined in code, adoptable as real Agents.
//
// Decision: Examples live in code (SEED_AGENTS), not in DB.
// Decision: "Use" creates a real Agent via the existing create flow
// Decision: Examples are identified by their addressable name

use crate::auth::{AuthState, ResolvedOrg};
use crate::seed::{SEED_AGENTS, SeedAgent};
use crate::services::CapabilityService;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::{Agent, AgentCapabilityConfig, Caller, DeploymentGrade, PlatformDefinition};
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

use super::agents::CreateAgentRequest;
use super::common::{ApiPolicyResultExt, ErrorResponse, impl_auth_state};

use crate::services::AgentService;

/// A read-only agent example defined in code
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentExample {
    /// Addressable name (e.g. "dad-jokes-agent")
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

fn find_seed_by_name(name: &str) -> Option<&'static SeedAgent> {
    SEED_AGENTS.iter().find(|s| s.name == name)
}

/// App state for agent example routes
#[derive(Clone)]
pub struct AppState {
    pub agent_service: Arc<AgentService>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
    pub grade: DeploymentGrade,
    pub platform_definition: Arc<PlatformDefinition>,
}

impl_auth_state!(AppState);

/// Create agent example routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/agent-examples", get(list_examples))
        .route("/v1/agent-examples/{name}/use", post(use_example))
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

/// POST /v1/agent-examples/{name}/use — create a real Agent from an example
#[utoipa::path(
    post,
    path = "/v1/agent-examples/{name}/use",
    params(
        ("name" = String, Path, description = "Example name (e.g. dad-jokes-agent)")
    ),
    responses(
        (status = 201, description = "Agent created from example", body = Agent),
        (status = 404, description = "Example not found"),
        (status = 403, description = "High-risk capabilities require admin role"),
    ),
    tag = "agent-examples"
)]
pub async fn use_example(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<(StatusCode, Json<Agent>), (StatusCode, Json<ErrorResponse>)> {
    let seed = find_seed_by_name(&name)
        .ok_or_else(|| ErrorResponse::not_found(&format!("agent example '{name}'")))?;

    // Check dev-only
    if seed.dev_only && !state.grade.experimental_features_enabled() {
        return Err(ErrorResponse::not_found(&format!("agent example '{name}'")));
    }

    // Check capabilities registered
    let missing: Vec<&str> = seed
        .capabilities
        .iter()
        .map(|c| c.id)
        .filter(|id| !state.platform_definition.capability_registry().has(id))
        .collect();
    if !missing.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Example requires unregistered capabilities: {missing:?}"),
            }),
        ));
    }

    let capabilities: Vec<AgentCapabilityConfig> = seed
        .capabilities
        .iter()
        .map(|cap| {
            let config = cap.config.map_or_else(|| serde_json::json!({}), |f| f());
            AgentCapabilityConfig::with_config(cap.id.to_string(), config)
        })
        .collect();

    // TM-AGENT-005: High-risk capabilities require admin role
    super::agents::require_admin_for_high_risk(&org, &capabilities, &state.capability_service)?;

    let req = CreateAgentRequest {
        id: None,
        name: seed.name.to_string(),
        display_name: seed.display_name.to_string(),
        description: Some(seed.description.to_string()),
        system_prompt: seed.system_prompt.to_string(),
        default_model_id: None,
        tags: seed.tags.iter().map(|s| s.to_string()).collect(),
        capabilities,
        initial_files: vec![],
        tools: vec![],
        network_access: None,
        max_iterations: None,
    };

    let caller = Caller::from(&org);
    let agent = state
        .agent_service
        .create(&caller, None, req)
        .await
        .map_policy_or_internal("use agent example")?;

    Ok((StatusCode::CREATED, Json(agent)))
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
    fn test_find_seed_by_name() {
        let seed = find_seed_by_name("dad-jokes-agent");
        assert!(seed.is_some());
        assert_eq!(seed.unwrap().display_name, "Dad Jokes Agent");
    }

    #[test]
    fn test_find_seed_by_name_not_found() {
        assert!(find_seed_by_name("nonexistent").is_none());
    }
}

// Agent Templates API — read-only templates defined in code, installable as real Agents
//
// Decision: Templates live in code (SEED_AGENTS), not in DB
// Decision: "Install" creates a real Agent via the existing create flow
// Decision: Templates use a slug (kebab-case name) as their identifier

use crate::auth::{AuthState, ResolvedOrg};
use crate::seed::{SEED_AGENTS, SeedAgent};
use crate::services::CapabilityService;
use axum::extract::FromRef;
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
use super::common::{ApiPolicyResultExt, ErrorResponse};

use crate::services::AgentService;

/// A read-only agent template defined in code
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentTemplate {
    /// URL-safe identifier (kebab-case of name)
    pub slug: String,
    /// Display name
    pub name: String,
    /// Short description
    pub description: String,
    /// Tags for categorization
    pub tags: Vec<String>,
    /// Capability IDs this template uses
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Whether this template requires dev/experimental mode
    pub dev_only: bool,
}

fn slug_from_name(name: &str) -> String {
    name.to_lowercase()
        .replace(' ', "-")
        .replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "")
}

fn seed_to_template(seed: &SeedAgent) -> AgentTemplate {
    AgentTemplate {
        slug: slug_from_name(seed.name),
        name: seed.name.to_string(),
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

fn find_seed_by_slug(slug: &str) -> Option<&'static SeedAgent> {
    SEED_AGENTS.iter().find(|s| slug_from_name(s.name) == slug)
}

/// App state for agent template routes
#[derive(Clone)]
pub struct AppState {
    pub agent_service: Arc<AgentService>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
    pub grade: DeploymentGrade,
    pub platform_definition: Arc<PlatformDefinition>,
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Create agent template routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/agent-templates", get(list_templates))
        .route("/v1/agent-templates/{slug}/install", post(install_template))
        .with_state(state)
}

/// GET /v1/agent-templates — list all available templates
#[utoipa::path(
    get,
    path = "/v1/agent-templates",
    responses(
        (status = 200, description = "List of agent templates", body = Vec<AgentTemplate>),
    ),
    tag = "agent-templates"
)]
pub async fn list_templates(
    _org: ResolvedOrg,
    State(state): State<AppState>,
) -> Json<Vec<AgentTemplate>> {
    let include_dev = state.grade.experimental_features_enabled();
    let platform = &state.platform_definition;

    let templates: Vec<AgentTemplate> = SEED_AGENTS
        .iter()
        .filter(|s| {
            if s.dev_only && !include_dev {
                return false;
            }
            // Only show templates whose capabilities are all registered
            s.capabilities
                .iter()
                .all(|cap| platform.capability_registry().has(cap.id))
        })
        .map(seed_to_template)
        .collect();

    Json(templates)
}

/// POST /v1/agent-templates/{slug}/install — create a real Agent from a template
#[utoipa::path(
    post,
    path = "/v1/agent-templates/{slug}/install",
    params(
        ("slug" = String, Path, description = "Template slug (kebab-case name)")
    ),
    responses(
        (status = 201, description = "Agent created from template", body = Agent),
        (status = 404, description = "Template not found"),
        (status = 403, description = "High-risk capabilities require admin role"),
    ),
    tag = "agent-templates"
)]
pub async fn install_template(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<(StatusCode, Json<Agent>), (StatusCode, Json<ErrorResponse>)> {
    let seed = find_seed_by_slug(&slug)
        .ok_or_else(|| ErrorResponse::not_found(&format!("agent template '{slug}'")))?;

    // Check dev-only
    if seed.dev_only && !state.grade.experimental_features_enabled() {
        return Err(ErrorResponse::not_found(&format!(
            "agent template '{slug}'"
        )));
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
                error: format!("Template requires unregistered capabilities: {missing:?}"),
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
        description: Some(seed.description.to_string()),
        system_prompt: seed.system_prompt.to_string(),
        default_model_id: None,
        tags: seed.tags.iter().map(|s| s.to_string()).collect(),
        capabilities,
        initial_files: vec![],
        tools: vec![],
    };

    let caller = Caller::from(&org);
    let agent = state
        .agent_service
        .create(&caller, None, req)
        .await
        .map_policy_or_internal("install agent template")?;

    Ok((StatusCode::CREATED, Json(agent)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slug_from_name() {
        assert_eq!(slug_from_name("Dad Jokes Agent"), "dad-jokes-agent");
        assert_eq!(slug_from_name("Python Coder"), "python-coder");
        assert_eq!(
            slug_from_name("Cloud Cost & Security Auditor"),
            "cloud-cost--security-auditor"
        );
    }

    #[test]
    fn test_all_seeds_have_unique_slugs() {
        let slugs: Vec<String> = SEED_AGENTS.iter().map(|s| slug_from_name(s.name)).collect();
        let mut unique = slugs.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(slugs.len(), unique.len(), "Duplicate template slugs");
    }

    #[test]
    fn test_find_seed_by_slug() {
        let seed = find_seed_by_slug("dad-jokes-agent");
        assert!(seed.is_some());
        assert_eq!(seed.unwrap().name, "Dad Jokes Agent");
    }

    #[test]
    fn test_find_seed_by_slug_not_found() {
        assert!(find_seed_by_slug("nonexistent").is_none());
    }
}

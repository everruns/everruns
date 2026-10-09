// Agent Examples API — read-only catalogue of built-in examples.
//
// Decision: Examples live in code (SEED_AGENTS), not in DB.
// Decision: Import is handled by POST /v1/agents/import?from-example={name}
// Decision: Examples are identified by their name
// Decision: Guided templates (crate::agent_templates) are examples with a
// `setup` the UI walks through after import; same list, same import path.

use crate::agent_templates::{REPOSITORY_PLACEHOLDER, TemplateSetup, agent_examples};
use crate::auth::{AuthState, ResolvedOrg};
use crate::setup::seed::SeedAgent;
use axum::{Json, Router, extract::State, routing::get};
use everruns_core::DeploymentGrade;
use everruns_core::host::HostComposition;
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::impl_auth_state;

/// A read-only agent example defined in code
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentExample {
    /// Name (e.g. "dad-jokes-agent")
    #[schema(example = "dad-jokes-agent")]
    pub name: String,
    /// Human-readable display name (e.g. "Dad Jokes Agent")
    #[schema(example = "Dad Jokes Agent")]
    pub display_name: String,
    /// Short description
    #[schema(example = "A friendly agent that tells dad jokes and knows what time it is.")]
    pub description: String,
    /// Explicit harness selected when importing this example.
    #[schema(example = "conversation")]
    pub harness_name: String,
    /// Tags for categorization
    #[schema(example = json!(["humor", "demo"]))]
    pub tags: Vec<String>,
    /// Capability IDs this example uses
    #[schema(value_type = Vec<crate::records::CapabilityRefSchema>, example = json!(["current_time"]))]
    pub capabilities: Vec<everruns_contracts::CapabilityRef>,
    /// Whether this example requires dev/experimental mode
    #[schema(example = false)]
    pub dev_only: bool,
    /// Guided setup to run after importing, for templates that need one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup: Option<AgentExampleSetup>,
}

/// What the UI walks the user through after importing a template.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentExampleSetup {
    /// The agent needs its own GitHub App (`POST /v1/agents/{id}/github/connect`).
    #[schema(example = true)]
    pub connect_github: bool,
    /// Other connection providers the agent's service account needs.
    #[schema(example = json!(["daytona"]))]
    pub connections: Vec<String>,
    /// Text in `trigger` to replace with the picked `owner/repo`.
    #[schema(example = "${repository}")]
    pub repository_placeholder: String,
    /// Body for `POST /v1/agents/{agent_id}/triggers` once the placeholder is
    /// replaced.
    #[schema(value_type = Object, example = json!({"trigger_type": "schedule", "cron_expression": "0 6 * * 1", "timezone": "UTC", "session_mode": "session_per_invocation", "message": "Scan ${repository} on its default branch."}))]
    pub trigger: serde_json::Value,
    /// Yes/no choices, each stored as one capability config key.
    #[schema(example = json!([]))]
    pub settings: Vec<AgentExampleSetting>,
}

/// One setup choice. Turning it on sets `config_key` to `true` in the config
/// of the agent's `capability`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentExampleSetting {
    /// Stable setup option key.
    #[schema(example = "open_fix_pull_requests")]
    pub key: String,
    /// Human-readable setup option label.
    #[schema(example = "Open fix pull requests")]
    pub label: String,
    /// Explains the effect of enabling this option.
    #[schema(example = "Let the scanner open draft pull requests for high-confidence fixes.")]
    pub description: String,
    /// Capability whose configuration this option changes.
    #[schema(example = "github")]
    pub capability: String,
    /// Boolean configuration property controlled by this option.
    #[schema(example = "allow_pull_requests")]
    pub config_key: String,
    /// Initial value shown during guided setup.
    #[schema(example = false)]
    pub default: bool,
}

fn setup_to_api(setup: &TemplateSetup) -> AgentExampleSetup {
    AgentExampleSetup {
        connect_github: setup.connect_github,
        connections: setup.connections.iter().map(|c| c.to_string()).collect(),
        repository_placeholder: REPOSITORY_PLACEHOLDER.to_string(),
        trigger: (setup.trigger)(),
        settings: setup
            .settings
            .iter()
            .map(|s| AgentExampleSetting {
                key: s.key.to_string(),
                label: s.label.to_string(),
                description: s.description.to_string(),
                capability: s.capability.to_string(),
                config_key: s.config_key.to_string(),
                default: s.default,
            })
            .collect(),
    }
}

fn seed_to_example(seed: &SeedAgent, setup: Option<&TemplateSetup>) -> AgentExample {
    AgentExample {
        name: seed.name.to_string(),
        display_name: seed.display_name.to_string(),
        description: seed.description.to_string(),
        harness_name: seed.harness_name.to_string(),
        tags: seed.tags.iter().map(|s| s.to_string()).collect(),
        capabilities: seed
            .capabilities
            .iter()
            .map(|cap| {
                let config = cap.config.map_or_else(|| json!({}), |f| f());
                everruns_contracts::CapabilityRef::with_config(cap.id.to_string(), config)
            })
            .collect(),
        dev_only: seed.dev_only,
        setup: setup.map(setup_to_api),
    }
}

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
    let platform = &state.host_composition;

    let examples: Vec<AgentExample> = agent_examples()
        .filter(|(s, _)| {
            if s.dev_only && !include_dev {
                return false;
            }
            // Only show examples whose capabilities are all registered
            s.capabilities
                .iter()
                .all(|cap| platform.capability_registry().has(cap.id))
        })
        .map(|(seed, setup)| seed_to_example(seed, setup))
        .collect();

    Json(examples)
}

#[cfg(test)]
mod tests {
    use super::*;
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

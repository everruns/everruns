// Agent examples (blueprints): list and adopt them as commands.
//
// Decision: examples live in code (`SEED_AGENTS` plus the guided templates in
// `crate::agent_templates`), not in the database, and are identified by name.
// Decision: listing and adopting are commands, so the CLI and MCP reach the
// same catalogue the UI does. `GET /v1/agent-examples` and
// `POST /v1/agents/import?from-example={name}` delegate here; there is one
// filter (dev-only, registered capabilities) and one adoption path.
// Decision: the deployment grade is read from the environment, as the REST
// state does (`app_builder` builds it with `DeploymentGrade::from_env`), and
// the capability registry is the one `CapabilityService` was built with.

use super::types::CreateAgentRequest;
use super::{AGENT_MANAGE, AGENT_VIEW, CreateAgent};
use crate::agent_templates::{REPOSITORY_PLACEHOLDER, TemplateSetup, agent_examples};
use crate::domains::agents::record::Agent;
use crate::domains::common::*;
use crate::setup::seed::SeedAgent;
use everruns_core::DeploymentGrade;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

/// Tag prefix that records the example an agent was adopted from.
pub const EXAMPLE_TAG_PREFIX: &str = "example:";

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

fn capability_refs(seed: &SeedAgent) -> Vec<everruns_contracts::CapabilityRef> {
    seed.capabilities
        .iter()
        .map(|cap| {
            let config = cap.config.map_or_else(|| json!({}), |f| f());
            everruns_contracts::CapabilityRef::with_config(cap.id.to_string(), config)
        })
        .collect()
}

pub(crate) fn seed_to_example(seed: &SeedAgent, setup: Option<&TemplateSetup>) -> AgentExample {
    AgentExample {
        name: seed.name.to_string(),
        display_name: seed.display_name.to_string(),
        description: seed.description.to_string(),
        harness_name: seed.harness_name.to_string(),
        tags: seed.tags.iter().map(|s| s.to_string()).collect(),
        capabilities: capability_refs(seed),
        dev_only: seed.dev_only,
        setup: setup.map(setup_to_api),
    }
}

/// Whether this deployment offers `seed`: dev-only examples need experimental
/// features, and every capability the example uses must be registered.
fn offered(seed: &SeedAgent, include_dev: bool, has_capability: &dyn Fn(&str) -> bool) -> bool {
    (include_dev || !seed.dev_only) && seed.capabilities.iter().all(|cap| has_capability(cap.id))
}

/// The examples a deployment offers, in catalogue order.
pub fn offered_examples(
    include_dev: bool,
    has_capability: &dyn Fn(&str) -> bool,
) -> Vec<AgentExample> {
    agent_examples()
        .filter(|(seed, _)| offered(seed, include_dev, has_capability))
        .map(|(seed, setup)| seed_to_example(seed, setup))
        .collect()
}

fn include_dev() -> bool {
    DeploymentGrade::from_env().experimental_features_enabled()
}

// ============================================================================
// ListAgentExamples
// ============================================================================

/// List the agent examples (blueprints) this deployment offers.
#[derive(Debug, Default, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct ListAgentExamples {}

#[command(
    name = "list_agent_examples",
    category = "agents",
    description = "List adoptable agent examples (blueprints such as pr-reviewer): name, description, harness, capabilities, and the guided setup a template needs after adoption (GitHub App, connections, trigger). Adopt one by name with adopt_agent_example.",
    method = "GET",
    path = "/v1/agent-examples",
    policy = AGENT_VIEW,
    cli = CliRoute::new(&["agents", "examples"], "list").with_examples(&[CliExample::new("Find a ready-made agent to start from instead of writing one by hand", "everruns agents examples list")]),
)]
impl Command for ListAgentExamples {
    type Output = Vec<AgentExample>;

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<AgentExample>())
    }

    fn output_shape() -> &'static str {
        "array"
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<AgentExample>, CommandError> {
        let registry = ctx.capability_service.registry();
        Ok(offered_examples(include_dev(), &|id| registry.has(id)))
    }
}

// ============================================================================
// AdoptAgentExample
// ============================================================================

/// Create an agent from an agent example.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AdoptAgentExample {
    /// Example name from list_agent_examples, such as `pr-reviewer`.
    pub name: String,
}

#[command(
    name = "adopt_agent_example",
    category = "agents",
    description = "Create an agent from an agent example by name (see list_agent_examples). The agent is named after the example, with a short suffix when that name is taken, and tagged example:<name>. A template's guided setup (GitHub App, connections, trigger) is not run: do those steps next.",
    method = "POST",
    path = "/v1/agents/import",
    policy = AGENT_MANAGE,
    positional = "name",
    cli = CliRoute::new(&["agents", "examples"], "adopt").with_args(&[CliArg::new("name").at(1)]).with_examples(&[CliExample::new("Start a pull request reviewer from its template", "everruns agents examples adopt pr-reviewer --reason 'Set up PR reviews for the repo'")]),
)]
impl Command for AdoptAgentExample {
    type Output = Agent;

    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        let name = self.name.trim();
        let not_found = || CommandError::not_found(&format!("agent example '{name}'"));
        let seed = crate::agent_templates::find_agent_example(name).ok_or_else(not_found)?;
        // A dev-only example does not exist outside experimental deployments.
        if seed.dev_only && !include_dev() {
            return Err(not_found());
        }
        let registry = ctx.capability_service.registry();
        let missing: Vec<&str> = seed
            .capabilities
            .iter()
            .map(|c| c.id)
            .filter(|id| !registry.has(id))
            .collect();
        if !missing.is_empty() {
            return Err(CommandError::bad_request(format!(
                "Example requires unregistered capabilities: {missing:?}"
            )));
        }

        let req = CreateAgentRequest {
            service_virtual_user_id: None,
            id: None,
            name: unique_name(ctx, seed.name).await?,
            display_name: Some(seed.display_name.to_string()),
            description: Some(seed.description.to_string()),
            intro_markdown: None,
            short_description: None,
            starters: Vec::new(),
            system_prompt: seed.system_prompt.to_string(),
            default_model_id: None,
            harness_id: None,
            harness_name: Some(seed.harness_name.to_string()),
            // `example:<name>` records which example the agent was adopted from,
            // so the Agents page can tell a guided example whose setup never
            // finished (no trigger yet) from an agent someone built by hand.
            tags: seed
                .tags
                .iter()
                .map(|s| s.to_string())
                .chain(std::iter::once(format!(
                    "{EXAMPLE_TAG_PREFIX}{}",
                    seed.name
                )))
                .collect(),
            capabilities: capability_refs(seed),
            sandbox_policy: None,
            initial_files: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            max_output_tokens: None,
            communication: Default::default(),
        };

        // Composed under this command's own policy (the same AGENT_MANAGE) and
        // history entry; CreateAgent still enforces TM-AGENT-005 (high-risk
        // capabilities need an admin) and the per-org agent cap.
        CreateAgent(req).execute(ctx).await
    }
}

/// The example's name, or a suffixed variant when it is taken. Several tries
/// keep a one-shot collision (or a concurrent adoption) from failing.
async fn unique_name(ctx: &Ctx, base: &str) -> Result<String, CommandError> {
    use rand::RngExt;

    for attempt in 0..10 {
        let candidate = if attempt == 0 {
            base.to_string()
        } else {
            let suffix: String = rand::rng()
                .sample_iter(&rand::distr::Alphanumeric)
                .take(5)
                .map(|c| (c as char).to_ascii_lowercase())
                .collect();
            format!("{base}-{suffix}")
        };
        if ctx
            .db
            .get_agent_by_name(ctx.org_id(), &candidate)
            .await?
            .is_none()
        {
            return Ok(candidate);
        }
    }
    Err(CommandError::conflict(format!(
        "No free agent name derived from '{base}'"
    )))
}

#[cfg(test)]
mod tests;

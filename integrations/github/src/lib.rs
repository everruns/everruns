//! GitHub-backed blueprints for Everruns.
//!
//! This crate registers the `github_scout` capability through Everruns' inventory
//! plugin system. The capability is blueprint-only: it does not add tools to a
//! host agent, and instead contributes the `github_scout` agent blueprint.
//!
//! `github_scout` runs as a read-only child agent with private GitHub REST API
//! tools for code search, file reads, and issue or pull request search. Tool
//! credentials are resolved from the existing `github` user connection, with a
//! `GITHUB_TOKEN` session secret fallback for local and compatibility flows.

mod client;
mod tools;

use everruns_core::capabilities::{
    AgentBlueprint, BlueprintModel, Capability, CapabilityLocalization, CapabilityStatus,
    IntegrationPlugin,
};
use everruns_core::tools::Tool;
use serde_json::json;

use tools::{ReadGitHubFileTool, SearchGitHubCodeTool, SearchGitHubIssuesTool};

inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,
        feature_flag: None,
        factory: || Box::new(GitHubScoutCapability),
    }
}

pub const GITHUB_API_BASE: &str = "https://api.github.com";
pub const GITHUB_CONNECTION_PROVIDER: &str = "github";
pub const GITHUB_TOKEN_SECRET: &str = "GITHUB_TOKEN";

pub struct GitHubScoutCapability;

impl Capability for GitHubScoutCapability {
    fn id(&self) -> &str {
        "github_scout"
    }

    fn name(&self) -> &str {
        "GitHub Scout"
    }

    fn description(&self) -> &str {
        "Blueprint-only GitHub repository scout that can spawn read-only GitHub exploration subagents."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("github")
    }

    fn category(&self) -> Option<&str> {
        Some("Integrations")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["subagents"]
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "GitHub Scout",
            "Розвідник репозиторіїв GitHub, що працює лише через blueprint і може породжувати \
             субагентів для дослідження GitHub у режимі лише читання.",
        )]
    }

    fn agent_blueprints(&self) -> Vec<AgentBlueprint> {
        vec![AgentBlueprint {
            id: "github_scout",
            name: "GitHub Scout",
            description: "Search GitHub repositories for code, files, issues, and pull requests. Fast read-only agent for codebase exploration and pattern discovery.",
            model: BlueprintModel::Fixed("claude-haiku-4-5-20251001".to_string()),
            system_prompt: GITHUB_SCOUT_PROMPT,
            tools: vec![
                Box::new(SearchGitHubCodeTool),
                Box::new(ReadGitHubFileTool),
                Box::new(SearchGitHubIssuesTool),
            ],
            max_turns: Some(15),
            config_schema: Some(json!({
                "type": "object",
                "properties": {
                    "repos": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "pattern": "^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$"
                        },
                        "description": "Repository list to scope searches, in owner/repo format."
                    }
                },
                "additionalProperties": false
            })),
        }]
    }
}

const GITHUB_SCOUT_PROMPT: &str = r#"You are GitHub Scout, a read-only repository exploration agent.

Use your GitHub tools to find concrete code, files, issues, and pull requests relevant to the task. Prefer targeted searches over broad scans. When the host provides config.repos, scope searches to those repositories unless the task clearly asks otherwise.

Return a concise summary with:
- the answer or finding,
- the most relevant file paths, symbols, issues, or pull requests,
- direct URLs when useful,
- any uncertainty or gaps."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_is_blueprint_only() {
        let cap = GitHubScoutCapability;
        assert_eq!(cap.id(), "github_scout");
        assert_eq!(cap.name(), "GitHub Scout");
        assert!(cap.tools().is_empty());
        assert_eq!(cap.dependencies(), vec!["subagents"]);
    }

    #[test]
    fn uk_localization_resolves() {
        let cap = GitHubScoutCapability;
        assert_ne!(cap.localized_description(Some("uk")), cap.description());
        assert_eq!(cap.localized_name(Some("uk")), "GitHub Scout");
    }

    #[test]
    fn contributes_github_scout_blueprint() {
        let cap = GitHubScoutCapability;
        let blueprints = cap.agent_blueprints();
        assert_eq!(blueprints.len(), 1);

        let scout = &blueprints[0];
        assert_eq!(scout.id, "github_scout");
        assert_eq!(scout.name, "GitHub Scout");
        assert_eq!(scout.max_turns, Some(15));
        assert!(matches!(
            scout.model,
            BlueprintModel::Fixed(ref model) if model == "claude-haiku-4-5-20251001"
        ));

        let tool_names: Vec<&str> = scout.tools.iter().map(|tool| tool.name()).collect();
        assert_eq!(
            tool_names,
            vec![
                "search_github_code",
                "read_github_file",
                "search_github_issues"
            ]
        );
    }
}

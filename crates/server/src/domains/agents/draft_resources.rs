// What the agent builder may wire up beyond the prompt: a GitHub trigger,
// MCP servers from the org catalog, and the model (proactive agents D6).
//
// Decisions:
// - MCP servers come only from the org catalog, by preset name. The draft never
//   adds a host or a credential. An OAuth preset acts as the agent's service
//   identity, because a proactive agent runs unattended with nobody to sign in;
//   a preset without auth, or with an org key, needs no grant.
// - Notifying the owner goes through the catalog (Visti today): the builder is
//   told to attach a preset whose description says it sends notifications. There
//   is no built-in notify capability (D3 was descoped).
// - GitHub events come from a fixed list. `changes_only` keeps only CI outcome
//   changes, so it is dropped when no CI event is chosen; so are `branches` and
//   `workflows`, which narrow CI events to named branches and workflow names.
// - The model is never the LLM's choice. The draft keeps the org default unless
//   that model has no working key, then names the first enabled chat model that
//   has one and says why. "Working" is the model list's `healthy` flag: an
//   active provider with a key. Funding is not knowable before a call.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domains::common::*;
use everruns_core::mcp_server::{McpServerActsAs, McpServerAuthMode};

/// GitHub events the builder may subscribe a trigger to.
pub const DRAFT_GITHUB_EVENTS: &[&str] = &[
    "pull_request.opened",
    "pull_request.reopened",
    "pull_request.synchronize",
    "pull_request.ready_for_review",
    "pull_request.closed",
    "issue_comment.created",
    "workflow_run.completed",
    "check_suite.completed",
];

/// Events whose normalized CI outcome `changes_only` compares.
const CI_EVENTS: &[&str] = &["workflow_run.completed", "check_suite.completed"];

const MAX_REPOSITORIES: usize = 20;
const MAX_CI_NAMES: usize = 10;
const MAX_CI_NAME_CHARS: usize = 100;
const MAX_TRIGGER_MESSAGE_CHARS: usize = 2_000;
const MAX_OFFERED_PRESETS: usize = 40;

/// A GitHub trigger to create once GitHub is connected on the agent.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, ToSchema)]
pub struct DraftGitHubTrigger {
    /// Events from the builder's fixed list, e.g. `workflow_run.completed`.
    #[serde(default)]
    pub events: Vec<String>,
    /// `owner/name` repositories; empty means every repository the App sees.
    #[serde(default)]
    pub repositories: Vec<String>,
    /// Run only when a CI outcome differs from the previous one for the same
    /// repository and branch.
    #[serde(default)]
    pub changes_only: bool,
    /// CI events only: branches to accept, e.g. `main`; empty means every branch.
    #[serde(default)]
    pub branches: Vec<String>,
    /// CI events only: workflow (or check suite app) names to accept, e.g. `CI`;
    /// empty means every workflow.
    #[serde(default)]
    pub workflows: Vec<String>,
    /// Message the agent receives for each event.
    #[serde(default)]
    pub message: String,
}

/// A catalog MCP server to attach.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, ToSchema)]
#[serde(from = "McpServerRepr")]
pub struct DraftMcpServer {
    /// Catalog preset name.
    pub name: String,
    /// Whose grant the attachment uses, set from the preset's auth.
    #[schema(value_type = String, example = "service")]
    pub acts_as: McpServerActsAs,
}

/// The model answers with bare preset names; the browser sends full entries.
#[derive(Deserialize)]
#[serde(untagged)]
enum McpServerRepr {
    Name(String),
    Full {
        name: String,
        #[serde(default)]
        acts_as: McpServerActsAs,
    },
}

impl From<McpServerRepr> for DraftMcpServer {
    fn from(repr: McpServerRepr) -> Self {
        match repr {
            McpServerRepr::Name(name) => Self {
                name,
                acts_as: McpServerActsAs::None,
            },
            McpServerRepr::Full { name, acts_as } => Self { name, acts_as },
        }
    }
}

/// A catalog preset the builder may attach.
#[derive(Debug, Clone)]
pub struct OfferedPreset {
    pub name: String,
    pub summary: String,
    pub acts_as: McpServerActsAs,
}

/// Active org catalog presets, newest first.
pub async fn offered_presets(ctx: &Ctx) -> Result<Vec<OfferedPreset>, CommandError> {
    let rows = ctx
        .db
        .list_mcp_server_catalog_page(ctx.org_id(), None, MAX_OFFERED_PRESETS as i64)
        .await?;
    Ok(rows
        .iter()
        .map(crate::domains::mcp_servers::queries::row_to_mcp_server)
        .filter(|server| {
            matches!(
                server.status,
                crate::domains::mcp_servers::record::McpServerStatus::Active
            ) && server.archived_at.is_none()
        })
        .map(|server| OfferedPreset {
            summary: server
                .description
                .as_deref()
                .map(|d| d.chars().take(160).collect())
                .unwrap_or_default(),
            acts_as: acts_as_for(server.auth_mode),
            name: server.name,
        })
        .collect())
}

fn acts_as_for(auth_mode: McpServerAuthMode) -> McpServerActsAs {
    match auth_mode {
        McpServerAuthMode::OAuth => McpServerActsAs::Service,
        McpServerAuthMode::None | McpServerAuthMode::ApiKey => McpServerActsAs::None,
    }
}

/// Keep catalog presets only, once each, with the grant their auth needs.
pub fn normalize_mcp_servers(
    proposed: Vec<DraftMcpServer>,
    presets: &[OfferedPreset],
) -> Vec<DraftMcpServer> {
    let mut out: Vec<DraftMcpServer> = Vec::new();
    for server in proposed {
        let Some(preset) = presets.iter().find(|p| p.name == server.name) else {
            continue;
        };
        if out.iter().any(|s| s.name == preset.name) {
            continue;
        }
        out.push(DraftMcpServer {
            name: preset.name.clone(),
            acts_as: preset.acts_as,
        });
    }
    out
}

/// Keep known events and well-formed repositories; drop a trigger with no
/// event or no message.
pub fn normalize_github_trigger(
    proposed: Option<DraftGitHubTrigger>,
) -> Option<DraftGitHubTrigger> {
    let trigger = proposed?;
    let mut events: Vec<String> = Vec::new();
    for event in trigger.events {
        let event = event.trim().to_string();
        if DRAFT_GITHUB_EVENTS.contains(&event.as_str()) && !events.contains(&event) {
            events.push(event);
        }
    }
    let message: String = trigger
        .message
        .trim()
        .chars()
        .take(MAX_TRIGGER_MESSAGE_CHARS)
        .collect();
    if events.is_empty() || message.is_empty() {
        return None;
    }
    let mut repositories: Vec<String> = Vec::new();
    for repo in trigger.repositories {
        let repo = repo
            .trim()
            .trim_start_matches("https://github.com/")
            .to_string();
        if is_repository(&repo) && !repositories.contains(&repo) {
            repositories.push(repo);
        }
    }
    repositories.truncate(MAX_REPOSITORIES);
    let has_ci = events.iter().any(|e| CI_EVENTS.contains(&e.as_str()));
    let ci_names = |names: Vec<String>| -> Vec<String> {
        if !has_ci {
            return Vec::new();
        }
        let mut out: Vec<String> = Vec::new();
        for name in names {
            let name = name.trim().to_string();
            if !name.is_empty()
                && name.chars().count() <= MAX_CI_NAME_CHARS
                && !name.chars().any(char::is_control)
                && !out.contains(&name)
            {
                out.push(name);
            }
        }
        out.truncate(MAX_CI_NAMES);
        out
    };
    Some(DraftGitHubTrigger {
        changes_only: trigger.changes_only && has_ci,
        branches: ci_names(trigger.branches),
        workflows: ci_names(trigger.workflows),
        events,
        repositories,
        message,
    })
}

fn is_repository(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let valid = |part: Option<&str>| {
        part.is_some_and(|p| {
            !p.is_empty()
                && p.len() <= 100
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
    };
    valid(parts.next()) && valid(parts.next()) && parts.next().is_none()
}

/// The model the draft pins, and why, when the org default cannot run.
pub struct ModelChoice {
    pub model_id: Option<String>,
    pub note: Option<String>,
}

/// Keep the org default when it has a working key; otherwise pick the first
/// enabled chat model that has one. Lookup failures keep the default.
pub async fn choose_model(ctx: &Ctx) -> ModelChoice {
    let keep_default = ModelChoice {
        model_id: None,
        note: None,
    };
    let service = crate::domains::models::queries::service(ctx);
    let default = service.get_default(&ctx.caller).await.ok().flatten();
    if default.as_ref().is_some_and(|m| m.healthy) {
        return keep_default;
    }
    let Ok(models) = service
        .list_all_with_filters(&ctx.caller, None, false, false)
        .await
    else {
        return keep_default;
    };
    let Some(model) = models
        .into_iter()
        .find(|m| m.enabled && m.healthy && m.service == everruns_contracts::ServiceKind::Chat)
    else {
        return keep_default;
    };
    let why = match default {
        Some(default) => format!(
            "The org default model, {}, has no working key",
            default.display_name
        ),
        None => "The org has no default model".to_string(),
    };
    ModelChoice {
        model_id: Some(model.id.to_string()),
        note: Some(format!("{why}, so this agent uses {}.", model.display_name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(name: &str, acts_as: McpServerActsAs) -> OfferedPreset {
        OfferedPreset {
            name: name.into(),
            summary: String::new(),
            acts_as,
        }
    }

    #[test]
    fn mcp_servers_come_only_from_the_catalog_with_the_preset_grant() {
        let presets = [
            preset("visti", McpServerActsAs::None),
            preset("github", McpServerActsAs::Service),
        ];
        let proposed: Vec<DraftMcpServer> = serde_json::from_value(serde_json::json!([
            "visti",
            {"name": "github", "acts_as": "user"},
            "evil-host",
            "visti"
        ]))
        .unwrap();
        let out = normalize_mcp_servers(proposed, &presets);
        assert_eq!(
            out,
            vec![
                DraftMcpServer {
                    name: "visti".into(),
                    acts_as: McpServerActsAs::None
                },
                DraftMcpServer {
                    name: "github".into(),
                    acts_as: McpServerActsAs::Service
                },
            ]
        );
    }

    #[test]
    fn github_trigger_keeps_known_events_and_valid_repositories() {
        let out = normalize_github_trigger(Some(DraftGitHubTrigger {
            events: vec![
                "workflow_run.completed".into(),
                "push".into(),
                "workflow_run.completed".into(),
            ],
            repositories: vec![
                "https://github.com/everruns/everruns".into(),
                "not a repo".into(),
                "a/b/c".into(),
            ],
            changes_only: true,
            branches: vec![" main ".into(), "main".into(), "".into()],
            workflows: vec!["CI".into()],
            message: "  CI finished: {{github.ci.outcome}}  ".into(),
        }))
        .unwrap();
        assert_eq!(out.events, vec!["workflow_run.completed"]);
        assert_eq!(out.repositories, vec!["everruns/everruns"]);
        assert!(out.changes_only);
        assert_eq!(out.branches, vec!["main"]);
        assert_eq!(out.workflows, vec!["CI"]);
        assert_eq!(out.message, "CI finished: {{github.ci.outcome}}");
    }

    #[test]
    fn changes_only_needs_a_ci_event_and_a_trigger_needs_events_and_a_message() {
        let pr_only = normalize_github_trigger(Some(DraftGitHubTrigger {
            events: vec!["pull_request.opened".into()],
            repositories: vec![],
            changes_only: true,
            branches: vec!["main".into()],
            workflows: vec!["CI".into()],
            message: "Review it".into(),
        }))
        .unwrap();
        assert!(!pr_only.changes_only);
        assert!(pr_only.branches.is_empty() && pr_only.workflows.is_empty());

        assert!(
            normalize_github_trigger(Some(DraftGitHubTrigger {
                events: vec!["push".into()],
                repositories: vec![],
                changes_only: false,
                branches: vec![],
                workflows: vec![],
                message: "x".into(),
            }))
            .is_none()
        );
        assert!(
            normalize_github_trigger(Some(DraftGitHubTrigger {
                events: vec!["pull_request.opened".into()],
                repositories: vec![],
                changes_only: false,
                branches: vec![],
                workflows: vec![],
                message: "  ".into(),
            }))
            .is_none()
        );
    }

    #[test]
    fn oauth_presets_act_as_the_service_identity() {
        assert_eq!(
            acts_as_for(McpServerAuthMode::OAuth),
            McpServerActsAs::Service
        );
        assert_eq!(
            acts_as_for(McpServerAuthMode::ApiKey),
            McpServerActsAs::None
        );
        assert_eq!(acts_as_for(McpServerAuthMode::None), McpServerActsAs::None);
    }
}

//! Fixers: one-click repairs declared on a health issue
//! (knowledge/operations/health-issues.md, "Fixers").
//!
//! A fixer resolves the condition an issue reports, or prepares the change and
//! sends the person to the one step only they can take (approving it in the
//! provider). It never declares success itself: only a fresh check resolves an
//! issue, as before.
//!
//! Decisions:
//! - The server decides which fixers an issue offers, and runs only those. A
//!   request names a fixer kind, never a URL or a command, and a consent URL is
//!   built here from the provider's fixed origin.
//! - Slack apps Everruns created reuse the install flow
//!   (`channels::slack::install::begin_reconsent`): scopes are added to the
//!   exported manifest and Slack's OAuth consent returns to the issue.
//! - Slack apps configured by hand are fixable when the organization's
//!   connected workspace can edit them, which is the usual case: the person
//!   who made the app is a collaborator and connected the workspace. The app is
//!   found from its own bot token (`auth.test` gives the bot, `bots.info` the
//!   app), and the person reinstalls it from Slack's install page. Reinstalling
//!   keeps the bot token, so nothing is pasted back. When Slack refuses, the
//!   manual steps remain.
//! - Pausing a runaway schedule is the same action the finding offers under
//!   `/actions`, declared here so the Health card shows it as a fix too.
//! - A model missing from its provider's last model list offers that
//!   provider's model sync. The list can be old, and the sync is what tells
//!   "listed after all" (the issue resolves) from "really gone" (it stays) from
//!   "the provider cannot be reached" (the sync's own error is returned, such
//!   as a rejected key).
//! - Each run is recorded once per episode in `finding_actions`, so an issue
//!   shows who fixed it and when.
use super::{
    background_work,
    commands::{issue, present},
    self_healing,
    types::HealthIssue,
};
use crate::domains::agent_channels::ingress::row_to_ingress;
use crate::domains::agent_channels::record::slack_channel::{SlackChannelConfig, slack_bot_scopes};
use crate::domains::agent_channels::record::slack_provisioning::SlackProvisioningError;
use crate::domains::agents::AGENT_MANAGE;
use crate::domains::common::*;
use crate::storage::HealthIssueRow;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use utoipa::ToSchema;
use uuid::Uuid;

/// Add the missing bot permissions to a Slack app and have it approved.
pub const SLACK_ADD_PERMISSIONS: &str = "slack.add_permissions";
/// Reconnect a Slack app whose credentials Slack rejected.
pub const SLACK_RECONNECT: &str = "slack.reconnect";
/// Pause a schedule whose runs keep ending the same way.
pub const SCHEDULE_PAUSE: &str = "schedule.pause";
/// Read a provider's model list again, for a model missing from the last one.
pub const MODELS_SYNC: &str = "models.sync";

const SLACK_API: &str = "https://slack.com/api";
const SLACK_AUTH_FAILURES: &[&str] = &[
    "invalid_auth",
    "not_authed",
    "token_revoked",
    "token_expired",
    "account_inactive",
];

/// A repair Everruns can make for an issue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct IssueFixer {
    /// What it does: `slack.add_permissions`, `slack.reconnect` or `schedule.pause`.
    #[schema(example = "slack.add_permissions")]
    pub kind: String,
    /// Button text.
    #[schema(example = "Fix in Slack")]
    pub label: String,
    /// What happens when it runs, step by step, in plain words.
    pub steps: Vec<String>,
    /// `in_place` when running it is the whole fix; `consent` when the person
    /// then approves the change in the provider.
    #[schema(example = "consent")]
    pub effect: String,
    /// Whether the fix can be undone from the finding.
    pub undoable: bool,
    /// Whether the provider may ask a workspace administrator to approve.
    pub needs_provider_admin: bool,
    /// Why it cannot run now, and what unlocks it. Absent when it can run.
    pub unavailable_reason: Option<String>,
}

/// The result of running a fixer.
#[derive(Debug, Serialize, ToSchema)]
pub struct IssueFixOutcome {
    /// `done` (the change is made and the issue was checked again) or
    /// `consent` (approve the change at `consent_url`, then come back).
    #[schema(example = "consent")]
    pub status: String,
    /// The provider page that approves the change. Built by the server on the
    /// provider's own origin.
    pub consent_url: Option<String>,
    /// Whether approving returns the browser to the issue by itself. When
    /// false the person comes back on their own and the page checks again.
    pub returns_automatically: bool,
    /// The issue after the fix.
    pub issue: HealthIssue,
}

/// Run one of the fixers an issue offers.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RunIssueFixerRequest {
    /// The fixer to run, as the issue's `fixers` list names it.
    #[schema(example = "slack.add_permissions")]
    pub kind: String,
}

/// Run a fixer on a health issue.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RunIssueFixer {
    /// The issue.
    pub issue_id: Uuid,
    #[serde(flatten)]
    pub req: RunIssueFixerRequest,
}

/// Where a Slack app came from.
enum SlackOrigin {
    /// Created by this deployment; its consent returns to Everruns.
    Everruns,
    /// Configured by hand with a pasted manifest and token.
    ByHand,
}

/// What the fixers need to know about an issue's Slack channel.
struct SlackChannel {
    origin: SlackOrigin,
    config: SlackChannelConfig,
}

async fn slack_channel(
    ctx: &Ctx,
    row: &HealthIssueRow,
) -> Result<Option<SlackChannel>, CommandError> {
    let Some(public_id) = row.channel_public_id.as_deref() else {
        return Ok(None);
    };
    let Some(channel) = ctx.db.get_ingress_channel_by_public_id(public_id).await? else {
        return Ok(None);
    };
    if channel.channel_type != "slack" || channel.org_id != ctx.org_id() {
        return Ok(None);
    }
    let (_, channel) = row_to_ingress(ctx.encryption.as_ref(), channel)?;
    let Ok(config) = serde_json::from_value::<SlackChannelConfig>(channel.channel_config) else {
        return Ok(None);
    };
    if config.bot_token.is_empty() {
        return Ok(None);
    }
    let origin = match config.provisioned_app {
        Some(_) => SlackOrigin::Everruns,
        None => SlackOrigin::ByHand,
    };
    Ok(Some(SlackChannel { origin, config }))
}

/// Why the organization cannot edit Slack apps in this workspace, if it cannot.
async fn workspace_blocker(
    ctx: &Ctx,
    team_id: Option<&str>,
) -> Result<Option<String>, CommandError> {
    if ctx.slack_provisioner.is_none() {
        return Ok(Some(
            "This deployment cannot edit Slack apps. Use the manual steps below.".into(),
        ));
    }
    let connections = ctx.db.list_org_slack_connections(ctx.org_id()).await?;
    let connection = connections
        .iter()
        .find(|c| team_id.is_none() || c.team_id.as_deref() == team_id);
    Ok(match connection {
        None => Some(
            "Connect this Slack workspace to the organization, as someone who can edit the app in Slack, and Everruns can fix it."
                .into(),
        ),
        Some(c) if c.state == "reconnect_required" => Some(
            "Reconnect this Slack workspace in the organization's Slack settings, and Everruns can fix it."
                .into(),
        ),
        Some(_) => None,
    })
}

fn slack_permission_steps(agent: &str, scopes: &[String], origin: &SlackOrigin) -> Vec<String> {
    let scopes = scopes.join(", ");
    match origin {
        SlackOrigin::Everruns => vec![
            format!("Everruns adds {scopes} to the {agent} app in Slack. Its other settings stay as they are."),
            "Slack opens so you can approve the new permission. Your workspace may need an administrator to approve it.".into(),
            "You come back here and Everruns checks the app again.".into(),
        ],
        SlackOrigin::ByHand => vec![
            format!("Everruns adds {scopes} to the {agent} app, using this organization's connection to the Slack workspace. Its other settings stay as they are."),
            "Slack opens the app's Install App page. Reinstall it there. The bot token stays the same, so nothing needs pasting back.".into(),
            "Come back here and Everruns checks the app again.".into(),
        ],
    }
}

/// The fixers an issue offers now. Empty when nothing can be repaired from
/// Everruns, or the issue is resolved.
pub async fn fixers_for(ctx: &Ctx, row: &HealthIssueRow) -> Result<Vec<IssueFixer>, CommandError> {
    if !matches!(row.status.as_str(), "open" | "needs_check") {
        return Ok(Vec::new());
    }
    if row.code == background_work::SCHEDULE_RUNAWAY {
        let actions = self_healing::actions_for(ctx, row).await?;
        return Ok(actions
            .available
            .iter()
            .filter(|a| a.kind == self_healing::PAUSE_SCHEDULE)
            .map(|_| IssueFixer {
                kind: SCHEDULE_PAUSE.into(),
                label: "Pause schedule".into(),
                steps: vec![
                    "Everruns pauses the schedule, so no more runs start.".into(),
                    "Undo resumes it whenever you are ready.".into(),
                ],
                effect: "in_place".into(),
                undoable: true,
                needs_provider_admin: false,
                unavailable_reason: None,
            })
            .collect());
    }
    if super::model_health::owns(&row.code) {
        return Ok(model_sync_fixer(ctx, row).await?.into_iter().collect());
    }
    if row.code != "slack.permissions" {
        return Ok(Vec::new());
    }
    let Some(slack) = slack_channel(ctx, row).await? else {
        return Ok(Vec::new());
    };
    let agent = row.agent_name.as_deref().unwrap_or("agent");
    let team = slack
        .config
        .provisioned_app
        .as_ref()
        .and_then(|p| p.team_id.as_deref())
        .or(slack.config.team_id.as_deref());
    let rejected = row
        .error_code
        .as_deref()
        .is_some_and(|code| SLACK_AUTH_FAILURES.contains(&code));
    if rejected {
        // A hand-made app's revoked token can only be replaced by its owner.
        return Ok(match slack.origin {
            SlackOrigin::Everruns => vec![IssueFixer {
                kind: SLACK_RECONNECT.into(),
                label: "Reconnect in Slack".into(),
                steps: vec![
                    format!("Slack opens so you can approve the {agent} app again."),
                    "You come back here and Everruns checks the app again.".into(),
                ],
                effect: "consent".into(),
                undoable: false,
                needs_provider_admin: true,
                unavailable_reason: workspace_blocker(ctx, team).await?,
            }],
            SlackOrigin::ByHand => Vec::new(),
        });
    }
    if row.missing_scopes.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![IssueFixer {
        kind: SLACK_ADD_PERMISSIONS.into(),
        label: "Fix in Slack".into(),
        steps: slack_permission_steps(agent, &row.missing_scopes, &slack.origin),
        effect: "consent".into(),
        undoable: false,
        needs_provider_admin: true,
        unavailable_reason: workspace_blocker(ctx, team).await?,
    }])
}

#[command(
    name = "run_health_issue_fixer",
    category = "health_issues",
    tag = "health-issues",
    summary = "Fix a health issue",
    description = "Run a fixer the issue offers, such as adding missing Slack permissions to the agent's app. A fix that needs approval in the provider returns the page to approve it on; the issue resolves only when a fresh check confirms it.",
    method = "POST",
    cli = CliRoute::new(&["health-issues"], "fix").with_examples(&[CliExample::new("Add missing Slack permissions to an agent's app", "everruns health-issues fix --issue-id 550e8400-e29b-41d4-a716-446655440000 --kind slack.add_permissions")]),
    path = "/v1/health-issues/{issue_id}/fix",
    policy = AGENT_MANAGE,
    request_body(RunIssueFixerRequest),
    http = plain,
)]
impl Command for RunIssueFixer {
    type Output = IssueFixOutcome;

    async fn execute(self, ctx: &Ctx) -> Result<IssueFixOutcome, CommandError> {
        run_fixer(ctx, self.issue_id, self.req.kind.trim(), SLACK_API).await
    }
}

/// [`RunIssueFixer`] with the Slack Web API base as a parameter, for tests.
pub(super) async fn run_fixer(
    ctx: &Ctx,
    issue_id: Uuid,
    kind: &str,
    slack_api: &str,
) -> Result<IssueFixOutcome, CommandError> {
    {
        let row = issue(ctx, issue_id).await?;
        let offered = fixers_for(ctx, &row).await?;
        let Some(fixer) = offered.iter().find(|f| f.kind == kind) else {
            return Err(CommandError::conflict(
                "That fix is not available for this issue",
            ));
        };
        if let Some(reason) = &fixer.unavailable_reason {
            return Err(CommandError::conflict(reason.clone()));
        }
        let (consent_url, returns_automatically) = match kind {
            SCHEDULE_PAUSE => {
                self_healing::pause_from_finding(ctx, &row).await?;
                (None, false)
            }
            SLACK_ADD_PERMISSIONS | SLACK_RECONNECT => slack_fix(ctx, &row, slack_api).await?,
            MODELS_SYNC => {
                sync_models(ctx, &row).await?;
                (None, false)
            }
            _ => {
                return Err(CommandError::conflict(
                    "That fix is not available for this issue",
                ));
            }
        };
        if kind != SCHEDULE_PAUSE {
            // Pausing records itself as the finding's action, with undo.
            ctx.db
                .record_finding_action(
                    ctx.org_id(),
                    row.id,
                    row.episode_id,
                    kind,
                    row.channel_public_id.as_deref().unwrap_or_default(),
                    ctx.caller.user_id,
                )
                .await?;
        }
        let issue = present(ctx, issue(ctx, issue_id).await?).await?;
        Ok(IssueFixOutcome {
            status: if consent_url.is_some() {
                "consent"
            } else {
                "done"
            }
            .into(),
            consent_url,
            returns_automatically,
            issue,
        })
    }
}

/// The provider a model issue's listing finding is about, when it is one.
fn listing_provider(row: &HealthIssueRow) -> Option<(Uuid, String)> {
    let evidence = row.evidence.as_ref()?;
    if evidence["reason"].as_str() != Some("no_longer_listed") {
        return None;
    }
    let provider_id = evidence["provider_id"].as_str()?.parse().ok()?;
    let name = evidence["provider"].as_str().unwrap_or("the provider");
    Some((provider_id, name.to_string()))
}

async fn model_sync_fixer(
    ctx: &Ctx,
    row: &HealthIssueRow,
) -> Result<Option<IssueFixer>, CommandError> {
    let Some((provider_id, name)) = listing_provider(row) else {
        return Ok(None);
    };
    let Some(provider) = ctx.db.get_provider(ctx.org_id(), provider_id).await? else {
        return Ok(None);
    };
    let unavailable_reason = if ctx.model_sync_service.is_none() {
        Some(
            "This deployment cannot sync models from here. Use Sync models on the provider's page."
                .into(),
        )
    } else if !provider.api_key_set && provider.provider_type != "chatgpt" {
        Some(format!(
            "{name} has no API key, so its model list cannot be read. Add one on the provider's page."
        ))
    } else {
        None
    };
    Ok(Some(IssueFixer {
        kind: MODELS_SYNC.into(),
        label: format!("Sync {name} models"),
        steps: vec![
            format!("Everruns reads {name}'s current model list."),
            format!("If {name} still offers the model, the issue clears."),
            format!(
                "If {name} cannot be reached, for example because it rejects the key, you see its error."
            ),
        ],
        effect: "in_place".into(),
        undoable: false,
        needs_provider_admin: false,
        unavailable_reason,
    }))
}

async fn sync_models(ctx: &Ctx, row: &HealthIssueRow) -> Result<(), CommandError> {
    let (provider_id, name) = listing_provider(row)
        .ok_or_else(|| CommandError::conflict("That fix is not available for this issue"))?;
    let sync = ctx
        .model_sync_service
        .as_ref()
        .ok_or_else(|| CommandError::conflict("This deployment cannot sync models from here"))?;
    // The sync rechecks the organization's models, which resolves the issue
    // when the provider lists the model.
    match sync.sync_provider(ctx.org_id(), provider_id).await {
        Ok(crate::domains::models::SyncResult::Failed { error }) => Err(CommandError::conflict(
            format!("{name} could not be reached: {error}"),
        )),
        Ok(_) => Ok(()),
        Err(error) => Err(CommandError::conflict(format!(
            "{name}'s models could not be synced: {error}"
        ))),
    }
}

async fn slack_fix(
    ctx: &Ctx,
    row: &HealthIssueRow,
    slack_api: &str,
) -> Result<(Option<String>, bool), CommandError> {
    let provisioner = ctx
        .slack_provisioner
        .clone()
        .ok_or_else(|| CommandError::conflict("This deployment cannot edit Slack apps"))?;
    let slack = slack_channel(ctx, row)
        .await?
        .ok_or_else(|| CommandError::conflict("This Slack channel is no longer configured"))?;
    let channel = row.channel_public_id.as_deref().unwrap_or_default();
    match slack.origin {
        SlackOrigin::Everruns => {
            // The public API base the app's OAuth redirect names; the REST
            // and MCP states set it from the auth config.
            let url = crate::channels::slack::install::begin_reconsent(
                &ctx.db,
                ctx.encryption.as_ref(),
                provisioner.as_ref(),
                &ctx.github_apps.api_base_url,
                ctx.org_id(),
                channel,
                row.id,
            )
            .await
            .map_err(reconsent_error)?;
            Ok((Some(url), true))
        }
        SlackOrigin::ByHand => {
            let app_id = slack_app_id(slack_api, &slack.config.bot_token)
                .await
                .map_err(|e| e.into_command_error())?;
            provisioner
                .update_permissions(
                    ctx.org_id(),
                    slack.config.team_id.as_deref(),
                    &app_id,
                    &slack_bot_scopes(slack.config.agent_surface_enabled),
                )
                .await
                .map_err(by_hand_error)?;
            Ok((Some(install_page(&app_id)?), false))
        }
    }
}

/// Slack's page for (re)installing an app to its workspace.
pub fn install_page(app_id: &str) -> Result<String, CommandError> {
    // App ids are `A` plus upper-case letters and digits; anything else is
    // not one and must not end up in a URL.
    if app_id.len() < 2
        || !app_id.starts_with('A')
        || !app_id
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return Err(CommandError::unprocessable(
            "Slack returned an unexpected app id",
        ));
    }
    Ok(format!(
        "https://api.slack.com/apps/{app_id}/install-on-team"
    ))
}

fn reconsent_error(error: crate::channels::slack::install::ReconsentError) -> CommandError {
    use crate::channels::slack::install::ReconsentError as E;
    match error {
        E::NotFound => CommandError::not_found("Slack channel"),
        E::NotProvisioned => CommandError::conflict("Everruns did not create this Slack app"),
        E::Provisioning(error) => provisioning_error(error),
        E::Internal(error) => CommandError::internal(error),
    }
}

fn provisioning_error(error: SlackProvisioningError) -> CommandError {
    match error {
        SlackProvisioningError::Unreachable(_) => {
            CommandError::unavailable("Could not reach Slack. Try again in a moment.")
        }
        SlackProvisioningError::Rejected(code) => {
            tracing::warn!(slack_error = %code, "Slack refused a health fix");
            CommandError::unprocessable(format!("Slack refused the change: {code}")).with_code(code)
        }
        SlackProvisioningError::RejectedWithProblems { code, problems } => {
            tracing::warn!(slack_error = %code, ?problems, "Slack refused a health fix");
            CommandError::unprocessable(format!(
                "Slack refused the change: {code} ({})",
                problems.join("; ")
            ))
            .with_code(code)
        }
        other => CommandError::conflict(other.to_string()),
    }
}

/// A hand-made app the connection cannot edit answers with one of these.
fn by_hand_error(error: SlackProvisioningError) -> CommandError {
    match &error {
        SlackProvisioningError::Rejected(code)
            if matches!(
                code.as_str(),
                "app_not_found" | "invalid_app" | "invalid_app_id" | "not_allowed" | "no_permission" | "not_authorized"
            ) =>
        {
            CommandError::conflict(
                "Slack would not let this organization's workspace connection edit the app. Use the manual steps, or connect the workspace as someone who can edit the app.",
            )
            .with_code(code.clone())
        }
        _ => provisioning_error(error),
    }
}

/// Why the app behind a bot token could not be found.
#[derive(Debug, PartialEq)]
pub enum AppLookupError {
    /// Slack answered, and refused: the token is not usable.
    Rejected(String),
    /// Slack could not be reached or answered something unexpected.
    Unavailable,
}

impl AppLookupError {
    fn into_command_error(self) -> CommandError {
        match self {
            AppLookupError::Rejected(code) if code == "missing_scope" => CommandError::conflict(
                "Slack would not say which app this bot belongs to, because the bot lacks the users:read permission. Use the manual steps.",
            )
            .with_code(code),
            AppLookupError::Rejected(code) => CommandError::unprocessable(format!(
                "Slack would not say which app this bot belongs to: {code}"
            ))
            .with_code(code),
            AppLookupError::Unavailable => {
                CommandError::unavailable("Could not reach Slack. Try again in a moment.")
            }
        }
    }
}

/// The Slack app (`A…`) a bot token belongs to: `auth.test` names the bot,
/// `bots.info` its app. Both answer with the token's own permissions.
pub async fn slack_app_id(api_base: &str, bot_token: &str) -> Result<String, AppLookupError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| AppLookupError::Unavailable)?;
    let call = |method: &str, form: Vec<(&'static str, String)>| {
        let request = client
            .post(format!("{}/{method}", api_base.trim_end_matches('/')))
            .bearer_auth(bot_token)
            .form(&form);
        async move {
            let body: serde_json::Value = request
                .send()
                .await
                .map_err(|_| AppLookupError::Unavailable)?
                .json()
                .await
                .map_err(|_| AppLookupError::Unavailable)?;
            if body["ok"] != true {
                return Err(match body["error"].as_str() {
                    Some(code) => AppLookupError::Rejected(code.to_string()),
                    None => AppLookupError::Unavailable,
                });
            }
            Ok(body)
        }
    };
    let auth = call("auth.test", Vec::new()).await?;
    let bot_id = auth["bot_id"]
        .as_str()
        .ok_or(AppLookupError::Unavailable)?
        .to_string();
    let info = call("bots.info", vec![("bot", bot_id)]).await?;
    info["bot"]["app_id"]
        .as_str()
        .map(str::to_string)
        .ok_or(AppLookupError::Unavailable)
}

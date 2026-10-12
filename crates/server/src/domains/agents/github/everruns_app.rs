// An agent's explicit choice of the shared Everruns App (D3 in
// knowledge/integrations/github-model.md).
//
// Decision: an org admin links shared App installations to the org
// (Settings > Organization, `api::user_connections::github_org`); an agent then
// chooses one of them on its GitHub card. The choice is a `github` row on the
// agent's service virtual user with `connection_type = everruns_app`, the
// link's `installation_id`, and `provider_metadata.org_connection_id` naming
// the link. Tokens mint with the deployment's shared key (`GitHubIdentityKind`).
//
// Decision: the choice is offered only to an agent with no GitHub identity of
// its own. An agent whose own App exists (installed or not) must disconnect or
// start over first, so choosing never silently replaces the agent's bot, and
// nothing ever falls back to the shared App on its own.
//
// Decision: writing the choice needs both agent management and identity
// management (the agent's GitHub identity lives on its service virtual user),
// and the link must belong to the caller's org.

use super::*;
use crate::github_apps::EVERRUNS_APP_CONNECTION_TYPE;
use crate::storage::{CreateVirtualUserConnectionRow, VirtualUserConnectionRow};
use everruns_core::{Permission, Policy, Rule};

/// Manage which GitHub identity an agent acts as.
pub const AGENT_GITHUB_IDENTITY_MANAGE: Policy = Policy {
    id: "agent.github_identity.manage",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgVirtualUsersManage),
    ],
};

/// A shared Everruns App installation the org links, as an agent can choose it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, ToSchema)]
pub struct EverrunsAppInstallation {
    /// The org link's connection id; pass it to choose this installation.
    pub connection_id: Uuid,
    /// GitHub account the installation is on.
    pub account: String,
    /// GitHub's account type: `User` or `Organization`, when known.
    pub account_type: Option<String>,
    /// `all` or `selected`, when known.
    pub repository_selection: Option<String>,
}

impl EverrunsAppInstallation {
    pub(crate) fn from_link(row: &VirtualUserConnectionRow) -> Self {
        let metadata = |key: &str| {
            row.provider_metadata
                .as_ref()
                .and_then(|m| m.get(key))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        Self {
            connection_id: row.id,
            account: row
                .provider_username
                .clone()
                .or_else(|| row.name.clone())
                .unwrap_or_default(),
            account_type: metadata("account_type"),
            repository_selection: metadata("repository_selection"),
        }
    }
}

/// The installations an agent in `org_id` can choose. Empty when the
/// deployment has no shared App, since nothing could mint for them.
pub(crate) async fn choosable_installations(
    db: &StorageBackend,
    github: &GitHubAppContext,
    org_id: i64,
) -> Result<Vec<EverrunsAppInstallation>, CommandError> {
    if github.shared_app_slug.is_none() {
        return Ok(Vec::new());
    }
    Ok(db
        .list_everruns_app_links(org_id)
        .await?
        .iter()
        .map(EverrunsAppInstallation::from_link)
        .collect())
}

/// Which org link to act through.
#[derive(Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct UseEverrunsAppRequest {
    /// Connection id of one of the org's linked installations, from the
    /// agent's GitHub status (`everruns_app_installations`).
    pub connection_id: Uuid,
}

/// Make an agent act as the shared Everruns App through one org link.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UseAgentEverrunsApp {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
    #[serde(flatten)]
    pub req: UseEverrunsAppRequest,
}

#[command(
    name = "use_agent_everruns_app",
    category = "agents",
    description = "Make an agent act on GitHub as the shared Everruns App, through one installation an org admin linked in Settings > Organization. Only for an agent without a GitHub App of its own; disconnect that first.",
    method = "POST",
    path = "/v1/agents/{agent_id}/github/everruns-app",
    policy = AGENT_GITHUB_IDENTITY_MANAGE,
    cli = CliRoute::new(&["agents", "github"], "use-everruns-app").with_examples(&[CliExample::new("Let an agent act as the Everruns app through the org's acme installation", "everruns agents github use-everruns-app --agent-id agent_01h9 --connection-id 0190f5a2-7c1e-7a3b-9d2e-4f5a6b7c8d9e --reason 'Use the shared app for the reviewer'")]),
    http = plain,
    request_body(UseEverrunsAppRequest),
    responses(
        (status = 404, description = "Agent or linked installation not found"),
        (status = 409, description = "The agent has a GitHub App of its own"),
        (status = 503, description = "The shared Everruns App is not configured"),
    ),
)]
impl Command for UseAgentEverrunsApp {
    type Output = AgentGitHubStatus;

    async fn execute(self, ctx: &Ctx) -> Result<AgentGitHubStatus, CommandError> {
        if ctx.github_apps.shared_app_slug.is_none() {
            return Err(CommandError::unavailable(
                "The shared Everruns GitHub App is not configured on this deployment",
            ));
        }
        let org_id = ctx.org_id();
        let agent = load_agent(ctx, &self.agent_id).await?;
        // The link must be this org's; another org's id reads as not found.
        let link = ctx
            .db
            .get_everruns_app_link(org_id, self.req.connection_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Linked Everruns GitHub App installation"))?;
        let installation_id = link
            .installation_id
            .ok_or_else(|| CommandError::not_found("Linked Everruns GitHub App installation"))?;

        let (identity_id, _) = ensure_identity_for_agent(&ctx.db, org_id, &agent).await?;
        if ctx
            .db
            .get_github_app_for_identity(org_id, identity_id)
            .await?
            .is_some()
        {
            return Err(CommandError::conflict(
                "This agent has a GitHub App of its own. Disconnect it (or start over) before \
                 choosing the Everruns app",
            ));
        }

        let installation = EverrunsAppInstallation::from_link(&link);
        let mut metadata = serde_json::json!({ "org_connection_id": link.id });
        if let Some(account_type) = &installation.account_type {
            metadata["account_type"] = account_type.clone().into();
        }
        if let Some(selection) = &installation.repository_selection {
            metadata["repository_selection"] = selection.clone().into();
        }
        ctx.db
            .upsert_virtual_user_connection_for_active_agent(
                org_id,
                agent.id,
                CreateVirtualUserConnectionRow {
                    virtual_user_id: identity_id,
                    provider: GITHUB_PROVIDER.to_string(),
                    connection_type: EVERRUNS_APP_CONNECTION_TYPE.to_string(),
                    provider_user_id: link.provider_user_id.clone(),
                    provider_username: link.provider_username.clone(),
                    access_token_encrypted: None,
                    refresh_token_encrypted: None,
                    scopes: link.scopes.clone(),
                    expires_at: None,
                    installation_id: Some(installation_id),
                    provider_metadata: Some(metadata),
                },
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Agent"))?;

        // The agent row may have just gained its identity.
        let agent = load_agent(ctx, &self.agent_id).await?;
        status_for_agent(&ctx.db, org_id, &agent, &ctx.github_apps).await
    }
}

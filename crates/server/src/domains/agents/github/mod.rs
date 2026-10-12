// An agent's own GitHub App, as commands: status, connect, disconnect.
//
// Decision: one implementation for every surface. The agent page's REST routes
// (`/v1/agents/{agent_id}/github`, `.../github/connect`) are these commands
// served by the generic HTTP adapter, the `everruns agents github` CLI and MCP
// `execute` run the same commands, and the identity-scoped routes in
// `api::github_apps` (`/v1/virtual-users/{id}/connections/github/app`) call the
// functions below. Only the GitHub redirects (manifest and setup callbacks)
// stay hand-written: they are browser redirects that carry none of our auth.
//
// Decision: `connect` cannot create an App outside a browser. GitHub creates it
// from a manifest the browser form-posts and shows a confirmation page, so the
// `create_app` answer carries `next_step` pointing at the web UI for a CLI or
// MCP caller. An App that already exists answers `install`, a URL any browser
// can open.
//
// See `crate::github_apps` for the flow and THREAT[TM-GHAPP-*].

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::AGENT_VIEW;
use crate::domains::common::*;
use crate::domains::virtual_users::VIRTUAL_USER_MANAGE;
use crate::domains::virtual_users::lifecycle::ensure_identity_for_agent;
use crate::github_apps::{
    GITHUB_APP_CONNECTION_TYPE, GITHUB_PROVIDER, GitHubAppApi, GitHubAppContext, ManifestInput,
    SetupState, app_credentials, build_manifest, default_app_name, install_url,
    manifest_form_action, missing_permissions,
};
use crate::kernel_imports::contracts::typed_id::VirtualUserId;
use crate::storage::{AgentRow, EncryptionService, StorageBackend};

// ============================================================================
// Types
// ============================================================================

/// GitHub as seen from an agent's page.
#[derive(Debug, Default, Serialize, PartialEq, ToSchema)]
pub struct AgentGitHubStatus {
    /// The agent's identity, once it has one; disconnect goes through it.
    pub identity_id: Option<String>,
    /// An installation is bound, so tools and triggers work.
    pub connected: bool,
    /// The App exists. Without `connected` it was never installed, or its
    /// installation was removed.
    pub app_created: bool,
    pub app_slug: Option<String>,
    pub app_url: Option<String>,
    /// GitHub account the App is installed on.
    pub account: Option<String>,
    /// `all` or `selected`.
    pub repository_selection: Option<String>,
    /// Permissions new Apps request that this installation lacks (`name:level`),
    /// e.g. `actions:read` for CI triggers on an App created before them.
    /// Empty when not connected.
    pub missing_permissions: Vec<String>,
}

/// Where a new App is created and where the browser lands afterwards.
#[derive(Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct BeginConnectRequest {
    /// GitHub organization that should own a new App. Omit for the personal
    /// account of whoever confirms it on GitHub. A private App can only be
    /// installed on the account that owns it.
    #[serde(default)]
    pub owner_org: Option<String>,
    /// UI path to return to when the flow ends.
    #[serde(default)]
    pub return_to: Option<String>,
}

/// What happens next.
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BeginConnectResponse {
    /// GitHub must create the App first. A browser posts `manifest` as the
    /// `manifest` form field to `action` (a top-level navigation, so GitHub's
    /// confirmation page is shown). Outside a browser, follow `next_step`.
    CreateApp {
        action: String,
        manifest: String,
        next_step: String,
    },
    /// The App exists; open `url` in a browser to install it or change which
    /// repositories it can reach.
    Install { url: String, app_slug: String },
}

/// What a disconnect did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitHubDisconnectOutcome {
    /// The installation was removed (and uninstalled on GitHub, best effort).
    /// The App stays, so connecting again reuses it.
    Disconnected,
    /// The App was never installed, so it was forgotten: the next connect
    /// creates a new one, for example in a GitHub organization.
    StartedOver,
}

/// Result of disconnecting an agent from GitHub.
#[derive(Debug, Serialize, ToSchema)]
pub struct AgentGitHubDisconnect {
    pub outcome: GitHubDisconnectOutcome,
}

const CREATE_APP_NEXT_STEP: &str = "Creating the GitHub App needs a browser: open the agent in the web UI, \
     select Integrations, and choose Connect GitHub on the GitHub card. Run this command again \
     afterwards to get an install link.";

// ============================================================================
// Shared with the identity-scoped routes
// ============================================================================

/// The status the agent page shows.
pub(crate) async fn status_for_agent(
    db: &StorageBackend,
    org_id: i64,
    agent: &AgentRow,
) -> Result<AgentGitHubStatus, CommandError> {
    let Some(identity_id) = agent.virtual_user_id else {
        return Ok(AgentGitHubStatus::default());
    };
    let app = db.get_github_app_for_identity(org_id, identity_id).await?;
    let connection = db
        .get_virtual_user_connection(identity_id, GITHUB_PROVIDER)
        .await?
        .filter(|c| c.connection_type == GITHUB_APP_CONNECTION_TYPE);
    let repository_selection = connection
        .as_ref()
        .and_then(|c| c.provider_metadata.as_ref())
        .and_then(|m| m.get("repository_selection"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Ok(AgentGitHubStatus {
        identity_id: Some(identity_id.to_string()),
        connected: connection.is_some(),
        app_created: app.is_some(),
        app_slug: app.as_ref().map(|app| app.slug.clone()),
        app_url: app.as_ref().map(|app| app.html_url.clone()),
        missing_permissions: connection
            .as_ref()
            .map(|c| missing_permissions(c.scopes.as_deref()))
            .unwrap_or_default(),
        account: connection.and_then(|c| c.provider_username),
        repository_selection,
    })
}

/// Start connecting an identity: a manifest to create its App, or the
/// existing App's install URL.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn begin_connect(
    db: &StorageBackend,
    github: &GitHubAppContext,
    encryption: Option<&EncryptionService>,
    org_id: i64,
    user_id: Option<Uuid>,
    identity_id: VirtualUserId,
    identity_name: &str,
    request: BeginConnectRequest,
) -> Result<BeginConnectResponse, CommandError> {
    let encryption =
        encryption.ok_or_else(|| CommandError::unavailable("Encryption not configured"))?;
    let user_id = user_id.ok_or_else(|| CommandError::forbidden("A signed-in user is required"))?;

    let existing = db.get_github_app_for_identity(org_id, identity_id).await?;
    let app_row_id = existing.as_ref().map_or_else(Uuid::new_v4, |app| app.id);
    let setup_state = SetupState::new(
        org_id,
        identity_id.uuid(),
        user_id,
        app_row_id,
        request.return_to,
    )
    .seal(encryption)
    .map_err(CommandError::internal)?;

    let web_url = &github.api.endpoints().web_url;
    if let Some(app) = existing {
        return Ok(BeginConnectResponse::Install {
            url: install_url(web_url, &app.slug, &setup_state),
            app_slug: app.slug,
        });
    }

    let name = default_app_name(identity_name);
    let manifest = build_manifest(&ManifestInput {
        name: &name,
        api_base_url: &github.api_base_url,
        frontend_url: &github.frontend_url,
        app_row_id,
    });
    Ok(BeginConnectResponse::CreateApp {
        action: manifest_form_action(web_url, request.owner_org.as_deref(), &setup_state),
        manifest: manifest.to_string(),
        next_step: CREATE_APP_NEXT_STEP.to_string(),
    })
}

/// Disconnect an identity from GitHub.
///
/// Uninstalls the App (best effort) and removes the identity's `github`
/// connection. The App itself stays so a reconnect reuses it; GitHub offers no
/// API to delete an App. With no installation, forgets the App instead so the
/// next connect creates a fresh one.
pub(crate) async fn disconnect_identity(
    db: &StorageBackend,
    github: &GitHubAppApi,
    encryption: Option<&EncryptionService>,
    org_id: i64,
    identity_id: VirtualUserId,
) -> Result<GitHubDisconnectOutcome, CommandError> {
    let connection = db
        .get_virtual_user_connection(identity_id, GITHUB_PROVIDER)
        .await?;
    let app = db.get_github_app_for_identity(org_id, identity_id).await?;

    if let (Some(app), Some(installation_id)) = (
        app.as_ref(),
        connection.as_ref().and_then(|c| c.installation_id),
    ) {
        match app_credentials(encryption, app) {
            Ok(credentials) => {
                if let Err(err) = github
                    .delete_installation(&credentials, installation_id)
                    .await
                {
                    tracing::warn!(error = %err, app_id = app.app_id, "GitHub uninstall failed");
                }
            }
            Err(err) => tracing::warn!(error = %err, "GitHub App key unavailable for uninstall"),
        }
    }

    // An App that was created but never installed has no connection. Removing
    // it lets the user start over, for example to create the App in an
    // organization instead of their personal account: a private App can only
    // be installed on the account that owns it. The App itself stays on GitHub
    // for the user to delete there; nothing here can reach it any more.
    if connection.is_none() {
        let app = app.ok_or_else(|| CommandError::not_found("Connection"))?;
        db.delete_github_app(app.id).await?;
        return Ok(GitHubDisconnectOutcome::StartedOver);
    }
    db.delete_virtual_user_connection(identity_id, GITHUB_PROVIDER)
        .await?;
    Ok(GitHubDisconnectOutcome::Disconnected)
}

async fn load_agent(ctx: &Ctx, agent_id: &str) -> Result<AgentRow, CommandError> {
    ctx.db
        .get_agent_by_public_id(ctx.org_id(), agent_id)
        .await?
        .filter(|agent| agent.status == "active")
        .ok_or_else(|| CommandError::not_found("Agent"))
}

// ============================================================================
// GetAgentGitHub
// ============================================================================

/// Show an agent's GitHub connection.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetAgentGitHub {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
}

#[command(
    name = "get_agent_github",
    category = "agents",
    description = "Show an agent's own GitHub App: whether it exists, whether it is installed, on which account, and which permissions it lacks.",
    method = "GET",
    path = "/v1/agents/{agent_id}/github",
    policy = AGENT_VIEW,
    cli = CliRoute::new(&["agents", "github"], "get").with_examples(&[CliExample::new("Check whether an agent's GitHub App is installed and on which account", "everruns agents github get --agent-id agent_01h9")]),
    http = plain,
    responses((status = 404, description = "Agent not found")),
)]
impl Command for GetAgentGitHub {
    type Output = AgentGitHubStatus;

    async fn execute(self, ctx: &Ctx) -> Result<AgentGitHubStatus, CommandError> {
        let agent = load_agent(ctx, &self.agent_id).await?;
        status_for_agent(&ctx.db, ctx.org_id(), &agent).await
    }
}

// ============================================================================
// ConnectAgentGitHub
// ============================================================================

/// Start connecting an agent to GitHub.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ConnectAgentGitHub {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
    #[serde(flatten)]
    pub req: BeginConnectRequest,
}

#[command(
    name = "connect_agent_github",
    category = "agents",
    description = "Start connecting an agent to GitHub through its own GitHub App. Returns `install` with a URL to open in a browser when the App exists, or `create_app` when GitHub must first create it, which needs the web UI (Agent > Integrations > Connect GitHub).",
    method = "POST",
    path = "/v1/agents/{agent_id}/github/connect",
    policy = VIRTUAL_USER_MANAGE,
    cli = CliRoute::new(&["agents", "github"], "connect").with_examples(&[CliExample::new("Get the link that installs an agent's GitHub App on more repositories", "everruns agents github connect --agent-id agent_01h9 --reason 'Let the reviewer reach the API repo'")]),
    http = plain,
    request_body(BeginConnectRequest),
    responses((status = 404, description = "Agent not found")),
)]
impl Command for ConnectAgentGitHub {
    type Output = BeginConnectResponse;

    async fn execute(self, ctx: &Ctx) -> Result<BeginConnectResponse, CommandError> {
        let agent = load_agent(ctx, &self.agent_id).await?;
        let (identity_id, _) = ensure_identity_for_agent(&ctx.db, ctx.org_id(), &agent).await?;
        let identity = ctx
            .db
            .get_virtual_user(ctx.org_id(), identity_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Virtual user"))?;
        begin_connect(
            &ctx.db,
            &ctx.github_apps,
            ctx.encryption.as_deref(),
            ctx.org_id(),
            ctx.caller.user_id,
            identity_id,
            &identity.name,
            self.req,
        )
        .await
    }
}

// ============================================================================
// DisconnectAgentGitHub
// ============================================================================

/// Disconnect an agent from GitHub, or start over with a new App.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DisconnectAgentGitHub {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
}

#[command(
    name = "disconnect_agent_github",
    category = "agents",
    description = "Disconnect an agent from GitHub: uninstall its GitHub App and remove the connection, keeping the App for a later reconnect. When the App was never installed, forget it instead, so the next connect creates a new one (for example in a GitHub organization).",
    method = "DELETE",
    path = "/v1/agents/{agent_id}/github",
    policy = VIRTUAL_USER_MANAGE,
    cli = CliRoute::new(&["agents", "github"], "disconnect").with_examples(&[
        CliExample::new("Uninstall an agent's GitHub App and remove its connection", "everruns agents github disconnect --agent-id agent_01h9 --reason 'Reviewer retired'"),
        CliExample::new("Start over after creating the App on the wrong account, before installing it", "everruns agents github disconnect --agent-id agent_01h9 --reason 'Recreate the App in the acme organization'"),
    ]),
    http = plain,
    responses((status = 404, description = "Agent or GitHub connection not found")),
)]
impl Command for DisconnectAgentGitHub {
    type Output = AgentGitHubDisconnect;

    async fn execute(self, ctx: &Ctx) -> Result<AgentGitHubDisconnect, CommandError> {
        let agent = load_agent(ctx, &self.agent_id).await?;
        let identity_id = agent
            .virtual_user_id
            .ok_or_else(|| CommandError::not_found("Connection"))?;
        let outcome = disconnect_identity(
            &ctx.db,
            &ctx.github_apps.api,
            ctx.encryption.as_deref(),
            ctx.org_id(),
            identity_id,
        )
        .await?;
        Ok(AgentGitHubDisconnect { outcome })
    }
}

#[cfg(test)]
mod tests;

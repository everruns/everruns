//! Continuous Agent source synchronization with opt-in conditional GitHub writes.
//! Source authority is task-local and scoped to one tenant and Agent, never a request flag.
mod accounts;
mod github;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
mod types;
mod write;
pub use accounts::{
    AgentSourceAccounts, ListAgentSourceAccounts, OrganizationSourceAccount, PersonalSourceAccount,
    SourceRepository,
};
pub use types::{AgentSourceStatus, SourceGitHubAccount, SourceMode};
#[cfg(test)]
mod write_tests;

use super::{AGENT_MANAGE, AGENT_VIEW, packages, queries};
use crate::domains::common::*;
use crate::storage::{AgentSourceRow, transaction};
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::{AgentId, VirtualUserId};
use everruns_core::Caller;
use everruns_core::agent_package::{AgentPackage, Format};
use github::SourceLocation;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

const READ_ONLY: &str = "This Agent's source is managed by GitHub. Edit the repository or disconnect source sync before editing";
tokio::task_local! { static APPLYING: (i64, Uuid); }

// THREAT[TM-GHAPP-013]: ownership bypass exists only in this exact task/tenant/Agent.
pub(crate) async fn ensure_writable(ctx: &Ctx, agent: AgentId) -> Result<(), CommandError> {
    ctx.db
        .lock_agent_definition(ctx.org_id(), agent.uuid())
        .await?;
    if APPLYING
        .try_with(|key| *key == (ctx.org_id(), agent.uuid()))
        .unwrap_or(false)
    {
        return Ok(());
    }
    if let Some(source) = ctx.db.get_agent_source(ctx.org_id(), agent.uuid()).await? {
        if source.mode != "two_way" {
            return Err(CommandError::conflict(READ_ONLY));
        }
        ctx.db
            .mark_agent_source_dirty(ctx.org_id(), agent.uuid())
            .await?;
    }
    Ok(())
}

/// Package-owned channel configuration is source-owned; publication and credentials stay local.
pub(crate) async fn ensure_channel_writable(
    ctx: &Ctx,
    agent: AgentId,
    config: &Value,
) -> Result<(), CommandError> {
    // An unnamed destination channel cannot be adopted by a package. Avoid
    // holding its Agent row while waiting for a local Slack credential edit.
    let Some(name) = config.get("package_name").and_then(Value::as_str) else {
        return Ok(());
    };
    ctx.db
        .lock_agent_definition(ctx.org_id(), agent.uuid())
        .await?;
    if APPLYING
        .try_with(|key| *key == (ctx.org_id(), agent.uuid()))
        .unwrap_or(false)
    {
        return Ok(());
    }
    if let Some(source) = ctx.db.get_agent_source(ctx.org_id(), agent.uuid()).await?
        && source
            .managed_channels
            .iter()
            .any(|channel| channel == name)
    {
        if source.mode != "two_way" {
            return Err(CommandError::conflict(READ_ONLY));
        }
        ctx.db
            .mark_agent_source_dirty(ctx.org_id(), agent.uuid())
            .await?;
    }
    Ok(())
}

/// A local package import can explicitly change source-owned channel intent.
/// Omitted destination bindings survive but stop participating in write-back.
pub(crate) async fn record_local_channels(
    ctx: &Ctx,
    agent: Uuid,
    names: &[String],
) -> Result<(), CommandError> {
    if APPLYING
        .try_with(|key| *key == (ctx.org_id(), agent))
        .unwrap_or(false)
    {
        return Ok(());
    }
    if let Some(source) = ctx.db.get_agent_source(ctx.org_id(), agent).await?
        && source.mode == "two_way"
    {
        ctx.db
            .set_agent_source_channels(ctx.org_id(), agent, names)
            .await?;
    }
    Ok(())
}

async fn agent(ctx: &Ctx, id: &str) -> Result<super::record::Agent, CommandError> {
    queries::resolve(&ctx.db, ctx.org_id(), id)
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))
}
async fn binding(ctx: &Ctx, id: &str) -> Result<AgentSourceRow, CommandError> {
    let a = agent(ctx, id).await?;
    ctx.db
        .get_agent_source(ctx.org_id(), a.internal_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Agent source"))
}
fn is_app_connection(connection: &crate::storage::VirtualUserConnectionRow) -> bool {
    matches!(
        connection.connection_type.as_str(),
        crate::github_apps::GITHUB_APP_CONNECTION_TYPE
            | crate::github_apps::EVERRUNS_APP_CONNECTION_TYPE
    )
}

/// The exact GitHub connection a source reads and writes through.
///
/// Decision (knowledge/runtime-resources/agent-sources.md): source sync is a
/// development-time use, so it never borrows the Agent's runtime identity. A
/// binding reads through the configuring person's own GitHub sign-in or an
/// organization link to the shared Everruns App. `Agent` only describes
/// bindings made before that split; they keep working until reconnected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SourceAccess {
    pub account: SourceAccountKind,
    pub identity: Uuid,
    pub connection: Uuid,
    pub installation: Option<i64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceAccountKind {
    Agent,
    Personal,
    Organization,
}
impl SourceAccountKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Personal => "personal",
            Self::Organization => "organization",
        }
    }
    fn parse(value: &str) -> Self {
        match value {
            "personal" => Self::Personal,
            "organization" => Self::Organization,
            _ => Self::Agent,
        }
    }
}
impl From<SourceGitHubAccount> for SourceAccountKind {
    fn from(value: SourceGitHubAccount) -> Self {
        match value {
            SourceGitHubAccount::Personal => Self::Personal,
            SourceGitHubAccount::Organization => Self::Organization,
        }
    }
}
impl SourceAccess {
    pub(super) fn of(row: &AgentSourceRow) -> Self {
        Self {
            account: SourceAccountKind::parse(&row.github_account),
            identity: row.identity_id,
            connection: row.connection_id,
            installation: row.installation_id,
        }
    }
}

/// The caller's own GitHub sign-in: a person connection, never an installation.
async fn personal_access(ctx: &Ctx) -> Result<SourceAccess, CommandError> {
    let user = ctx.caller.user_id.ok_or_else(|| {
        CommandError::bad_request("Sign in as a person to sync through your GitHub account")
    })?;
    let identity = ctx
        .db
        .default_virtual_user(ctx.org_id(), user)
        .await
        .map_err(|_| {
            CommandError::forbidden("Your account is not a member of this organization")
        })?;
    let connection = ctx
        .db
        .get_virtual_user_connection(identity.id, "github")
        .await?
        .filter(|c| !is_app_connection(c) && c.installation_id.is_none())
        .ok_or_else(|| CommandError::bad_request("Connect your GitHub account first"))?;
    Ok(SourceAccess {
        account: SourceAccountKind::Personal,
        identity: identity.id.uuid(),
        connection: connection.id,
        installation: None,
    })
}

/// The organization's link to the shared Everruns App for the repository's account.
async fn organization_access(ctx: &Ctx, repository: &str) -> Result<SourceAccess, CommandError> {
    let owner = repository.split_once('/').map(|(owner, _)| owner);
    let link = ctx
        .db
        .list_everruns_app_links(ctx.org_id())
        .await?
        .into_iter()
        .find(|link| {
            link.installation_id.is_some()
                && owner.is_some_and(|owner| {
                    link.provider_username
                        .as_deref()
                        .is_some_and(|login| login.eq_ignore_ascii_case(owner))
                })
        })
        .ok_or_else(|| {
            CommandError::bad_request(format!(
                "No organization GitHub account is connected for {}. An organization admin connects it",
                owner.unwrap_or(repository)
            ))
        })?;
    Ok(SourceAccess {
        account: SourceAccountKind::Organization,
        identity: link.virtual_user_id.uuid(),
        connection: link.id,
        installation: link.installation_id,
    })
}

/// Select the development account for a new or changed source. Without an
/// explicit choice the person's own GitHub answers first, then the org link.
async fn select_access(
    ctx: &Ctx,
    account: Option<SourceGitHubAccount>,
    repository: &str,
) -> Result<SourceAccess, CommandError> {
    match account {
        Some(SourceGitHubAccount::Personal) => personal_access(ctx).await,
        Some(SourceGitHubAccount::Organization) => organization_access(ctx, repository).await,
        None => match personal_access(ctx).await {
            Ok(access) => Ok(access),
            Err(_) => organization_access(ctx, repository).await.map_err(|_| {
                CommandError::bad_request(
                    "Connect your GitHub account, or ask an organization admin to connect the repository's GitHub account",
                )
            }),
        },
    }
}

async fn validate_connection(
    ctx: &Ctx,
    access: &SourceAccess,
    configured_by: Option<Uuid>,
) -> Result<(), CommandError> {
    // THREAT[TM-GHAPP-014]: never substitute a replacement installation or identity.
    let changed = || {
        CommandError::bad_request(
            "The source GitHub connection changed or was disconnected. Reconnect and select the source again",
        )
    };
    ctx.db
        .get_virtual_user(ctx.org_id(), VirtualUserId::from_uuid(access.identity))
        .await?
        .filter(|v| v.status == "active")
        .ok_or_else(|| CommandError::bad_request("Source GitHub identity is unavailable"))?;
    let current = ctx
        .db
        .get_virtual_user_connection_by_id(
            VirtualUserId::from_uuid(access.identity),
            access.connection,
            "github",
        )
        .await?
        .filter(|c| c.installation_id == access.installation)
        .ok_or_else(changed)?;
    let matches = match access.account {
        SourceAccountKind::Agent => is_app_connection(&current) && access.installation.is_some(),
        SourceAccountKind::Organization => {
            current.connection_type == crate::github_apps::EVERRUNS_APP_CONNECTION_TYPE
                && current.owner_scope == "organization"
        }
        SourceAccountKind::Personal => !is_app_connection(&current),
    };
    if !matches {
        return Err(changed());
    }
    // A personal source reads only through the configuring person's own sign-in.
    if access.account == SourceAccountKind::Personal {
        let person = match configured_by.or(ctx.caller.user_id) {
            Some(user) => ctx.db.default_virtual_user(ctx.org_id(), user).await.ok(),
            None => None,
        };
        if person.is_none_or(|p| p.id.uuid() != access.identity) {
            return Err(changed());
        }
    }
    Ok(())
}
pub(super) async fn token(
    ctx: &Ctx,
    access: &SourceAccess,
    configured_by: Option<Uuid>,
    repository: &str,
    permission: &str,
) -> Result<String, CommandError> {
    validate_connection(ctx, access, configured_by).await?;
    let request = everruns_contracts::runtime::GitHubAccess::new(
        everruns_core::McpServerActsAs::Service,
        &[("contents", permission)],
    )
    .for_repository(repository);
    let unavailable = || {
        if permission == "write" {
            CommandError::bad_request(
                "Two-way sync needs Contents write permission on this repository. Update the GitHub App and accept its installation permissions",
            )
        } else {
            CommandError::bad_request(
                "Could not authorize read access to the source repository. Check the GitHub connection",
            )
        }
    };
    let token = ctx
        .connection_resolver
        .as_ref()
        .ok_or_else(|| CommandError::unavailable("GitHub connection resolver is unavailable"))?
        .get_github_token_for_connection(ctx.org_id(), access.identity, access.connection, &request)
        .await
        .map_err(|_| unavailable())?
        .ok_or_else(unavailable)?;
    validate_connection(ctx, access, configured_by).await?;
    Ok(token)
}
fn digest(package: &AgentPackage) -> Result<String, CommandError> {
    let bytes = serde_json::to_vec(&package.canonical().map_err(packages::package_error)?)
        .map_err(|e| CommandError::internal(e.into()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
async fn current_digest(ctx: &Ctx, id: &str) -> Result<String, CommandError> {
    digest(&write::local_package(ctx, id).await?)
}
async fn validate(ctx: &Ctx, package: &AgentPackage, id: &str) -> Result<Value, CommandError> {
    let input = packages::PackageInput {
        content: package
            .to_string(Format::Json)
            .map_err(packages::package_error)?,
        file: None,
        format: Some("json".into()),
        target: Some(id.into()),
    };
    packages::ValidateAgentPackage(input).run(ctx).await
}
async fn apply(
    ctx: &Ctx,
    package: &AgentPackage,
    id: &str,
    uuid: Uuid,
) -> Result<(), CommandError> {
    let validation = validate(ctx, package, id).await?;
    if validation["valid"] != true {
        return Err(CommandError::bad_request(format!(
            "Source package validation failed: {}",
            validation["diagnostics"]
        )));
    }
    APPLYING
        .scope((ctx.org_id(), uuid), async {
            packages::ImportAgent::Package(packages::PackageInput {
                content: package
                    .to_string(Format::Json)
                    .map_err(packages::package_error)?,
                file: None,
                format: Some("json".into()),
                target: Some(id.into()),
            })
            .run(ctx)
            .await?;
            Ok(())
        })
        .await
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetAgentSource {
    pub agent_id: String,
}
#[command(name="get_agent_source",summary="Get Agent source status",category="agents",description="Show the Agent's continuous GitHub source binding and sync status.",method="GET",path="/v1/agents/{agent_id}/source",policy=AGENT_VIEW,http=plain,cli=CliRoute::new(&["agents", "source"], "get").with_examples(&[CliExample::new("See the applied GitHub revision and any sync error", "everruns agents source get --agent-id agent_01h9")]))]
impl Command for GetAgentSource {
    type Output = Option<AgentSourceStatus>;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let a = agent(ctx, &self.agent_id).await?;
        Ok(ctx
            .db
            .get_agent_source(ctx.org_id(), a.internal_id)
            .await?
            .map(Into::into))
    }
}

/// Repository location to browse before selecting an Agent source package.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SourceOptionsRequest {
    /// GitHub repository in owner/name form.
    #[schema(example = "acme/agents")]
    pub repository: String,
    /// Branch to browse; omitted uses the repository's default branch.
    #[serde(default)]
    #[schema(example = "main")]
    pub branch: Option<String>,
    /// Repository-relative directory to browse; empty selects the root.
    #[serde(default)]
    #[schema(example = "agents")]
    pub path: String,
    /// Development GitHub account to read through; omitted prefers the caller's own.
    #[serde(default)]
    pub github_account: Option<SourceGitHubAccount>,
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetAgentSourceOptions {
    pub agent_id: String,
    #[serde(flatten)]
    pub req: SourceOptionsRequest,
}
#[command(name="get_agent_source_options",summary="Browse GitHub source locations",category="agents",description="Browse GitHub branches and Agent package paths.",method="POST",path="/v1/agents/{agent_id}/source/options",policy=AGENT_VIEW,http=plain,request_body(SourceOptionsRequest),read_only=true,cli=CliRoute::new(&["agents", "source", "options"], "get").with_examples(&[CliExample::new("Find branches and package paths before choosing a source", "everruns agents source options get --agent-id agent_01h9 --repository acme/agents --branch main --path agents")]))]
impl Command for GetAgentSourceOptions {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        agent(ctx, &self.agent_id).await?;
        let repo = crate::domains::git_sources::normalize_github_repository(&self.req.repository)
            .map_err(CommandError::bad_request)?;
        let access = select_access(ctx, self.req.github_account, &repo).await?;
        let token = token(ctx, &access, None, &repo, "read").await?;
        let meta = github::metadata(ctx, &repo, &token).await?;
        let default_branch = meta["default_branch"]
            .as_str()
            .ok_or_else(|| CommandError::bad_request("Repository has no default branch"))?;
        let mut location = SourceLocation {
            repository: repo.clone(),
            branch: self.req.branch.unwrap_or_else(|| default_branch.into()),
            path: self.req.path,
        };
        location.validate()?;
        let branches = github::branches(ctx, &repo, &token).await?;
        let paths = github::browse(ctx, &location, &token).await?;
        Ok(
            json!({"default_branch":default_branch,"branch":location.branch,"branches":branches,"paths":paths}),
        )
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PreviewAgentSource {
    pub agent_id: String,
    #[serde(flatten)]
    pub req: SourceLocation,
    /// Development GitHub account to read through; omitted prefers the caller's own.
    #[serde(default)]
    pub github_account: Option<SourceGitHubAccount>,
}
#[command(name="preview_agent_source",summary="Preview Agent source changes",category="agents",description="Validate and compare a GitHub Agent package without changing the Agent.",method="POST",path="/v1/agents/{agent_id}/source/preview",policy=AGENT_VIEW,http=plain,request_body(SourceLocation),read_only=true,cli=CliRoute::new(&["agents", "source"], "preview").with_examples(&[CliExample::new("Review a repository package without changing the Agent", "everruns agents source preview --agent-id agent_01h9 --repository acme/agents --branch main --path agents/support")]))]
impl Command for PreviewAgentSource {
    type Output = Value;
    async fn execute(mut self, ctx: &Ctx) -> Result<Value, CommandError> {
        self.req.validate()?;
        agent(ctx, &self.agent_id).await?;
        let access = select_access(ctx, self.github_account, &self.req.repository).await?;
        let token = token(ctx, &access, None, &self.req.repository, "read").await?;
        let snapshot = github::snapshot(ctx, &self.req, &token).await?;
        let mut validation = validate(ctx, &snapshot.package, &self.agent_id).await?;
        validation["commit"] = json!(snapshot.commit);
        validation["agent_digest"] = json!(current_digest(ctx, &self.agent_id).await?);
        if validation["valid"] == true {
            let current = write::local_package(ctx, &self.agent_id).await?;
            validation["changes"] = serde_json::to_value(
                current
                    .diff(&snapshot.package)
                    .map_err(packages::package_error)?,
            )
            .map_err(|e| CommandError::internal(e.into()))?;
        }
        Ok(validation)
    }
}

/// Reviewed package location and concurrency tokens required to enable source sync.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ConnectSourceRequest {
    /// Synchronization direction; two_way requires repository Contents write access.
    #[serde(default)]
    #[schema(example = "two_way")]
    pub mode: SourceMode,
    /// Repository, branch and package path that were previewed.
    #[serde(flatten)]
    pub location: SourceLocation,
    /// Development GitHub account the source reads and writes through; omitted
    /// prefers the caller's own GitHub, then the organization's link.
    #[serde(default)]
    pub github_account: Option<SourceGitHubAccount>,
    /// Exact source commit returned by preview; a changed branch requires another preview.
    #[schema(example = "0123456789abcdef0123456789abcdef01234567")]
    pub commit: String,
    /// Agent package digest returned by preview, protecting concurrent local edits.
    #[schema(example = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")]
    pub agent_digest: String,
    /// Current binding generation when replacing a source; omit for an unbound Agent.
    #[serde(default)]
    #[schema(example = "018f30c8-c970-7cb5-a8c9-9348aa71ba8c")]
    pub expected_generation: Option<Uuid>,
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ConnectAgentSource {
    pub agent_id: String,
    #[serde(flatten)]
    pub req: ConnectSourceRequest,
}
#[command(name="connect_agent_source",summary="Connect continuous GitHub source sync",category="agents",description="Apply a reviewed GitHub package and enable continuous sync. Pull mode is read-only; two_way also publishes local edits using conditional commits.",method="PUT",path="/v1/agents/{agent_id}/source",policy=AGENT_MANAGE,http=plain,request_body(ConnectSourceRequest),cli=CliRoute::new(&["agents", "source"], "connect").with_examples(&[CliExample::new("Enable continuous sync using the commit and Agent digest returned by preview", "everruns agents source connect --agent-id agent_01h9 --repository acme/agents --branch main --path agents/support --commit 0123456789abcdef0123456789abcdef01234567 --agent-digest 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef --reason \"Deploy the reviewed GitHub package\"")]))]
impl Command for ConnectAgentSource {
    type Output = AgentSourceStatus;
    async fn execute(mut self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        self.req.location.validate()?;
        let a = agent(ctx, &self.agent_id).await?;
        queries::ensure_not_built_in(&ctx.db, ctx.org_id(), &self.agent_id, "modify").await?;
        if a.status != super::record::AgentStatus::Active {
            return Err(CommandError::bad_request(
                "Only active Agents can sync source",
            ));
        }
        let access =
            select_access(ctx, self.req.github_account, &self.req.location.repository).await?;
        let repository = self.req.location.repository.clone();
        let token = token(ctx, &access, None, &repository, "read").await?;
        if self.req.mode == SourceMode::TwoWay {
            self::token(ctx, &access, None, &repository, "write").await?;
        }
        let snapshot = github::snapshot(ctx, &self.req.location, &token).await?;
        if snapshot.commit != self.req.commit {
            return Err(CommandError::conflict(
                "The source branch changed. Preview it again before enabling sync",
            ));
        }
        let now = Utc::now();
        let mut row = AgentSourceRow {
            agent_id: a.internal_id,
            org_id: ctx.org_id(),
            generation: Uuid::new_v4(),
            identity_id: access.identity,
            connection_id: access.connection,
            installation_id: access.installation,
            github_account: access.account.as_str().into(),
            configured_by: ctx.caller.user_id,
            configured_role: serde_json::to_value(ctx.caller.role)
                .map_err(|e| CommandError::internal(e.into()))?
                .as_str()
                .unwrap_or("member")
                .to_string(),
            repository_id: snapshot.repository_id,
            repository: snapshot.repository,
            branch: self.req.location.branch,
            path: self.req.location.path,
            mode: self.req.mode.as_str().into(),
            local_digest: None,
            local_package: None,
            local_dirty: false,
            outbox: None,
            managed_channels: snapshot.package.manifest.channels.keys().cloned().collect(),
            applied_commit: Some(snapshot.commit),
            package_digest: Some(digest(&snapshot.package)?),
            status: "synced".into(),
            error: None,
            last_checked_at: Some(now),
            last_synced_at: Some(now),
            next_sync_at: now,
            lease_token: None,
            lease_until: None,
        };
        transaction::scope(
            &ctx.db,
            async {
                ctx.db
                    .lock_agent_definition(ctx.org_id(), a.internal_id)
                    .await?;
                let existing = ctx.db.get_agent_source(ctx.org_id(), a.internal_id).await?;
                if existing.as_ref().map(|r| r.generation) != self.req.expected_generation
                    || current_digest(ctx, &self.agent_id).await? != self.req.agent_digest
                {
                    return Err(CommandError::conflict(
                        "The Agent or source binding changed. Preview it again",
                    ));
                }
                let fresh = agent(ctx, &self.agent_id).await?;
                if fresh.status != super::record::AgentStatus::Active {
                    return Err(CommandError::conflict("The Agent is no longer active"));
                }
                if validate_connection(ctx, &access, None).await.is_err() {
                    return Err(CommandError::conflict(
                        "The GitHub connection changed. Preview the source again",
                    ));
                }
                apply(ctx, &snapshot.package, &self.agent_id, a.internal_id).await?;
                let local =
                    packages::export_source(ctx, &self.agent_id, Some(&row.managed_channels))
                        .await?;
                row.local_digest = Some(digest(&local)?);
                row.local_package = Some(
                    serde_json::to_value(&local.manifest)
                        .map_err(|e| CommandError::internal(e.into()))?,
                );
                ctx.db.set_agent_source(&row).await?;
                Ok(AgentSourceStatus::from(row))
            },
            CommandError::internal,
        )
        .await
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct DisconnectAgentSource {
    pub agent_id: String,
}
#[command(name="disconnect_agent_source",summary="Disconnect Agent source sync",category="agents",description="Disconnect GitHub source sync and restore local editing. Keeps the last applied Agent definition.",method="DELETE",path="/v1/agents/{agent_id}/source",policy=AGENT_MANAGE,http=plain,cli=CliRoute::new(&["agents", "source"], "disconnect").with_examples(&[CliExample::new("Keep the last applied definition and restore local editing", "everruns agents source disconnect --agent-id agent_01h9 --reason \"Resume local editing\"")]))]
impl Command for DisconnectAgentSource {
    type Output = Value;
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let a = agent(ctx, &self.agent_id).await?;
        ctx.db
            .lock_agent_definition(ctx.org_id(), a.internal_id)
            .await?;
        ctx.db
            .delete_agent_source(ctx.org_id(), a.internal_id)
            .await?;
        Ok(json!({"disconnected":true}))
    }
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SyncAgentSource {
    pub agent_id: String,
}
#[command(name="sync_agent_source",summary="Request an Agent source check",category="agents",description="Queue an immediate check of this Agent's GitHub source.",method="POST",path="/v1/agents/{agent_id}/source/sync",policy=AGENT_MANAGE,http=plain,cli=CliRoute::new(&["agents", "source"], "sync").with_examples(&[CliExample::new("Check the source now after pushing a repository change", "everruns agents source sync --agent-id agent_01h9 --reason \"Apply the latest repository change\"")]))]
impl Command for SyncAgentSource {
    type Output = AgentSourceStatus;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let row = binding(ctx, &self.agent_id).await?;
        ctx.db
            .request_agent_source_sync(ctx.org_id(), row.agent_id)
            .await?;
        Ok(ctx
            .db
            .get_agent_source(ctx.org_id(), row.agent_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Agent source"))?
            .into())
    }
}

async fn sync(ctx: &Ctx, row: &AgentSourceRow) -> Result<(), CommandError> {
    // Database IDs and public Agent IDs can differ for created Agents.
    let id = ctx
        .db
        .get_agent(ctx.org_id(), AgentId::from_uuid(row.agent_id))
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))?
        .public_id;
    let a = agent(ctx, &id).await?;
    if a.status != super::record::AgentStatus::Active {
        return Ok(());
    }
    if row.mode == "two_way" {
        return write::sync(ctx, row, &id).await;
    }
    let access = SourceAccess::of(row);
    let token = token(ctx, &access, row.configured_by, &row.repository, "read").await?;
    let location = SourceLocation {
        repository: row.repository.clone(),
        branch: row.branch.clone(),
        path: row.path.clone(),
    };
    if github::unchanged(
        ctx,
        &location,
        &token,
        row.repository_id,
        row.applied_commit.as_deref(),
    )
    .await?
    {
        return transaction::scope(
            &ctx.db,
            async {
                ctx.db
                    .lock_agent_definition(row.org_id, row.agent_id)
                    .await?;
                if let Some(current) = ctx.db.get_agent_source(row.org_id, row.agent_id).await?
                    && current.generation == row.generation
                    && current.lease_token == row.lease_token
                    && current.lease_until.is_some_and(|t| t > Utc::now())
                {
                    ctx.db
                        .finish_agent_source_sync(row, None, None, None, None)
                        .await?;
                }
                Ok(())
            },
            CommandError::internal,
        )
        .await;
    }
    let snapshot = github::snapshot(
        ctx,
        &SourceLocation {
            repository: row.repository.clone(),
            branch: row.branch.clone(),
            path: row.path.clone(),
        },
        &token,
    )
    .await?;
    if snapshot.repository_id != row.repository_id {
        return Err(CommandError::bad_request(
            "The repository identity changed. Select the source again",
        ));
    }
    let package_digest = digest(&snapshot.package)?;
    let mut sync_ctx = ctx.clone();
    sync_ctx.change_intent = Some(crate::domains::change_history::ChangeIntent {
        reason: Some(format!(
            "Sync Agent source from {} at {}",
            row.repository, snapshot.commit
        )),
        surface: Some(crate::domains::change_history::ChangeSurface::Internal),
        ..Default::default()
    });
    transaction::scope(
        &ctx.db,
        async {
            ctx.db
                .lock_agent_definition(row.org_id, row.agent_id)
                .await?;
            let Some(current) = ctx.db.get_agent_source(row.org_id, row.agent_id).await? else {
                return Ok(());
            };
            if current.generation != row.generation
                || current.lease_token != row.lease_token
                || current.lease_until.is_none_or(|t| t <= Utc::now())
            {
                return Ok(());
            }
            // THREAT[TM-GHAPP-012]: stale jobs cannot commit past binding/lease changes.
            let a = agent(ctx, &id).await?;
            if a.status != super::record::AgentStatus::Active {
                return Ok(());
            }
            validate_connection(ctx, &access, row.configured_by).await?;
            if current.package_digest.as_deref() != Some(&package_digest) {
                apply(&sync_ctx, &snapshot.package, &id, row.agent_id).await?;
            }
            ctx.db
                .finish_agent_source_sync(
                    row,
                    Some(&snapshot.commit),
                    Some(&package_digest),
                    Some(
                        &snapshot
                            .package
                            .manifest
                            .channels
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>(),
                    ),
                    None,
                )
                .await?;
            Ok(())
        },
        CommandError::internal,
    )
    .await
}

/// Reconcile one leased binding. Failures retain the previous applied revision.
pub async fn reconcile_one(template: &Ctx) -> Result<bool, CommandError> {
    let Some(row) = template.db.claim_agent_source_sync().await? else {
        return Ok(false);
    };
    let result = tokio::time::timeout(std::time::Duration::from_secs(180), async {
        let mut ctx = template.clone();
        // Repository changes never inherit an internal caller's policy bypass.
        ctx.caller = Caller::internal(row.org_id);
        ctx.caller.is_internal = false;
        ctx.caller.is_platform_user = false;
        ctx.caller.role = serde_json::from_value(json!(row.configured_role))
            .map_err(|_| CommandError::forbidden("Source sync role is unavailable"))?;
        if let Some(user) = row.configured_by {
            let member = ctx
                .db
                .get_organization_member(row.org_id, user)
                .await?
                .ok_or_else(|| {
                    CommandError::forbidden(
                        "The user who configured source sync is no longer an organization member",
                    )
                })?;
            ctx.caller.user_id = Some(user);
            ctx.caller.role = serde_json::from_value(json!(member.role))
                .map_err(|_| CommandError::forbidden("Source sync membership is unavailable"))?;
        }
        let org = ctx
            .db
            .get_organization(row.org_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Organization"))?;
        ctx.caller.org_public_id = org.public_id;
        ctx.feature_flags = crate::records::feature_flags::FeatureFlagPolicy::current()
            .for_org(&ctx.db.list_org_feature_flags(row.org_id).await?);
        AGENT_MANAGE
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .map_err(|_| {
                CommandError::forbidden(
                    "The source sync owner no longer has Agent management permission",
                )
            })?;
        sync(&ctx, &row).await
    })
    .await
    .unwrap_or_else(|_| {
        Err(CommandError::unavailable(
            "GitHub source sync timed out. Retrying automatically",
        ))
    });
    if let Err(error) = result {
        let message = if error.status().is_server_error() {
            "Could not complete source sync. Retrying automatically".to_string()
        } else {
            error.message().chars().take(2048).collect()
        };
        template
            .db
            .finish_agent_source_sync(&row, None, None, None, Some(&message))
            .await?;
    }
    Ok(true)
}

pub fn source_sync_job(template: Ctx) -> crate::background::cluster_jobs::ClusterJob {
    crate::background::cluster_jobs::ClusterJob::every(
        "agent-source-sync",
        "agent_source_sync",
        "Reconcile continuous GitHub Agent sources.",
        std::time::Duration::from_secs(5),
        move || {
            let template = template.clone();
            Box::pin(async move {
                use futures::StreamExt;
                // Bounded concurrency prevents one slow repository delaying every Agent.
                futures::stream::iter(0..16)
                    .for_each_concurrent(4, |_| {
                        let template = template.clone();
                        async move {
                            if let Err(error) = reconcile_one(&template).await {
                                tracing::error!(%error, "Could not reconcile Agent source");
                            }
                        }
                    })
                    .await;
            })
        },
    )
}

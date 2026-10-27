//! Continuous Agent source synchronization with opt-in conditional GitHub writes.
//! Source authority is task-local and scoped to one tenant and Agent, never a request flag.
mod github;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
mod types;
mod write;
pub use types::{AgentSourceStatus, SourceMode};
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
async fn identity(ctx: &Ctx, a: &super::record::Agent) -> Result<(Uuid, Uuid, i64), CommandError> {
    let id = a.service_virtual_user_id.ok_or_else(|| {
        CommandError::bad_request("Connect GitHub in this Agent's Integrations first")
    })?;
    ctx.db
        .get_virtual_user(ctx.org_id(), id)
        .await?
        .filter(|v| v.status == "active")
        .ok_or_else(|| CommandError::bad_request("Agent GitHub identity is unavailable"))?;
    let connection = ctx
        .db
        .get_virtual_user_connection(id, "github")
        .await?
        .filter(|c| c.installation_id.is_some() && is_app_connection(c))
        .ok_or_else(|| {
            CommandError::bad_request("Connect the Agent to a GitHub App installation first")
        })?;
    Ok((
        id.uuid(),
        connection.id,
        connection
            .installation_id
            .ok_or_else(|| CommandError::bad_request("GitHub installation is unavailable"))?,
    ))
}
async fn validate_connection(
    ctx: &Ctx,
    identity: Uuid,
    connection: Uuid,
    installation: i64,
) -> Result<(), CommandError> {
    // THREAT[TM-GHAPP-014]: never substitute a replacement installation or identity.
    ctx.db
        .get_virtual_user(ctx.org_id(), VirtualUserId::from_uuid(identity))
        .await?
        .filter(|v| v.status == "active")
        .ok_or_else(|| CommandError::bad_request("Source GitHub identity is unavailable"))?;
    let current = ctx
        .db
        .get_virtual_user_connection(VirtualUserId::from_uuid(identity), "github")
        .await?;
    if !current.is_some_and(|c| {
        c.id == connection && c.installation_id == Some(installation) && is_app_connection(&c)
    }) {
        return Err(CommandError::bad_request(
            "The source GitHub connection changed or was disconnected. Reconnect and select the source again",
        ));
    }
    Ok(())
}
async fn token(
    ctx: &Ctx,
    identity: Uuid,
    connection: Uuid,
    installation: i64,
    repository: &str,
) -> Result<String, CommandError> {
    validate_connection(ctx, identity, connection, installation).await?;
    let token = ctx.connection_resolver.as_ref().ok_or_else(||CommandError::unavailable("GitHub connection resolver is unavailable"))?
        .get_github_token_for_owner(ctx.org_id(),identity,&crate::domains::git_sources::github_read_access(repository))
        .await.map_err(|_|CommandError::bad_request("Could not authorize read access to the source repository. Check the GitHub connection"))?
        .ok_or_else(||CommandError::bad_request("GitHub read access to the source repository is unavailable"))?;
    validate_connection(ctx, identity, connection, installation).await?;
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
        let a = agent(ctx, &self.agent_id).await?;
        let (identity, connection, installation) = identity(ctx, &a).await?;
        let repo = crate::domains::git_sources::normalize_github_repository(&self.req.repository)
            .map_err(CommandError::bad_request)?;
        let token = token(ctx, identity, connection, installation, &repo).await?;
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
}
#[command(name="preview_agent_source",summary="Preview Agent source changes",category="agents",description="Validate and compare a GitHub Agent package without changing the Agent.",method="POST",path="/v1/agents/{agent_id}/source/preview",policy=AGENT_VIEW,http=plain,request_body(SourceLocation),read_only=true,cli=CliRoute::new(&["agents", "source"], "preview").with_examples(&[CliExample::new("Review a repository package without changing the Agent", "everruns agents source preview --agent-id agent_01h9 --repository acme/agents --branch main --path agents/support")]))]
impl Command for PreviewAgentSource {
    type Output = Value;
    async fn execute(mut self, ctx: &Ctx) -> Result<Value, CommandError> {
        self.req.validate()?;
        let a = agent(ctx, &self.agent_id).await?;
        let (identity, connection, installation) = identity(ctx, &a).await?;
        let token = token(
            ctx,
            identity,
            connection,
            installation,
            &self.req.repository,
        )
        .await?;
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
        let (identity_id, connection_id, installation_id) = identity(ctx, &a).await?;
        let token = token(
            ctx,
            identity_id,
            connection_id,
            installation_id,
            &self.req.location.repository,
        )
        .await?;
        if self.req.mode == SourceMode::TwoWay {
            write::token(
                ctx,
                identity_id,
                connection_id,
                installation_id,
                &self.req.location.repository,
            )
            .await?;
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
            identity_id,
            connection_id,
            installation_id,
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
                if identity(ctx, &fresh).await? != (identity_id, connection_id, installation_id) {
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
    let token = token(
        ctx,
        row.identity_id,
        row.connection_id,
        row.installation_id,
        &row.repository,
    )
    .await?;
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
            validate_connection(ctx, row.identity_id, row.connection_id, row.installation_id)
                .await?;
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

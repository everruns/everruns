//! Durable outbound delivery. Immutable Git objects are prepared before a fenced,
//! non-forced ref update. The persisted commit makes uncertain responses recoverable.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::BTreeSet;

const CONFLICT: &str = "Both GitHub and Everruns changed the Agent package. Local edits are retained. Review the GitHub version to replace local edits, or disconnect to keep editing locally";

#[derive(Serialize, Deserialize)]
struct Outbox {
    parent: String,
    commit: String,
    local_digest: String,
    local_package: Value,
    package_digest: String,
    channels: Vec<String>,
}

pub(super) async fn token(
    ctx: &Ctx,
    identity: Uuid,
    connection: Uuid,
    installation: i64,
    repo: &str,
) -> Result<String, CommandError> {
    validate_connection(ctx, identity, connection, installation).await?;
    let access = everruns_contracts::runtime::GitHubAccess::new(
        everruns_core::McpServerActsAs::User,
        &[("contents", "write")],
    )
    .for_repository(repo);
    let result = ctx.connection_resolver.as_ref()
        .ok_or_else(|| CommandError::unavailable("GitHub connection resolver is unavailable"))?
        .get_github_token_for_owner(ctx.org_id(), identity, &access).await
        .map_err(|_| CommandError::bad_request("Two-way sync needs Contents write permission on this repository. Update the GitHub App and accept its installation permissions"))?
        .ok_or_else(|| CommandError::bad_request("GitHub write access is unavailable"))?;
    validate_connection(ctx, identity, connection, installation).await?;
    Ok(result)
}

pub(super) async fn local_package(ctx: &Ctx, id: &str) -> Result<AgentPackage, CommandError> {
    let a = agent(ctx, id).await?;
    let source = ctx.db.get_agent_source(ctx.org_id(), a.internal_id).await?;
    packages::export_source(
        ctx,
        id,
        source.as_ref().map(|s| s.managed_channels.as_slice()),
    )
    .await
}

async fn active(ctx: &Ctx, row: &AgentSourceRow) -> Result<Option<AgentSourceRow>, CommandError> {
    ctx.db
        .lock_agent_definition(row.org_id, row.agent_id)
        .await?;
    let Some(current) = ctx.db.get_agent_source(row.org_id, row.agent_id).await? else {
        return Ok(None);
    };
    if current.mode != "two_way"
        || current.generation != row.generation
        || current.lease_token != row.lease_token
        || current.lease_until.is_none_or(|t| t <= Utc::now())
    {
        return Ok(None);
    }
    let a = ctx
        .db
        .get_agent(row.org_id, AgentId::from_uuid(row.agent_id))
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    if a.status != "active" {
        return Ok(None);
    }
    validate_connection(ctx, row.identity_id, row.connection_id, row.installation_id).await?;
    Ok(Some(current))
}

async fn write(
    ctx: &Ctx,
    row: &AgentSourceRow,
    suffix: &str,
    body: Value,
    token: &str,
    patch: bool,
) -> Result<Value, CommandError> {
    ctx.github_apps
        .api
        .write_source_json(
            if patch {
                reqwest::Method::PATCH
            } else {
                reqwest::Method::POST
            },
            &format!("/repos/{}/{suffix}", row.repository),
            token,
            &body,
        )
        .await
        .map_err(|e| CommandError::bad_request(e.to_string()))
}

async fn prepare(
    ctx: &Ctx,
    row: &AgentSourceRow,
    snapshot: &github::Snapshot,
    previous: &AgentPackage,
    local: &AgentPackage,
    token: &str,
) -> Result<Outbox, CommandError> {
    let entries = AgentPackage::edit_entries(
        snapshot.entries.clone(),
        &snapshot.manifest,
        previous,
        local,
    )
    .map_err(packages::package_error)?;
    let proposed = AgentPackage::from_manifest_entries(entries.clone(), &snapshot.manifest)
        .map_err(packages::package_error)?;
    // The same validation as inbound import protects policies before publishing.
    let id = ctx
        .db
        .get_agent(row.org_id, AgentId::from_uuid(row.agent_id))
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))?
        .public_id;
    if validate(ctx, &proposed, &id).await?["valid"] != true {
        return Err(CommandError::bad_request(
            "The edited package is not valid for this Agent",
        ));
    }
    let paths: BTreeSet<_> = snapshot.entries.keys().chain(entries.keys()).collect();
    let mut changes = Vec::new();
    for path in paths {
        if snapshot.entries.get(path) == entries.get(path) {
            continue;
        }
        let full_path = if snapshot.package_root.is_empty() {
            path.clone()
        } else {
            format!("{}/{path}", snapshot.package_root)
        };
        // No workflow permission is requested, even for a package rooted at /.
        if full_path.starts_with(".github/workflows/") {
            return Err(CommandError::bad_request(
                "Source sync cannot modify GitHub workflows",
            ));
        }
        let sha = if let Some(bytes) = entries.get(path) {
            let value = write(
                ctx,
                row,
                "git/blobs",
                json!({"encoding":"base64", "content":STANDARD.encode(bytes)}),
                token,
                false,
            )
            .await?;
            json!(github::sha(&value)?)
        } else {
            Value::Null
        };
        changes.push(json!({"path":full_path,"mode":snapshot.modes.get(path).map(String::as_str).unwrap_or("100644"),"type":"blob","sha":sha}));
    }
    let tree = write(
        ctx,
        row,
        "git/trees",
        json!({"base_tree":snapshot.root_tree,"tree":changes}),
        token,
        false,
    )
    .await?;
    let commit = write(ctx, row, "git/commits", json!({"message":"Sync Agent source from Everruns", "tree":github::sha(&tree)?, "parents":[snapshot.commit]}), token, false).await?;
    Ok(Outbox {
        parent: snapshot.commit.clone(),
        commit: github::sha(&commit)?,
        local_digest: digest(local)?,
        local_package: serde_json::to_value(&local.manifest)
            .map_err(|e| CommandError::internal(e.into()))?,
        package_digest: digest(&proposed)?,
        channels: proposed.manifest.channels.keys().cloned().collect(),
    })
}

async fn publish(
    ctx: &Ctx,
    row: &AgentSourceRow,
    id: &str,
    outbox: Outbox,
    token: &str,
) -> Result<(), CommandError> {
    transaction::scope(
        &ctx.db,
        async {
            let Some(current) = active(ctx, row).await? else {
                return Ok(());
            };
            if current.outbox.as_ref().and_then(|o| o["commit"].as_str()) != Some(&outbox.commit) {
                return Ok(());
            }
            let info = github::metadata(ctx, &row.repository, token).await?;
            if info["id"].as_i64() != Some(row.repository_id) {
                return Err(CommandError::conflict(
                    "The repository identity changed. Select the source again",
                ));
            }
            let location = SourceLocation {
                repository: row.repository.clone(),
                branch: row.branch.clone(),
                path: row.path.clone(),
            };
            let (head, _) = github::root(ctx, &location, token).await?;
            let delivered = if head == outbox.commit {
                true
            } else if head == outbox.parent {
                false
            } else {
                // A successful update may have lost its HTTP response, then acquired
                // newer descendants. Never replay a stale ref over those commits.
                let comparison = github::read(
                    ctx,
                    &row.repository,
                    &format!("compare/{}...{}", outbox.commit, head),
                    token,
                )
                .await?;
                if matches!(comparison["status"].as_str(), Some("ahead" | "identical")) {
                    true
                } else {
                    ctx.db.conflict_agent_source(row, CONFLICT).await?;
                    return Ok(());
                }
            };
            if !delivered {
            // A durable outbox is not a cached policy grant. Recheck current
            // dependencies, feature availability and authored permissions before
            // a retry makes an external change. Already-delivered commits only
            // need acknowledgment and never replay this ref update.
            let candidate = AgentPackage::new(serde_json::from_value(outbox.local_package.clone())
                .map_err(|e| CommandError::internal(e.into()))?).map_err(packages::package_error)?;
            if validate(ctx, &candidate, id).await?["valid"] != true {
                return Err(CommandError::bad_request("Queued edits no longer satisfy current Agent policies. Review the source before retrying"));
            }
                // THREAT[TM-GHAPP-015]: sibling commits are rejected by GitHub;
                // force=false is mandatory. A branch protection rejection retains
                // the outbox and local definition for an explicit recovery.
                write(
                    ctx,
                    row,
                    &format!("git/refs/heads/{}", github::segment(&row.branch)),
                    json!({"sha":outbox.commit,"force":false}),
                    token,
                    true,
                )
                .await?;
            }
            let dirty = digest(&local_package(ctx, id).await?)? != outbox.local_digest;
            ctx.db
                .finish_agent_source_write(row, &outbox.local_digest, &outbox.local_package, dirty)
                .await?;
            ctx.db
                .finish_agent_source_sync(
                    row,
                    Some(&outbox.commit),
                    Some(&outbox.package_digest),
                    if dirty { None } else { Some(&outbox.channels) },
                    None,
                )
                .await?;
            Ok(())
        },
        CommandError::internal,
    )
    .await
}

pub(super) async fn sync(ctx: &Ctx, row: &AgentSourceRow, id: &str) -> Result<(), CommandError> {
    let token = token(
        ctx,
        row.identity_id,
        row.connection_id,
        row.installation_id,
        &row.repository,
    )
    .await?;
    if let Some(outbox) = &row.outbox {
        return publish(
            ctx,
            row,
            id,
            serde_json::from_value(outbox.clone()).map_err(|e| CommandError::internal(e.into()))?,
            &token,
        )
        .await;
    }
    let location = SourceLocation {
        repository: row.repository.clone(),
        branch: row.branch.clone(),
        path: row.path.clone(),
    };
    // Most continuous checks are idle. Match pull mode's cheap ref check;
    // package blobs are read only when either side actually changed.
    if !row.local_dirty
        && github::unchanged(
            ctx,
            &location,
            &token,
            row.repository_id,
            row.applied_commit.as_deref(),
        )
        .await?
    {
        let done = transaction::scope(
            &ctx.db,
            async {
                let Some(current) = active(ctx, row).await? else {
                    return Ok(true);
                };
                let local = local_package(ctx, id).await?;
                let local_digest = digest(&local)?;
                if current.local_digest.as_deref() != Some(&local_digest) {
                    return Ok(false);
                }
                let value = serde_json::to_value(&local.manifest)
                    .map_err(|e| CommandError::internal(e.into()))?;
                ctx.db
                    .finish_agent_source_write(row, &local_digest, &value, false)
                    .await?;
                ctx.db
                    .finish_agent_source_sync(row, None, None, None, None)
                    .await?;
                Ok(true)
            },
            CommandError::internal,
        )
        .await?;
        if done {
            return Ok(());
        }
    }
    let snapshot = github::snapshot(ctx, &location, &token).await?;
    if snapshot.repository_id != row.repository_id {
        return Err(CommandError::conflict(
            "The repository identity changed. Select the source again",
        ));
    }
    let remote_digest = digest(&snapshot.package)?;
    let local = transaction::scope(
        &ctx.db,
        async {
            let Some(current) = active(ctx, row).await? else {
                return Ok(None);
            };
            let local = local_package(ctx, id).await?;
            let local_digest = digest(&local)?;
            let local_changed = current.local_digest.as_deref() != Some(&local_digest);
            let remote_changed = current.package_digest.as_deref() != Some(&remote_digest);
            if local_changed && remote_changed {
                ctx.db.conflict_agent_source(row, CONFLICT).await?;
                return Ok(None);
            }
            if local_changed {
                return Ok(Some(local));
            }
            if remote_changed {
                let mut apply_ctx = ctx.clone();
                apply_ctx.change_intent = Some(crate::domains::change_history::ChangeIntent {
                    reason: Some(format!(
                        "Sync Agent source from {} at {}",
                        row.repository, snapshot.commit
                    )),
                    surface: Some(crate::domains::change_history::ChangeSurface::Internal),
                    ..Default::default()
                });
                apply(&apply_ctx, &snapshot.package, id, row.agent_id).await?;
            }
            let names: Vec<_> = snapshot.package.manifest.channels.keys().cloned().collect();
            let local = packages::export_source(ctx, id, Some(&names)).await?;
            let value = serde_json::to_value(&local.manifest)
                .map_err(|e| CommandError::internal(e.into()))?;
            ctx.db
                .finish_agent_source_write(row, &digest(&local)?, &value, false)
                .await?;
            ctx.db
                .finish_agent_source_sync(
                    row,
                    Some(&snapshot.commit),
                    Some(&remote_digest),
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
            Ok(None)
        },
        CommandError::internal,
    )
    .await?;
    let Some(local) = local else {
        return Ok(());
    };
    let previous = AgentPackage::new(
        serde_json::from_value(row.local_package.clone().ok_or_else(|| {
            CommandError::conflict("Review the source again to establish a write baseline")
        })?)
        .map_err(|e| CommandError::internal(e.into()))?,
    )
    .map_err(packages::package_error)?;
    let outbox = transaction::scope(
        &ctx.db,
        async {
            if active(ctx, row).await?.is_none() {
                return Ok(None);
            }
            // Serialize all remote writes with disconnect/rebind, including creation
            // of immutable objects. Only a request already in flight can outlive a
            // cancelled transaction; a ref is never published before durable staging.
            let outbox = prepare(ctx, row, &snapshot, &previous, &local, &token).await?;
            ctx.db
                .stage_agent_source_outbox(
                    row,
                    &serde_json::to_value(&outbox).map_err(|e| CommandError::internal(e.into()))?,
                )
                .await?;
            Ok(Some(outbox))
        },
        CommandError::internal,
    )
    .await?;
    if let Some(outbox) = outbox {
        publish(ctx, row, id, outbox, &token).await?;
    }

    Ok(())
}

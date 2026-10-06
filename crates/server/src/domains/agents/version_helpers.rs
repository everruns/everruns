use super::*;

pub(super) async fn resolve_agent(ctx: &Ctx, id: &str) -> Result<Agent, CommandError> {
    q::resolve(&ctx.db, ctx.org_id(), id)
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))
}

/// Resolve an agent for a command that mutates its version history.
///
/// Versions are part of the definition, so they are protected: a platform
/// upgrade ships a new built-in version, and an org that rolled its own would
/// silently diverge. Read-only version commands use [`resolve_agent`] instead.
pub(super) async fn resolve_agent_for_mutation(ctx: &Ctx, id: &str) -> Result<Agent, CommandError> {
    let agent = resolve_agent(ctx, id).await?;
    q::ensure_not_built_in(&ctx.db, ctx.org_id(), id, "modify").await?;
    Ok(agent)
}

pub(super) async fn resolve_agent_version(
    ctx: &Ctx,
    version_id: AgentVersionId,
) -> Result<AgentVersion, CommandError> {
    ctx.db
        .get_agent_version(ctx.org_id(), version_id)
        .await?
        .map(q::row_to_agent_version)
        .ok_or_else(|| CommandError::not_found("Agent version"))
}

pub(super) async fn build_resolved_config(
    ctx: &Ctx,
    agent: &Agent,
) -> Result<serde_json::Value, CommandError> {
    let preview = PreviewAgent {
        harness_id: None,
        initial_files: agent.initial_files.clone(),
        system_prompt: Some(agent.system_prompt.clone()),
        capabilities: agent.capabilities.clone(),
        tools: agent.tools.clone(),
        mcp_servers: agent.mcp_servers.clone(),
    }
    .execute(ctx)
    .await?;
    Ok(serde_json::json!({
        "system_prompt": preview.system_prompt,
        "tools": preview.tools,
        "capabilities": agent.capabilities,
        "mcp_servers": agent.mcp_servers,
        "default_model_id": agent.default_model_id.map(|id| id.to_string()),
        "harness_id": agent.harness_id.to_string(),
        "max_iterations": agent.max_iterations,
        "parallel_tool_calls": agent.parallel_tool_calls,
    }))
}

pub(super) fn bump_published_version(
    previous: Option<&crate::storage::models::AgentVersionRow>,
    change_kind: &AgentVersionChangeKind,
) -> (i32, i32, i32, String) {
    if previous.is_none() {
        return (0, 1, 0, "0.1.0".to_string());
    }

    let (mut major, mut minor, mut patch) = previous
        .map(|v| (v.semver_major, v.semver_minor, v.semver_patch))
        .unwrap_or((0, 0, 0));
    match change_kind {
        AgentVersionChangeKind::Major => {
            major += 1;
            minor = 0;
            patch = 0;
        }
        AgentVersionChangeKind::Minor | AgentVersionChangeKind::Fork => {
            minor += 1;
            patch = 0;
        }
        _ => patch += 1,
    }
    let version = format!("{major}.{minor}.{patch}");
    (major, minor, patch, version)
}

pub(super) async fn create_version_from_agent(
    ctx: &Ctx,
    agent: &Agent,
    change_kind: AgentVersionChangeKind,
    summary: Option<String>,
    source_version_id: Option<AgentVersionId>,
    is_published: bool,
) -> Result<AgentVersion, CommandError> {
    let agent_id = AgentId::from_uuid(agent.internal_id);
    let previous_snapshot = ctx
        .db
        .get_latest_agent_snapshot(ctx.org_id(), agent_id)
        .await?;
    let previous_published = ctx
        .db
        .get_latest_agent_version(ctx.org_id(), agent_id)
        .await?;
    let version_number = previous_snapshot
        .as_ref()
        .map_or(1, |row| row.version_number + 1);
    let (semver_major, semver_minor, semver_patch, version) = if is_published {
        bump_published_version(previous_published.as_ref(), &change_kind)
    } else {
        (0, 0, 0, format!("draft.{version_number}"))
    };
    let authored_config = q::authored_config(agent);
    let resolved_config = build_resolved_config(ctx, agent).await?;
    let version_id = AgentVersionId::new();
    let row = ctx
        .db
        .create_agent_version(crate::storage::models::CreateAgentVersionRow {
            id: version_id,
            public_id: version_id.to_string(),
            org_id: ctx.org_id(),
            agent_id,
            version_number,
            semver_major,
            semver_minor,
            semver_patch,
            version,
            is_published,
            parent_version_id: if is_published {
                previous_published.map(|row| row.id)
            } else {
                previous_snapshot.map(|row| row.id)
            },
            source_version_id,
            created_by_principal_id: None,
            change_kind: change_kind.to_string(),
            summary,
            config_hash: q::config_hash(&authored_config),
            authored_config,
            resolved_config,
        })
        .await?;
    if is_published && agent.default_version_id.is_none() {
        ctx.db
            .update_agent(
                ctx.org_id(),
                agent_id,
                UpdateAgent {
                    default_version_id: Some(row.id),
                    ..Default::default()
                },
            )
            .await?;
    }
    Ok(q::row_to_agent_version(row))
}

pub(super) async fn create_auto_snapshot_from_agent(
    ctx: &Ctx,
    agent: &Agent,
) -> Result<(), CommandError> {
    if !ctx.feature_flags.agent_versions {
        return Ok(());
    }

    let agent_id = AgentId::from_uuid(agent.internal_id);
    let authored_config = q::authored_config(agent);
    let config_hash = q::config_hash(&authored_config);
    if ctx
        .db
        .get_latest_agent_snapshot(ctx.org_id(), agent_id)
        .await?
        .is_some_and(|row| row.config_hash == config_hash)
    {
        return Ok(());
    }

    // THREAT[TM-DOS-013]: Repeated no-op Agent updates can grow hidden snapshot rows.
    // Mitigation: automatic snapshots are feature-gated, deduplicated by latest config hash, and retained to a bounded per-Agent window.
    let mut last_conflict = None;
    for _ in 0..3 {
        match create_version_from_agent(ctx, agent, AgentVersionChangeKind::Auto, None, None, false)
            .await
        {
            Ok(_) => {
                ctx.db
                    .prune_agent_auto_snapshots(
                        ctx.org_id(),
                        agent_id,
                        MAX_AUTO_SNAPSHOTS_PER_AGENT,
                    )
                    .await?;
                return Ok(());
            }
            Err(CommandError {
                kind: CommandErrorKind::Conflict(message),
                ..
            }) => {
                last_conflict = Some(message);
            }
            Err(error) => return Err(error),
        }
    }
    Err(CommandError::conflict(last_conflict.unwrap_or_else(|| {
        "Agent version number conflict".to_string()
    })))
}

//! `worker_plugins_call`: the Plugins capability's one way into the
//! organization's plugins, for one session (knowledge/integrations/plugins.md,
//! "Loading on request").
//!
//! Decisions:
//! - One internal command carrying a [`PluginsCall`], run as the session's org
//!   after confirming the session belongs to it: a v1 worker sends it over the
//!   link, the injected server runs it in process for a data plane
//!   (`data_plane::calls::PLUGINS_CALL`). Nothing about plugins reaches a cell
//!   or agentd.
//! - Approved means installed and active. Directory plugins nobody approved
//!   are listed (`needs_approval`) and never installed from here: what an org
//!   allows is an admin's call, not a model's.
//! - A plugin loads only where the agent provides the environment it needs
//!   (`everruns_contracts::runtime::capabilities::PluginEnvironment`), resolved from the
//!   session's capabilities exactly as its runtime resolves them.
//! - Loading writes the chat's own records, so it works the same on both
//!   planes and needs no change to the agent or the session's definition:
//!   each skill under `/.agents/skills/{skill}/` (seeded where absent, so a
//!   file the agent changed stays), each MCP server as a session MCP record
//!   (`everruns_core::session_mcp_servers`, source `plugin`). A record never
//!   replaces a server of the same name that something else added.
//! - A plugin whose MCP server still needs an admin to choose its acting
//!   identity does not load: that server could not run.

use everruns_capabilities::capabilities::PluginsCall;
use everruns_contracts::runtime::capabilities::PluginEnvironment;
use everruns_contracts::session_commands;
use everruns_core::session_file::InitialFile;
use everruns_core::session_mcp_servers::{
    SessionMcpServer, SessionMcpServerSource, get_session_mcp_server, load_session_mcp_servers,
    put_session_mcp_server,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;

use super::queries as q;
use crate::domains::common::*;
use crate::domains::sessions::SESSION_MANAGE;
use crate::kernel_imports::contracts::typed_id::SessionId;
use crate::storage::PluginInstallRow;

/// Most plugins one search lists.
const MAX_RESULTS: usize = 20;
/// Where skills live in a session's workspace (`SkillsCapability`).
const SKILLS_ROOT: &str = "/.agents/skills";

#[derive(Debug, Deserialize, ToSchema, Serialize)]
pub struct WorkerPluginsCall {
    /// Session's prefixed public identifier.
    pub session_id: String,
    /// What to do, as the `plugins` tools name it.
    #[schema(value_type = Object)]
    pub call: PluginsCall,
}

#[command(
    name = "worker_plugins_call",
    category = "sessions",
    description = "Internal: search the organization's plugins or load one into a session.",
    method = "POST",
    path = "/internal/sessions/{session_id}/plugins",
    policy = SESSION_MANAGE
)]
impl Command for WorkerPluginsCall {
    type Output = Value;

    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let session_id =
            crate::domains::session_storage::queries::parse_owned_session_id(&self.session_id)?;
        let session = crate::domains::sessions::SessionService::new(ctx.db.clone())
            .get(&ctx.caller, session_id.uuid(), None)
            .await
            .map_err(CommandError::internal)?
            .ok_or_else(|| CommandError::not_found("Session"))?;
        let resolved = crate::worker_link::platform_command_surface::session_capabilities(
            &ctx.db,
            ctx.capability_service.registry(),
            ctx.org_id(),
            &session,
        )
        .await
        .map_err(CommandError::internal)?;
        let provided = PluginEnvironment::provided_by(resolved.iter().map(|c| c.capability_id()));
        match self.call {
            PluginsCall::Search { query } => search(ctx, session_id, provided, &query).await,
            PluginsCall::Load { name } => load(ctx, session_id, provided, &name).await,
        }
    }
}

fn matches(query: &str, fields: &[Option<&str>]) -> bool {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return true;
    }
    let haystack = fields
        .iter()
        .flatten()
        .map(|field| field.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    words.iter().all(|word| haystack.contains(word.as_str()))
}

fn manifest_text<'a>(install: &'a PluginInstallRow, key: &str) -> Option<&'a str> {
    install.manifest.get(key).and_then(Value::as_str)
}

/// The plugins the session already loaded, by name.
async fn loaded_plugins(
    ctx: &Ctx,
    session: SessionId,
) -> Result<std::collections::BTreeSet<String>, CommandError> {
    let storage = crate::domains::mcp_servers::session_servers::session_store(ctx, session).await?;
    Ok(load_session_mcp_servers(storage.as_ref(), session)
        .await
        .into_iter()
        .filter_map(|record| match record.source {
            SessionMcpServerSource::Plugin { plugin, .. } => Some(plugin),
            _ => None,
        })
        .collect())
}

async fn search(
    ctx: &Ctx,
    session: SessionId,
    provided: PluginEnvironment,
    query: &str,
) -> Result<Value, CommandError> {
    let installs = ctx.db.list_active_plugin_installs(ctx.org_id()).await?;
    let loaded = loaded_plugins(ctx, session).await?;
    let mut approved = Vec::new();
    for install in &installs {
        let description = manifest_text(install, "description");
        if !matches(
            query,
            &[
                Some(install.name.as_str()),
                description,
                manifest_text(install, "displayName"),
            ],
        ) {
            continue;
        }
        let requirement = q::install_requirement(install);
        let identity_required =
            crate::domains::capabilities::queries::legacy_mcp_servers_requiring_identity(
                &install.definition,
            );
        approved.push(json!({
            "name": install.name,
            "description": description,
            "version": install.version,
            "environment": requirement.environment,
            "fits_this_agent": requirement.environment.satisfied_by(provided),
            "loaded": loaded.contains(&install.name),
            "needs_admin_setup": !identity_required.is_empty(),
        }));
    }
    approved.truncate(MAX_RESULTS);

    // Directory plugins nobody approved yet: the agent may name them, an
    // admin decides.
    let installed: std::collections::BTreeSet<&str> = installs
        .iter()
        .map(|install| install.name.as_str())
        .collect();
    let all_installed: std::collections::BTreeSet<String> = ctx
        .db
        .list_plugin_installs(ctx.org_id(), None)
        .await?
        .into_iter()
        .map(|install| install.name)
        .collect();
    let mut available = Vec::new();
    for marketplace in ctx.db.list_plugin_marketplaces(ctx.org_id(), None).await? {
        if marketplace.status != "active" {
            continue;
        }
        let entries = marketplace
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.get("plugins"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for entry in entries {
            let Some(name) = entry.get("name").and_then(Value::as_str) else {
                continue;
            };
            if installed.contains(name) || all_installed.contains(name) {
                continue;
            }
            let description = entry.get("description").and_then(Value::as_str);
            let category = entry.get("category").and_then(Value::as_str);
            if !matches(query, &[Some(name), description, category]) {
                continue;
            }
            available.push(json!({
                "name": name,
                "description": description,
                "source": marketplace.name,
                "needs_approval": true,
            }));
            if available.len() >= MAX_RESULTS {
                break;
            }
        }
    }

    Ok(json!({
        "this_agent_provides": provided,
        "approved": approved,
        "needs_approval": available,
        "note": if approved.is_empty() && available.is_empty() {
            "No plugin matches. Try other words, or ask an organization admin to add one in Settings > Plugins."
        } else if approved.is_empty() {
            "None of these is approved yet: tell the person which plugin to ask an organization admin to approve in Settings > Plugins."
        } else {
            "Load an approved plugin with load_plugin by its name."
        },
    }))
}

async fn load(
    ctx: &Ctx,
    session: SessionId,
    provided: PluginEnvironment,
    name: &str,
) -> Result<Value, CommandError> {
    let install = ctx
        .db
        .get_plugin_install_by_name(ctx.org_id(), name)
        .await?
        .filter(|install| install.status == "active")
        .ok_or_else(|| {
            CommandError::bad_request(format!(
                "'{name}' is not an approved plugin in this organization. Use search_plugins; a plugin that needs approval must be added by an organization admin in Settings > Plugins."
            ))
        })?;
    let requirement = q::install_requirement(&install);
    if !requirement.environment.satisfied_by(provided) {
        return Err(CommandError::bad_request(format!(
            "'{name}' needs a {} agent ({}), and this agent provides {}. It cannot be loaded here.",
            requirement.environment.level_name(),
            requirement.environment,
            provided,
        )));
    }
    let identity_required =
        crate::domains::capabilities::queries::legacy_mcp_servers_requiring_identity(
            &install.definition,
        );
    if !identity_required.is_empty() {
        return Err(CommandError::bad_request(format!(
            "'{name}' is not ready: an organization admin must choose who its MCP server(s) act as ({}) in Settings > Plugins.",
            identity_required.join(", ")
        )));
    }
    let definition: everruns_core::DeclarativeCapabilityDefinition =
        serde_json::from_value(install.definition.clone())
            .map_err(|error| CommandError::internal(error.into()))?;

    let skills = seed_skills(ctx, session, &definition).await?;

    let storage = crate::domains::mcp_servers::session_servers::session_store(ctx, session).await?;
    let mut servers = Vec::new();
    let mut skipped = Vec::new();
    for (server_name, server) in definition.mcp_servers.iter().flatten() {
        let source = SessionMcpServerSource::Plugin {
            plugin: install.name.clone(),
            install_id: install.public_id.clone(),
        };
        if let Some(existing) = get_session_mcp_server(storage.as_ref(), session, server_name).await
            && existing.source != source
        {
            skipped.push(server_name.clone());
            continue;
        }
        put_session_mcp_server(
            storage.as_ref(),
            session,
            &SessionMcpServer {
                name: server_name.clone(),
                server: server.as_scoped().clone(),
                source,
            },
        )
        .await
        .map_err(|error| CommandError::internal(anyhow::anyhow!(error.to_string())))?;
        servers.push(json!({
            "name": server_name,
            "signs_in_as": server.acts_as,
        }));
    }

    Ok(json!({
        "loaded": install.name,
        "version": install.version,
        "skills": skills,
        "mcp_servers": servers,
        "skipped_mcp_servers": skipped,
        "note": match (skills.is_empty(), servers.is_empty()) {
            (_, false) => "Loaded. Its skills are listed now (list_skills); its MCP tools arrive with your next turn. A server that signs in as the person asks them to sign in on first use.",
            (false, true) => "Loaded. Its skills are listed now (list_skills).",
            (true, true) => "Loaded, but it brings no skills or MCP servers this agent can use.",
        },
    }))
}

/// Put the plugin's skills in the session's workspace, where absent: the
/// skill names written.
async fn seed_skills(
    ctx: &Ctx,
    session: SessionId,
    definition: &everruns_core::DeclarativeCapabilityDefinition,
) -> Result<Vec<String>, CommandError> {
    if definition.skills.is_empty() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    let mut names = Vec::new();
    for skill in &definition.skills {
        // Skill names are validated at compile time ([a-z0-9-]); a name that
        // is not a single path segment never reaches the workspace.
        if skill.name.is_empty() || skill.name.contains(['/', '\\']) || skill.name.starts_with('.')
        {
            continue;
        }
        let root = format!("{SKILLS_ROOT}/{}", skill.name);
        files.push(InitialFile {
            path: format!("{root}/SKILL.md"),
            content: everruns_core::reconstruct_skill_md(
                &skill.name,
                &skill.description,
                &skill.instructions,
                skill.user_invocable,
                skill.disable_model_invocation,
            ),
            encoding: "text".into(),
            is_readonly: false,
        });
        for file in &skill.files {
            if file
                .path
                .split('/')
                .any(|part| part == ".." || part.is_empty())
            {
                continue;
            }
            files.push(InitialFile {
                path: format!("{root}/{}", file.path),
                content: file.content.clone(),
                encoding: "text".into(),
                is_readonly: false,
            });
        }
        names.push(skill.name.clone());
    }

    if let Some(plane) =
        crate::domains::session_storage::plane::PlaneStorage::of(ctx, session).await?
    {
        let reply = plane
            .plane()
            .command(
                &session_commands::FS,
                &json!({ "op": "seed", "files": files }),
            )
            .await
            .map_err(CommandError::internal)?;
        if reply["result"] == "failed" {
            return Err(CommandError::internal(anyhow::anyhow!(
                "the plugin's skills were not copied to the session: {}",
                reply["message"].as_str().unwrap_or("unknown reason")
            )));
        }
        return Ok(names);
    }

    let workspace =
        crate::domains::session_files::queries::workspace_key_unscoped(&ctx.db, session)
            .await
            .map_err(CommandError::internal)?
            .ok_or_else(|| CommandError::not_found("Session"))?;
    let service = crate::domains::session_files::queries::service(ctx);
    for file in files {
        let exists = service
            .read_file(workspace, &file.path)
            .await
            .map_err(CommandError::internal)?
            .is_some();
        if exists {
            continue;
        }
        service
            .create_file(
                workspace,
                crate::domains::session_files::service::CreateFileInput {
                    path: file.path,
                    content: Some(file.content),
                    encoding: Some(file.encoding),
                    is_readonly: Some(file.is_readonly),
                },
            )
            .await
            .map_err(CommandError::internal)?;
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn every_query_word_must_appear_in_some_field() {
        let fields = [Some("visti"), Some("Keep a person in the loop"), None];
        assert!(matches("", &fields));
        assert!(matches("VISTI", &fields));
        assert!(matches("person loop", &fields));
        assert!(!matches("person slack", &fields));
    }
}

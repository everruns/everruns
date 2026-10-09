//! Shared import/export business operations for REST, MCP and Platform Chat.
//! Parsing is host-neutral; binding and policy stay on this existing domain.

use super::{
    AGENT_MANAGE, AGENT_VIEW, CreateAgent, GetAgent, UpsertAgent, types::CreateAgentRequest,
};
use crate::domains::common::*;
use crate::records::{Agent, ChannelType};
use everruns_contracts::{CapabilityRef, model_spec::ModelSpec, runtime_provider::ProviderKey};
use everruns_core::agent_package::{
    AgentPackage, Channel, File, Format, Manifest, PackageError, sensitive_key,
};
use everruns_core::{InitialFile, ScopedMcpServer, session_file::SessionFile};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::ToSchema;

pub fn format(value: Option<&str>) -> Result<Format, CommandError> {
    match value.unwrap_or("auto") {
        "auto" => Ok(Format::Auto),
        "markdown" | "md" => Ok(Format::Markdown),
        "toml" => Ok(Format::Toml),
        "yaml" | "yml" => Ok(Format::Yaml),
        "json" => Ok(Format::Json),
        _ => Err(CommandError::bad_request(
            "format must be markdown, toml, yaml or json",
        )),
    }
}
pub fn package_error(error: PackageError) -> CommandError {
    CommandError::bad_request(error.to_string())
}

/// A self-contained agent definition or package in the current session workspace.
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageInput {
    /// Markdown/TOML/YAML/JSON agent definition, with assets embedded.
    #[serde(default)]
    pub content: String,
    /// Path in the current Platform Chat session workspace (never server disk).
    #[serde(default)]
    pub file: Option<String>,
    /// Text encoding: auto, markdown, toml, yaml or json. Defaults to auto-detection.
    #[serde(default)]
    pub format: Option<String>,
    /// Existing agent name to update or compare. Omit to create.
    #[serde(default)]
    pub target: Option<String>,
}

pub fn parse(input: &PackageInput) -> Result<AgentPackage, CommandError> {
    let package = AgentPackage::parse(&input.content, format(input.format.as_deref())?)
        .map_err(package_error)?;
    package.files().map_err(package_error)?;
    Ok(package)
}

/// Resolve a session file/folder through the existing filesystem policy.
pub async fn parse_input(ctx: &Ctx, input: &PackageInput) -> Result<AgentPackage, CommandError> {
    let Some(path) = &input.file else {
        return parse(input);
    };
    if !input.content.is_empty() {
        return Err(CommandError::bad_request("choose content or file"));
    }
    let session = ctx
        .acting_for_session
        .ok_or_else(|| {
            CommandError::bad_request(
                "file requires a current session; MCP clients can send content instead",
            )
        })?
        .to_string();
    use crate::domains::session_files::{GetWorkspaceFile, types::GetResponse};
    let selected = GetWorkspaceFile {
        session_id: session.clone(),
        path: path.clone(),
        recursive: true,
    }
    .run(ctx)
    .await?;
    let listing = match selected {
        GetResponse::File(file) => {
            let bytes = SessionFile::decode_content(
                file.content.as_deref().unwrap_or_default(),
                &file.encoding,
            )
            .map_err(|e| CommandError::bad_request(e.to_string()))?;
            if bytes.starts_with(b"PK\x03\x04") || path.ends_with(".zip") {
                return AgentPackage::from_zip(&bytes).map_err(package_error);
            }
            let content = std::str::from_utf8(&bytes)
                .map_err(|e| CommandError::bad_request(e.to_string()))?;
            let requested = format(input.format.as_deref())?;
            let requested = if requested == Format::Auto {
                Format::from_extension(std::path::Path::new(path))
            } else {
                requested
            };
            let package = AgentPackage::parse(content, requested).map_err(package_error)?;
            let parent = std::path::Path::new(path)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("/");
            if package.files().is_ok() {
                let conventional = std::path::Path::new(path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        matches!(
                            name,
                            "agent.toml" | "agent.md" | "agent.yaml" | "agent.yml" | "agent.json"
                        )
                    });
                if conventional {
                    // Inspect sibling names only; read asset bytes through the
                    // same bounded folder loader as an explicit folder import.
                    let siblings = GetWorkspaceFile {
                        session_id: session.clone(),
                        path: parent.into(),
                        recursive: false,
                    }
                    .run(ctx)
                    .await?;
                    if let GetResponse::Listing(listing) = siblings
                        && listing.data.iter().any(|info| {
                            info.is_directory
                                && matches!(
                                    std::path::Path::new(&info.path)
                                        .file_name()
                                        .and_then(|name| name.to_str()),
                                    Some("files" | "skills" | ".agents")
                                )
                        })
                    {
                        return parse_folder(ctx, &session, parent).await;
                    }
                }
                return Ok(package);
            }
            return parse_folder(ctx, &session, parent).await;
        }
        GetResponse::Listing(listing) => listing,
    };
    read_folder(ctx, &session, path, listing.data).await
}

async fn parse_folder(ctx: &Ctx, session: &str, path: &str) -> Result<AgentPackage, CommandError> {
    use crate::domains::session_files::{GetWorkspaceFile, types::GetResponse};
    match (GetWorkspaceFile {
        session_id: session.into(),
        path: path.into(),
        recursive: true,
    })
    .run(ctx)
    .await?
    {
        GetResponse::Listing(listing) => read_folder(ctx, session, path, listing.data).await,
        _ => Err(CommandError::bad_request("expected agent folder")),
    }
}
async fn read_folder(
    ctx: &Ctx,
    session: &str,
    path: &str,
    files: Vec<everruns_core::session_file::FileInfo>,
) -> Result<AgentPackage, CommandError> {
    use crate::domains::session_files::{GetWorkspaceFile, types::GetResponse};
    let prefix = format!("{}/", path.trim_end_matches('/'));
    let mut entries = std::collections::BTreeMap::new();
    let manifests: Vec<_> = files
        .iter()
        .filter(|info| {
            info.path.strip_prefix(&prefix).is_some_and(|relative| {
                matches!(
                    relative,
                    "agent.toml" | "agent.md" | "agent.yaml" | "agent.yml" | "agent.json"
                )
            })
        })
        .collect();
    if manifests.len() != 1 {
        return Err(CommandError::bad_request(
            "folder must contain exactly one agent manifest",
        ));
    }
    let manifest_path = &manifests[0].path;
    let GetResponse::File(manifest_file) = (GetWorkspaceFile {
        session_id: session.into(),
        path: manifest_path.clone(),
        recursive: false,
    })
    .run(ctx)
    .await?
    else {
        return Err(CommandError::bad_request("expected agent manifest file"));
    };
    let manifest_bytes = SessionFile::decode_content(
        manifest_file.content.as_deref().unwrap_or_default(),
        &manifest_file.encoding,
    )
    .map_err(|e| CommandError::bad_request(e.to_string()))?;
    let package = AgentPackage::parse(
        std::str::from_utf8(&manifest_bytes)
            .map_err(|e| CommandError::bad_request(e.to_string()))?,
        Format::from_extension(std::path::Path::new(manifest_path)),
    )
    .map_err(package_error)?;
    let mut selected = package
        .referenced_paths(
            files
                .iter()
                .filter_map(|info| info.path.strip_prefix(&prefix)),
        )
        .map_err(package_error)?;
    selected.remove(manifest_path.strip_prefix(&prefix).unwrap_or_default());
    let mut total = manifest_bytes.len();
    entries.insert(
        manifest_path
            .strip_prefix(&prefix)
            .unwrap_or_default()
            .to_string(),
        manifest_bytes,
    );
    for info in files {
        if info.is_directory {
            continue;
        }
        let Some(relative) = info.path.strip_prefix(&prefix) else {
            continue;
        };
        if !selected.contains(relative) {
            continue;
        }
        if entries.len() >= everruns_core::agent_package::MAX_FILES + 2
            || info.size_bytes < 0
            || info.size_bytes as usize > everruns_core::agent_package::MAX_PACKAGE_BYTES
        {
            return Err(CommandError::bad_request("agent folder exceeds limits"));
        }
        let GetResponse::File(file) = (GetWorkspaceFile {
            session_id: session.into(),
            path: info.path.clone(),
            recursive: false,
        })
        .run(ctx)
        .await?
        else {
            continue;
        };
        let bytes = SessionFile::decode_content(
            file.content.as_deref().unwrap_or_default(),
            &file.encoding,
        )
        .map_err(|e| CommandError::bad_request(e.to_string()))?;
        total += bytes.len();
        if total > everruns_core::agent_package::MAX_PACKAGE_BYTES {
            return Err(CommandError::bad_request("agent folder exceeds 10 MiB"));
        }
        entries.insert(relative.to_string(), bytes);
    }
    AgentPackage::from_entries(entries).map_err(package_error)
}

pub async fn export(ctx: &Ctx, id: &str) -> Result<AgentPackage, CommandError> {
    let agent = GetAgent { id: id.into() }.run(ctx).await?;
    let mut value = serde_json::to_value(&agent).map_err(|e| CommandError::internal(e.into()))?;
    let object = value.as_object_mut().expect("agent object");
    let allowed = [
        "name",
        "display_name",
        "description",
        "system_prompt",
        "tags",
        "capabilities",
        "initial_files",
        "mcpServers",
        "mcp_servers",
        "network_access",
        "max_iterations",
        "parallel_tool_calls",
        "tools",
        "intro_markdown",
        "short_description",
        "starters",
        "sandbox_policy",
    ];
    object.retain(|key, _| allowed.contains(&key.as_str()));
    object.insert("schema_version".into(), json!(1));
    let instructions = object.remove("system_prompt").unwrap_or(json!(""));
    object.insert("instructions".into(), instructions);
    let harness = ctx
        .db
        .get_harness(ctx.org_id(), agent.harness_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Harness"))?;
    object.insert("harness".into(), json!(harness.name));
    if let Some(id) = agent.default_model_id {
        let model = ctx
            .db
            .get_model_with_provider(ctx.org_id(), id.uuid())
            .await?
            .ok_or_else(|| CommandError::not_found("Model"))?;
        object.insert(
            "model".into(),
            json!(ModelSpec::on(model.provider_name, model.model_id)),
        );
    }
    // Redact before parsing so literal credential-bearing legacy attachments
    // become requirements, never a portable copy of a human's credentials.
    if let Some(servers) = object.get_mut("mcpServers").and_then(Value::as_object_mut) {
        for (name, server) in servers {
            for section in ["headers", "env"] {
                if let Some(values) = server.get_mut(section).and_then(Value::as_object_mut) {
                    for (key, value) in values {
                        if sensitive_key(key)
                            && !value
                                .as_str()
                                .is_some_and(everruns_core::agent_package::binding)
                        {
                            let binding = format!("MCP_{}_{}", name, key)
                                .to_ascii_uppercase()
                                .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
                            *value = json!(format!("${{{binding}}}"));
                        }
                    }
                }
            }
        }
    }
    let mut manifest: Manifest =
        serde_json::from_value(value).map_err(|e| CommandError::bad_request(e.to_string()))?;
    for server in manifest.mcp_servers.values_mut() {
        if let Some(provider) = server.oauth_provider_id.as_deref()
            && let Some(id) = provider.strip_prefix("mcp_oauth_")
        {
            let id = uuid::Uuid::parse_str(id)
                .map_err(|_| CommandError::bad_request("invalid MCP OAuth binding"))?;
            let catalog = ctx
                .db
                .get_mcp_server(ctx.org_id(), id)
                .await?
                .ok_or_else(|| CommandError::not_found("MCP server"))?;
            *server = ScopedMcpServer {
                preset: Some(
                    format!("catalog:{}", catalog.name)
                        .parse()
                        .map_err(CommandError::bad_request)?,
                ),
                acts_as: server.acts_as,
                protocol_mode: server.protocol_mode,
                elicitation_policy: server.elicitation_policy,
                tool_discovery: server.tool_discovery,
                ..Default::default()
            };
        }
    }
    let mut caps = Vec::new();
    for cap in &agent.capabilities {
        if let Some(id) =
            everruns_core::capabilities::parse_skill_capability_id(cap.capability_id())
        {
            let skill = ctx
                .db
                .get_skill(ctx.org_id(), id)
                .await?
                .ok_or_else(|| CommandError::not_found("Skill"))?;
            let body = crate::domains::skills::GetSkillContent {
                id: skill.public_id.clone(),
            }
            .run(ctx)
            .await?;
            manifest.initial_files.push(File::Inline(InitialFile {
                path: format!("/.agents/skills/{}/SKILL.md", skill.name),
                content: body.skill_md,
                encoding: "text".into(),
                is_readonly: true,
            }));
            for file in ctx.db.list_skill_files(id).await? {
                let bytes = file
                    .content_binary
                    .unwrap_or_else(|| file.content.unwrap_or_default().into_bytes());
                let (content, encoding) = SessionFile::encode_content(&bytes);
                manifest.initial_files.push(File::Inline(InitialFile {
                    path: format!("/.agents/skills/{}/{}", skill.name, file.path),
                    content,
                    encoding,
                    is_readonly: true,
                }));
            }
            if !caps
                .iter()
                .any(|c: &CapabilityRef| c.capability_id() == "skills")
            {
                caps.push(CapabilityRef::new("skills"));
            }
        } else if let Some(id) = everruns_core::mcp::parse_mcp_capability_id(cap.capability_id()) {
            let server = ctx
                .db
                .get_mcp_server(ctx.org_id(), id)
                .await?
                .ok_or_else(|| CommandError::not_found("MCP server"))?;
            manifest.mcp_servers.insert(
                server.name.clone(),
                ScopedMcpServer {
                    preset: Some(
                        format!("catalog:{}", server.name)
                            .parse::<everruns_core::McpServerPresetRef>()
                            .map_err(|e| CommandError::bad_request(e.to_string()))?,
                    ),
                    ..Default::default()
                },
            );
        } else if everruns_contracts::is_plugin_capability(cap.capability_id()) {
            return Err(CommandError::bad_request(
                "Installed plugins cannot be exported by ID. Describe their MCP servers or declarative capabilities explicitly.",
            ));
        } else {
            caps.push(cap.clone());
        }
    }
    caps.sort_by(|a, b| a.capability_id().cmp(b.capability_id()));
    caps.dedup_by(|a, b| a.capability_id() == b.capability_id());
    manifest.capabilities = caps;
    for channel in (crate::domains::agent_channels::ListAgentChannels {
        agent_id: agent.name.clone(),
    })
    .run(ctx)
    .await?
    {
        let mut config = channel.channel_config;
        let name = config
            .get("package_name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}-{}", channel.channel_type, manifest.channels.len() + 1));
        scrub_channel(&mut config);
        manifest.channels.insert(
            name,
            Channel {
                channel_type: channel.channel_type.to_string(),
                enabled: false,
                config,
            },
        );
    }
    AgentPackage::new(manifest).map_err(package_error)
}

fn scrub_channel(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, _| {
                !sensitive_key(key)
                    && key != "id"
                    && !key.ends_with("_id")
                    && !key.ends_with("_configured")
                    && key != "package_name"
                    && key != "api_key_prefix"
            });
            for value in map.values_mut() {
                scrub_channel(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                scrub_channel(value);
            }
        }
        _ => {}
    }
}

pub async fn request(
    ctx: &Ctx,
    package: &AgentPackage,
) -> Result<CreateAgentRequest, CommandError> {
    package.files().map_err(package_error)?;
    let m = &package.manifest;
    // Hosted credentials must use catalog bindings, never worker environment variables.
    package.bind_mcp(|_| None).map_err(package_error)?;
    let model_id = if let Some(spec) = &m.model {
        let models = ctx.db.list_all_models(ctx.org_id()).await?;
        let matches: Vec<_> = models
            .iter()
            .filter(|m| {
                m.enabled
                    && m.model_id == spec.model
                    && ProviderKey::new(&m.provider_name) == spec.provider
            })
            .collect();
        if matches.len() != 1 {
            return Err(CommandError::bad_request(format!(
                "model: expected one enabled {}/{} model at the destination; found {}",
                spec.provider,
                spec.model,
                matches.len()
            )));
        }
        Some(matches[0].id)
    } else {
        package.legacy_model_id
    };
    let req = CreateAgentRequest {
        service_virtual_user_id: None,
        id: None,
        name: m.name.clone(),
        display_name: m.display_name.clone(),
        description: m.description.clone(),
        intro_markdown: m.intro_markdown.clone(),
        short_description: m.short_description.clone(),
        starters: serde_json::from_value(json!(m.starters))
            .map_err(|e| CommandError::bad_request(format!("starters: {e}")))?,
        system_prompt: m.instructions.clone(),
        default_model_id: model_id,
        harness_id: package.legacy_harness_id,
        harness_name: m.harness.clone(),
        tags: m.tags.clone(),
        capabilities: m.capabilities.clone(),
        sandbox_policy: m
            .sandbox_policy
            .clone()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| CommandError::bad_request(format!("sandbox_policy: {e}")))?,
        initial_files: package.files().map_err(package_error)?,
        tools: m.tools.clone(),
        mcp_servers: m.mcp_servers.clone(),
        network_access: m.network_access.clone(),
        max_iterations: m.max_iterations,
        parallel_tool_calls: m.parallel_tool_calls,
    };
    crate::api::validation::validate_create_agent_input(
        &req.name,
        req.display_name.as_deref(),
        req.description.as_deref(),
        &req.system_prompt,
        req.capabilities.len(),
        &req.initial_files,
    )
    .map_err(|_| CommandError::bad_request("Agent configuration exceeds platform limits"))?;
    super::managed::validate_managed_name(&req.name)?;
    crate::api::validation::check_platform_chat_content(
        req.intro_markdown.as_deref(),
        req.short_description.as_deref(),
        &req.starters,
    )
    .map_err(CommandError::bad_request)?;
    super::sandbox_policy::validate(req.sandbox_policy.as_ref())?;
    super::command_validation::validate_sandbox_template_sources(ctx, req.sandbox_policy.as_ref())
        .await?;
    super::managed::check_high_risk_caps(ctx, &req.capabilities).await?;
    let caps = crate::domains::capabilities::validation::normalize_capability_refs(
        &ctx.db,
        ctx.org_id(),
        req.capabilities.clone(),
    )
    .await?;
    crate::domains::capabilities::validation::validate_capability_refs(
        &ctx.db,
        ctx.org_id(),
        &caps,
    )
    .await?;
    crate::domains::capabilities::validation::validate_feature_gated_capability_refs(
        &ctx.feature_flags,
        &caps,
    )?;
    everruns_core::capabilities::resolve_capability_configs(
        &caps,
        ctx.capability_service.registry(),
    )
    .map_err(|e| CommandError::bad_request(e.to_string()))?;
    crate::domains::mcp_servers::scoped_mcp::validate_scoped_mcp_servers_for_org(
        &ctx.db,
        ctx.org_id(),
        &req.mcp_servers,
    )
    .await?;
    let harness =
        super::managed::resolve_create_harness_id(ctx, req.harness_id, req.harness_name.as_deref())
            .await?;
    super::managed::check_harness_assignment(ctx, harness).await?;
    for (name, channel) in &m.channels {
        if !matches!(
            channel.channel_type.as_str(),
            "ag_ui" | "public_chat" | "fcp" | "slack" | "voice"
        ) {
            return Err(CommandError::bad_request(format!(
                "channels.{name}: package imports support ag_ui, public_chat, fcp, slack and voice; use the channel/trigger API for this type"
            )));
        }
        if channel.channel_type == "public_chat" && !ctx.feature_flags.public_chat {
            return Err(CommandError::feature_not_enabled("public_chat"));
        }
        if channel.channel_type == "voice" && !ctx.feature_flags.voice {
            return Err(CommandError::feature_not_enabled("voice"));
        }
        crate::domains::agent_channels::validation::normalize_and_validate_channel_config(
            ChannelType::from_str_opt(&channel.channel_type).expect("validated channel type"),
            channel.config.clone(),
        )?;
    }
    Ok(req)
}

async fn destination(
    ctx: &Ctx,
    package: &AgentPackage,
    target: Option<&str>,
) -> Result<Option<Agent>, CommandError> {
    let existing = if let Some(target) = target {
        Some(GetAgent { id: target.into() }.run(ctx).await?)
    } else if let Some(id) = package.legacy_id {
        match (GetAgent { id: id.to_string() }).run(ctx).await {
            Ok(agent) => Some(agent),
            Err(e) if e.status() == axum::http::StatusCode::NOT_FOUND => None,
            Err(e) => return Err(e),
        }
    } else {
        None
    };
    if let Some(agent) = &existing
        && agent.status != crate::records::AgentStatus::Active
    {
        return Err(CommandError::bad_request("target agent must be active"));
    }
    super::queries::ensure_name_available(
        &ctx.db,
        ctx.org_id(),
        &package.manifest.name,
        existing
            .as_ref()
            .map(|a| everruns_contracts::typed_id::AgentId::from_uuid(a.internal_id)),
    )
    .await?;
    if existing.is_none()
        && ctx.db.count_agents_for_org(ctx.org_id()).await?
            >= ctx.resource_limits.max_agents_per_org
    {
        return Err(CommandError::conflict("Agent limit reached"));
    }
    if let Some(agent) = &existing {
        let channels = crate::domains::agent_channels::ListAgentChannels {
            agent_id: agent.name.clone(),
        }
        .run(ctx)
        .await?;
        for (name, channel) in &package.manifest.channels {
            if channels.iter().any(|c| {
                c.channel_config.get("package_name").and_then(Value::as_str) == Some(name)
                    && c.channel_type.to_string() != channel.channel_type
            }) {
                return Err(CommandError::bad_request(format!(
                    "channels.{name}: cannot change channel type"
                )));
            }
            if let Some(current) = channels.iter().find(|c| {
                c.channel_config.get("package_name").and_then(Value::as_str) == Some(name)
            }) {
                let mut config = channel.config.clone();
                config["package_name"] = json!(name);
                crate::domains::agent_channels::commands::preflight_package_channel_config(
                    ctx,
                    agent.internal_id,
                    &current.public_id.to_string(),
                    config,
                    channel.enabled.then_some(true),
                )
                .await?;
            }
        }
    }
    Ok(existing)
}

pub async fn apply(
    ctx: &Ctx,
    package: &AgentPackage,
    target: Option<&str>,
) -> Result<(Agent, bool), CommandError> {
    let req = request(ctx, package).await?;
    let resolved = destination(ctx, package, target).await?;
    let destination = resolved.as_ref().map(|a| a.public_id).or(package.legacy_id);
    let existing = if let Some(id) = resolved.as_ref().map(|a| a.public_id) {
        crate::domains::agent_channels::ListAgentChannels {
            agent_id: id.to_string(),
        }
        .run(ctx)
        .await?
    } else {
        Vec::new()
    };
    for (name, channel) in &package.manifest.channels {
        if existing.iter().any(|c| {
            c.channel_config.get("package_name").and_then(Value::as_str) == Some(name)
                && c.channel_type.to_string() != channel.channel_type
        }) {
            return Err(CommandError::bad_request(format!(
                "channels.{name}: cannot change channel type"
            )));
        }
    }
    let (agent, created) = if let Some(id) = destination {
        let result = UpsertAgent {
            replace_capabilities: true,
            id: id.to_string(),
            req,
        }
        .run(ctx)
        .await?;
        (result.agent, result.was_created)
    } else {
        (CreateAgent(req).run(ctx).await?, true)
    };
    for (name, channel) in &package.manifest.channels {
        let mut config = channel.config.clone();
        config["package_name"] = json!(name);
        if let Some(current) = existing
            .iter()
            .find(|c| c.channel_config.get("package_name").and_then(Value::as_str) == Some(name))
        {
            if current.channel_type.to_string() != channel.channel_type {
                return Err(CommandError::bad_request(format!(
                    "channels.{name}: cannot change channel type"
                )));
            }
            crate::domains::agent_channels::UpdateAgentChannelCmd {
                agent_id: agent.name.clone(),
                channel_id: current.public_id.to_string(),
                req: crate::domains::agent_channels::types::UpdateAgentChannelRequest {
                    channel_config: Some(config),
                    // Disabled intent preserves destination activation; enabling stays draft.
                    enabled: channel.enabled.then_some(true),
                },
            }
            .run(ctx)
            .await?;
        } else {
            crate::domains::agent_channels::CreateAgentChannel {
                agent_id: agent.name.clone(),
                req: crate::domains::agent_channels::types::CreateAgentChannelRequest {
                    channel_type: ChannelType::from_str_opt(&channel.channel_type)
                        .expect("validated channel type"),
                    channel_config: config,
                    enabled: channel.enabled,
                },
            }
            .run(ctx)
            .await?;
        }
    }
    Ok((agent, created))
}

/// Export a portable definition, optionally writing an artifact to the current session.
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ExportAgent {
    /// Agent name (legacy IDs are also accepted).
    pub id: String,
    /// Output format; defaults to JSON for responses and Markdown for artifacts.
    #[serde(default)]
    pub format: Option<String>,
    /// New artifact path in the current session workspace.
    #[serde(default)]
    pub out: Option<String>,
}
impl Command for ExportAgent {
    type Output = Value;
    fn meta() -> CommandMeta {
        CommandMeta {
            name: "export_agent",
            category: "agents",
            description: "Export a portable agent definition as JSON, without IDs or credentials.",
            method: "GET",
            path: "/v1/agents/{id}/export",
        }
    }
    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_VIEW)
    }
    // Artifact output writes the session filesystem. Keep this combined command
    // out of MCP's read-only query tool; REST downloads remain read-only.
    fn read_only() -> bool {
        false
    }
    fn cli() -> Option<CliRoute> {
        {
            const ARGS: &[CliArg] = &[CliArg::new("id").at(1)];
            const ROUTE: CliRoute = CliRoute::new(&["agents"], "export")
                .with_args(ARGS)
                .with_examples(&[CliExample::new(
                    "Export portable configuration by agent name",
                    "everruns agents export triage --format json",
                )]);
            Some(ROUTE)
        }
    }
    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let package = export(ctx, &self.id).await?;
        if let Some(path) = self.out {
            let session = ctx.acting_for_session.ok_or_else(|| {
                CommandError::bad_request("out requires a current session workspace")
            })?;
            let bytes = if self.format.as_deref() == Some("zip") {
                package.to_zip().map_err(package_error)?
            } else {
                package
                    .to_string(format(self.format.as_deref().or(Some("markdown")))?)
                    .map_err(package_error)?
                    .into_bytes()
            };
            let (content, encoding) = SessionFile::encode_content(&bytes);
            crate::domains::session_files::CreateWorkspaceFile {
                session_id: session.to_string(),
                path: path.clone(),
                req: crate::api::session_files::CreateFileRequest {
                    content: Some(content),
                    encoding: Some(encoding),
                    is_readonly: Some(false),
                    is_directory: None,
                },
            }
            .run(ctx)
            .await?;
            return Ok(json!({"path":path,"name":package.manifest.name}));
        }
        if self.format.as_deref().is_some_and(|f| f != "json") {
            return Ok(
                json!({"content":package.to_string(format(self.format.as_deref())?).map_err(package_error)?}),
            );
        }
        serde_json::to_value(package.manifest).map_err(|e| CommandError::internal(e.into()))
    }
}
inventory::submit! { CommandDescriptor::of::<ExportAgent>() }

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum ImportAgent {
    Package(PackageInput),
    Legacy(Box<CreateAgentRequest>),
}
impl CommandSchema for ImportAgent {
    fn param_schema() -> Value {
        delegated_param_schema::<PackageInput>()
    }
}
impl Command for ImportAgent {
    type Output = Agent;
    fn meta() -> CommandMeta {
        CommandMeta {
            name: "import_agent",
            category: "agents",
            description: "Import an agent package; target explicitly updates an existing agent by name.",
            method: "POST",
            path: "/v1/agents/import",
        }
    }
    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_MANAGE)
    }
    fn cli() -> Option<CliRoute> {
        {
            const ARGS: &[CliArg] = &[CliArg::new("file").at(1)];
            const ROUTE: CliRoute = CliRoute::new(&["agents"], "import")
                .with_args(ARGS)
                .with_examples(&[CliExample::new(
                    "Import an agent folder from the current workspace",
                    "everruns agents import /agents/triage --reason 'Sync from the repo definition'",
                )]);
            Some(ROUTE)
        }
    }
    fn positional_arg() -> Option<&'static str> {
        Some("file")
    }
    async fn execute(self, ctx: &Ctx) -> Result<Agent, CommandError> {
        match self {
            Self::Package(input) => Ok(apply(
                ctx,
                &parse_input(ctx, &input).await?,
                input.target.as_deref(),
            )
            .await?
            .0),
            Self::Legacy(req) => CreateAgent(*req).run(ctx).await,
        }
    }
}
inventory::submit! { CommandDescriptor::of::<ImportAgent>() }

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct ValidateAgentPackage(pub PackageInput);
impl CommandSchema for ValidateAgentPackage {
    fn param_schema() -> Value {
        delegated_param_schema::<PackageInput>()
    }
}
impl Command for ValidateAgentPackage {
    type Output = Value;
    fn meta() -> CommandMeta {
        CommandMeta {
            name: "validate_agent_package",
            category: "agents",
            description: "Validate an agent package and destination dependencies without changing resources.",
            method: "POST",
            path: "/v1/agents/validate",
        }
    }
    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_VIEW)
    }
    fn read_only() -> bool {
        true
    }
    fn cli() -> Option<CliRoute> {
        {
            const ARGS: &[CliArg] = &[CliArg::new("file").at(1)];
            const ROUTE: CliRoute = CliRoute::new(&["agents"], "validate")
                .with_args(ARGS)
                .with_examples(&[CliExample::new(
                    "Validate a workspace package without changing resources",
                    "everruns agents validate /agents/triage",
                )]);
            Some(ROUTE)
        }
    }
    fn positional_arg() -> Option<&'static str> {
        Some("file")
    }
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        if self.0.file.is_none() {
            let format = format(self.0.format.as_deref())?;
            match AgentPackage::parse(&self.0.content, format).and_then(|p| {
                p.files()?;
                Ok(p)
            }) {
                Ok(_) => {}
                Err(error) => return Ok(json!({"valid":false,"diagnostics":error.0})),
            }
        }
        match parse_input(ctx, &self.0).await {
            Ok(package) => {
                // Preview only authored values and file digests, never asset bodies
                // or resolved destination credentials. Available even if binding fails.
                let preview = package.canonical().map_err(package_error)?;
                match async {
                    request(ctx, &package).await?;
                    destination(ctx, &package, self.0.target.as_deref()).await?;
                    Ok::<_, CommandError>(())
                }
                .await
                {
                    Ok(_) => Ok(
                        json!({"valid":true,"diagnostics":[],"name":package.manifest.name,"preview":preview}),
                    ),
                    Err(e) => Ok(
                        json!({"valid":false,"preview":preview,"diagnostics":[{"path":"dependencies","message":e.message()}]}),
                    ),
                }
            }
            Err(e) => {
                Ok(json!({"valid":false,"diagnostics":[{"path":"manifest","message":e.message()}]}))
            }
        }
    }
}
inventory::submit! { CommandDescriptor::of::<ValidateAgentPackage>() }

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct DiffAgentPackage(pub PackageInput);
impl CommandSchema for DiffAgentPackage {
    fn param_schema() -> Value {
        delegated_param_schema::<PackageInput>()
    }
}
impl Command for DiffAgentPackage {
    type Output = Value;
    fn meta() -> CommandMeta {
        CommandMeta {
            name: "diff_agent_package",
            category: "agents",
            description: "Compare an agent package with an existing agent by name without applying changes.",
            method: "POST",
            path: "/v1/agents/diff",
        }
    }
    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&AGENT_VIEW)
    }
    fn read_only() -> bool {
        true
    }
    fn cli() -> Option<CliRoute> {
        {
            const ARGS: &[CliArg] = &[CliArg::new("file").at(1)];
            const ROUTE: CliRoute = CliRoute::new(&["agents"], "diff")
                .with_args(ARGS)
                .with_examples(&[CliExample::new(
                    "Review package changes against an existing agent",
                    "everruns agents diff /agents/triage --target triage",
                )]);
            Some(ROUTE)
        }
    }
    fn positional_arg() -> Option<&'static str> {
        Some("file")
    }
    async fn execute(self, ctx: &Ctx) -> Result<Value, CommandError> {
        let target = self
            .0
            .target
            .as_deref()
            .ok_or_else(|| CommandError::bad_request("target agent name is required"))?;
        let mut package = parse_input(ctx, &self.0).await?;
        let req = request(ctx, &package).await?;
        destination(ctx, &package, Some(target)).await?;
        if package.manifest.harness.is_none() {
            let id = super::managed::resolve_create_harness_id(
                ctx,
                req.harness_id,
                req.harness_name.as_deref(),
            )
            .await?;
            package.manifest.harness = Some(
                ctx.db
                    .get_harness(ctx.org_id(), id)
                    .await?
                    .ok_or_else(|| CommandError::not_found("Harness"))?
                    .name,
            );
        }
        let before = export(ctx, target).await?;
        // Import upserts channel declarations and preserves omitted bindings.
        for (name, channel) in &before.manifest.channels {
            package
                .manifest
                .channels
                .entry(name.clone())
                .or_insert_with(|| channel.clone());
        }
        let changes = before.diff(&package).map_err(package_error)?;
        Ok(json!({"target":target,"changes":changes,"changed":!changes.is_empty()}))
    }
}
inventory::submit! { CommandDescriptor::of::<DiffAgentPackage>() }

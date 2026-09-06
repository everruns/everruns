// Plugin compiler: PluginFileSet → CompiledPlugin
//
// Maps plugin directory components to a DeclarativeCapabilityDefinition per
// the table in knowledge/integrations/plugins.md.

use std::collections::BTreeMap;

use crate::capabilities::{
    CapabilityStatus, DeclarativeCapabilityDefinition, DeclarativeCapabilityFile,
    DeclarativeCapabilitySkill, DeclarativeCapabilitySkillFile,
    validate_declarative_capability_definition,
};
use crate::mcp_server::{
    McpServerAuthMode, McpServerTransportType, ScopedMcpServer, ScopedMcpServers,
};

use super::file_set::PluginFileSet;
use super::manifest::{
    AGENT_PLUGINS_V1_MCP_SCHEMA, AGENT_PLUGINS_V1_MCP_SCHEMA_JSON, McpServersField, PluginManifest,
    validate_json_schema,
};
use base64::Engine;

// Legacy host plugin names retain the original 43-byte compatibility limit.
const PLUGIN_CAPABILITY_PREFIX: &str = "plugin:";
const MAX_PLUGIN_NAME_BYTES: usize = 50 - PLUGIN_CAPABILITY_PREFIX.len(); // 43
const MAX_AGENT_PLUGIN_NAME_BYTES: usize = 64;

/// Result of compiling a plugin directory.
#[derive(Debug, Clone)]
pub struct CompiledPlugin {
    /// Parsed manifest.
    pub manifest: PluginManifest,
    /// Compiled declarative capability definition.
    pub definition: DeclarativeCapabilityDefinition,
    /// Non-fatal install warnings collected during compilation.
    pub warnings: Vec<String>,
}

/// Compile a `PluginFileSet` into a `CompiledPlugin`.
///
/// Maps each plugin component to the corresponding capability contribution per
/// the component-mapping table in `knowledge/integrations/plugins.md`. Errors are returned when
/// compilation cannot produce a valid `DeclarativeCapabilityDefinition`.
pub fn compile_plugin(file_set: &PluginFileSet) -> Result<CompiledPlugin, String> {
    let (manifest, mut warnings) = file_set.manifest()?;

    // --- name ---
    let is_agent_plugins_v1 = manifest.is_agent_plugins_v1();
    let name = if is_agent_plugins_v1 {
        validate_agent_plugins_name(&manifest.name)?
    } else {
        sanitize_plugin_name(&manifest.name)?
    };
    if !is_agent_plugins_v1 && name.len() > MAX_PLUGIN_NAME_BYTES {
        return Err(format!(
            "plugin name '{}' is {} bytes but must fit in {} bytes (plugin: prefix occupies {} bytes)",
            name,
            name.len(),
            MAX_PLUGIN_NAME_BYTES,
            PLUGIN_CAPABILITY_PREFIX.len()
        ));
    }

    // --- description (required) ---
    let description = match manifest
        .description
        .clone()
        .filter(|d| !d.trim().is_empty())
    {
        Some(description) => description,
        None if is_agent_plugins_v1 => format!("Agent plugin {name}"),
        None => {
            return Err("plugin manifest is missing a 'description' field".to_string());
        }
    };

    // --- display_name ---
    let display_name = manifest
        .display_name
        .clone()
        .filter(|d| !d.trim().is_empty());

    // --- agents → system_prompt ---
    let system_prompt = compile_agents(file_set, &manifest, &mut warnings);

    // --- skills ---
    let skills = compile_skills(file_set, &manifest, &mut warnings);

    // --- commands → user-invocable skills ---
    let command_skills = compile_commands(file_set, &manifest, &mut warnings);

    let mut all_skills = skills;
    all_skills.extend(command_skills);

    // --- MCP servers ---
    let mcp_servers = compile_mcp_servers(file_set, &manifest, &mut warnings)?;

    // Warn about unsupported component fields in the manifest.
    for ignored_field in &["hooks", "lspServers", "monitors", "themes", "outputStyles"] {
        if manifest.extra.contains_key(*ignored_field) {
            warnings.push(format!(
                "plugin manifest: '{ignored_field}' is not supported in v1 and will be ignored"
            ));
        }
    }

    let icon = compile_icon(file_set, &manifest, &mut warnings);

    let definition = DeclarativeCapabilityDefinition {
        name: name.clone(),
        display_name,
        description,
        status: CapabilityStatus::Available,
        icon,
        category: Some("Plugin".to_string()),
        system_prompt,
        mcp_servers,
        skills: all_skills,
        files: Vec::<DeclarativeCapabilityFile>::new(),
        dependencies: Vec::new(),
        features: Vec::new(),
        risk_level: crate::capabilities::RiskLevel::Low,
    };

    // Run through declarative validation to catch size/count violations.
    // The declarative capability validator intentionally retains its narrower
    // org-authored naming contract. Agent Plugins names are validated above,
    // so use a neutral name while reusing every content and size check.
    let mut validation_definition = definition.clone();
    if is_agent_plugins_v1 {
        validation_definition.name = "plugin".to_string();
    }
    validate_declarative_capability_definition(&validation_definition)
        .map_err(|e| format!("compiled plugin failed declarative validation: {e}"))?;

    Ok(CompiledPlugin {
        manifest,
        definition,
        warnings,
    })
}

fn compile_icon(
    file_set: &PluginFileSet,
    manifest: &PluginManifest,
    warnings: &mut Vec<String>,
) -> Option<String> {
    let Some(path) = manifest.icon.as_deref() else {
        return Some("puzzle".to_string());
    };
    let path = path.trim().trim_start_matches("./");
    if path.is_empty()
        || path.starts_with('/')
        || path.contains("..")
        || path.contains("://")
        || path.starts_with("data:")
        || !path.to_ascii_lowercase().ends_with(".svg")
    {
        warnings.push(format!(
            "plugin manifest: icon '{path}' must be a relative path to a bundled SVG; using the plugin fallback"
        ));
        return Some("puzzle".to_string());
    }

    let Some(svg) = file_set.text_file(path) else {
        warnings.push(format!(
            "plugin manifest: icon '{path}' is missing or is not UTF-8; using the plugin fallback"
        ));
        return Some("puzzle".to_string());
    };
    if let Err(reason) = validate_plugin_svg(&svg) {
        warnings.push(format!(
            "plugin manifest: icon '{path}' is unsafe or malformed ({reason}); using the plugin fallback"
        ));
        return Some("puzzle".to_string());
    }

    let encoded = base64::engine::general_purpose::STANDARD.encode(svg.as_bytes());
    Some(format!("data:image/svg+xml;base64,{encoded}"))
}

fn validate_plugin_svg(svg: &str) -> Result<(), &'static str> {
    let trimmed = svg.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("<svg") || !lower.ends_with("</svg>") {
        return Err("expected an SVG root element");
    }

    // SVG is rendered in an <img>, but reject active/external content before
    // persisting it as a second defense against browser behavior changes.
    const FORBIDDEN: &[&str] = &[
        "<script",
        "<style",
        "<foreignobject",
        "<iframe",
        "<object",
        "<embed",
        "<image",
        "<use",
        "<!doctype",
        "<?xml",
        "href=",
        "src=",
        "url(",
        "javascript:",
        "data:",
        "@import",
    ];
    if FORBIDDEN.iter().any(|needle| lower.contains(needle)) {
        return Err("active or external content is not allowed");
    }
    if lower.contains(" on") || lower.contains("\non") || lower.contains("\ton") {
        return Err("event handler attributes are not allowed");
    }
    Ok(())
}

/// Validate and normalize a plugin name into the format accepted by the
/// declarative name validator (lowercase, hyphens allowed, starts with letter).
fn sanitize_plugin_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("plugin name is empty".to_string());
    }
    // Validate: must start with lowercase letter, only [a-z0-9_-], no trailing -/_
    let mut chars = trimmed.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_lowercase() {
        return Err(format!(
            "plugin name '{}' must start with a lowercase letter",
            trimmed
        ));
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return Err(format!(
            "plugin name '{trimmed}' may only contain lowercase letters, digits, '-', and '_'"
        ));
    }
    if trimmed.ends_with('-') || trimmed.ends_with('_') {
        return Err(format!(
            "plugin name '{trimmed}' must not end with '-' or '_'"
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_agent_plugins_name(name: &str) -> Result<String, String> {
    if name.is_empty() || name.len() > MAX_AGENT_PLUGIN_NAME_BYTES {
        return Err(format!(
            "Agent Plugins name must be between 1 and {MAX_AGENT_PLUGIN_NAME_BYTES} characters"
        ));
    }
    let bytes = name.as_bytes();
    let is_alphanumeric = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    if !is_alphanumeric(bytes[0]) || !is_alphanumeric(bytes[bytes.len() - 1]) {
        return Err(
            "Agent Plugins name must start and end with a lowercase letter or digit".into(),
        );
    }
    if !bytes
        .iter()
        .all(|byte| is_alphanumeric(*byte) || matches!(*byte, b'-' | b'.'))
    {
        return Err(
            "Agent Plugins name may contain only lowercase letters, digits, '-' and '.'".into(),
        );
    }
    if name.contains("--") || name.contains("..") {
        return Err("Agent Plugins name cannot contain '--' or '..'".into());
    }
    Ok(name.to_string())
}

// ============================================================================
// Agents → system_prompt
// ============================================================================

/// Render agent files into a combined system prompt.
///
/// Each `.md` file under `agents/` (or the manifest-overridden path) is
/// rendered as a named `<agent>` XML section per knowledge/project/xml-prompt-formatting.md.
fn compile_agents(
    file_set: &PluginFileSet,
    manifest: &PluginManifest,
    _warnings: &mut Vec<String>,
) -> Option<String> {
    let agent_dirs = match &manifest.agents {
        Some(paths) => resolve_component_paths(paths),
        None => vec!["agents".to_string()],
    };

    let mut sections: Vec<String> = Vec::new();

    for agent_dir in &agent_dirs {
        let dir = strip_dot_slash(agent_dir);
        // List .md files directly under this directory.
        let mut entries: Vec<(&str, &str)> = file_set.list_dir(dir);
        entries.sort_by_key(|(name, _)| *name);

        for (filename, full_path) in entries {
            if !filename.ends_with(".md") {
                continue;
            }
            let Some(content) = file_set.text_file(full_path) else {
                continue;
            };

            // Parse frontmatter name/description if present.
            let (fm_name, fm_desc, body) = parse_simple_frontmatter(&content);

            // Use frontmatter `name` if available; fall back to filename stem.
            let agent_name =
                fm_name.unwrap_or_else(|| filename.trim_end_matches(".md").to_string());

            let mut section = format!("<agent name=\"{}\"", escape_attr(&agent_name));
            if let Some(desc) = fm_desc {
                section.push_str(&format!(" description=\"{}\"", escape_attr(&desc)));
            }
            section.push_str(">\n");
            section.push_str(body.trim());
            section.push_str("\n</agent>");
            sections.push(section);
        }
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

// ============================================================================
// Skills → DeclarativeCapabilitySkill
// ============================================================================

fn compile_skills(
    file_set: &PluginFileSet,
    manifest: &PluginManifest,
    warnings: &mut Vec<String>,
) -> Vec<DeclarativeCapabilitySkill> {
    if manifest.is_agent_plugins_v1() && file_set.files.contains_key("skills") {
        warnings.push(
            "skills exists but is not a directory; the skills component was disabled".to_string(),
        );
        return Vec::new();
    }

    let skill_dirs = match &manifest.skills {
        Some(paths) => resolve_component_paths(paths),
        None => vec!["skills".to_string()],
    };

    let mut skills = Vec::new();

    for skill_dir in &skill_dirs {
        let dir = strip_dot_slash(skill_dir);
        // Enumerate immediate subdirectories of the skills dir by finding
        // files under `dir/` and extracting the first path component.
        let prefix = format!("{dir}/");
        let mut seen_subdirs = std::collections::BTreeSet::new();
        for key in file_set.files.keys() {
            if let Some(rest) = key.strip_prefix(&prefix)
                && let Some(slash_pos) = rest.find('/')
            {
                seen_subdirs.insert(rest[..slash_pos].to_string());
            }
        }

        for subdir_name in &seen_subdirs {
            let skill_path = format!("{dir}/{subdir_name}");
            let skill_md_path = format!("{skill_path}/SKILL.md");

            let Some(skill_md_content) = file_set.text_file(&skill_md_path) else {
                continue;
            };

            // Parse SKILL.md using the existing parser.
            match crate::skill::parse_skill_md(&skill_md_content) {
                Ok(parsed) => {
                    // Collect sibling files (non-SKILL.md, text only).
                    let mut skill_files = Vec::new();
                    let all_skill_files = file_set.list_dir_recursive(&skill_path);
                    for file_path in all_skill_files {
                        if file_path == skill_md_path {
                            continue;
                        }
                        let rel_within_skill = file_path
                            .strip_prefix(&format!("{skill_path}/"))
                            .unwrap_or(file_path);

                        if let Some(bytes) = file_set.files.get(file_path) {
                            match String::from_utf8(bytes.clone()) {
                                Ok(text) => {
                                    skill_files.push(DeclarativeCapabilitySkillFile {
                                        path: rel_within_skill.to_string(),
                                        content: text,
                                    });
                                }
                                Err(_) => {
                                    warnings.push(format!(
                                        "skill '{}': binary file '{}' skipped (text only)",
                                        parsed.name, rel_within_skill
                                    ));
                                }
                            }
                        }
                    }

                    skills.push(DeclarativeCapabilitySkill {
                        name: parsed.name,
                        description: parsed.description,
                        instructions: parsed.instructions,
                        files: skill_files,
                        user_invocable: parsed.user_invocable,
                        disable_model_invocation: parsed.disable_model_invocation,
                    });
                }
                Err(errors) => {
                    warnings.push(format!(
                        "skill '{}': SKILL.md parse errors — {}: skill skipped",
                        subdir_name,
                        errors.join("; ")
                    ));
                }
            }
        }
    }

    skills
}

// ============================================================================
// Commands → user-invocable DeclarativeCapabilitySkill
// ============================================================================

fn compile_commands(
    file_set: &PluginFileSet,
    manifest: &PluginManifest,
    _warnings: &mut Vec<String>,
) -> Vec<DeclarativeCapabilitySkill> {
    let command_dirs = match &manifest.commands {
        Some(paths) => resolve_component_paths(paths),
        None => vec!["commands".to_string()],
    };

    let mut skills = Vec::new();

    for command_dir in &command_dirs {
        let dir = strip_dot_slash(command_dir);
        let mut entries: Vec<(&str, &str)> = file_set.list_dir(dir);
        entries.sort_by_key(|(name, _)| *name);

        for (filename, full_path) in entries {
            if !filename.ends_with(".md") {
                continue;
            }

            let Some(content) = file_set.text_file(full_path) else {
                continue;
            };

            let (fm_name, fm_desc, body) = parse_simple_frontmatter(&content);
            let stem = filename.trim_end_matches(".md");
            let name = fm_name.unwrap_or_else(|| stem.to_string());
            let description = fm_desc.unwrap_or_else(|| format!("/{name} command"));

            skills.push(DeclarativeCapabilitySkill {
                name,
                description,
                instructions: body.trim().to_string(),
                files: Vec::new(),
                user_invocable: true,
                disable_model_invocation: false,
            });
        }
    }

    skills
}

// ============================================================================
// MCP Servers
// ============================================================================

/// Parse and compile MCP server configuration.
///
/// v1 supports HTTP transport only. Stdio entries produce a warning and are skipped.
fn compile_mcp_servers(
    file_set: &PluginFileSet,
    manifest: &PluginManifest,
    warnings: &mut Vec<String>,
) -> Result<Option<ScopedMcpServers>, String> {
    if manifest.is_agent_plugins_v1() {
        return Ok(compile_agent_plugins_v1_mcp(file_set, manifest, warnings));
    }

    // Resolve where to look for MCP config.
    let mcp_source = match &manifest.mcp_servers {
        Some(McpServersField::Path(path)) => {
            // Load the referenced file.
            let p = strip_dot_slash(path);
            match file_set.text_file(p) {
                Some(content) => McpConfigSource::File(content),
                None => return Ok(None),
            }
        }
        Some(McpServersField::Paths(paths)) => {
            // Merge all referenced files.
            let mut merged: BTreeMap<String, serde_json::Value> = BTreeMap::new();
            for path in paths {
                let p = strip_dot_slash(path);
                if let Some(content) = file_set.text_file(p) {
                    let parsed = parse_mcp_json_file(&content, p)?;
                    merged.extend(parsed);
                }
            }
            McpConfigSource::Map(merged)
        }
        Some(McpServersField::Inline(map)) => {
            McpConfigSource::Map(map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        }
        None => {
            // Default: look for `.mcp.json` in the plugin root.
            match file_set.text_file(".mcp.json") {
                Some(content) => McpConfigSource::File(content),
                None => return Ok(None),
            }
        }
    };

    let raw_map = match mcp_source {
        McpConfigSource::File(content) => parse_mcp_json_file(&content, ".mcp.json")?,
        McpConfigSource::Map(m) => m,
    };

    if raw_map.is_empty() {
        return Ok(None);
    }

    let mut servers = ScopedMcpServers::new();

    for (server_name, server_config) in raw_map {
        // Extract transport type.
        let transport_str = server_config
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("http");

        // Detect stdio by command presence or explicit "stdio" type.
        let has_command = server_config.get("command").is_some();
        let is_stdio = transport_str == "stdio" || has_command;

        if is_stdio {
            warnings.push(format!(
                "MCP server '{server_name}': stdio transport is not supported in v1 and will be skipped"
            ));
            continue;
        }

        let url = server_config
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Literal headers (sent only to the plugin's own server URL).
        let mut headers = std::collections::HashMap::new();
        if let Some(header_map) = server_config.get("headers").and_then(|v| v.as_object()) {
            for (header_name, header_value) in header_map {
                match header_value.as_str() {
                    Some(value) => {
                        headers.insert(header_name.clone(), value.to_string());
                    }
                    None => warnings.push(format!(
                        "MCP server '{server_name}': header '{header_name}' is not a string and will be ignored"
                    )),
                }
            }
        }

        // Authentication. `"auth": "oauth"` (alias `auth_mode`) is an Everruns
        // extension marking the server as OAuth-authenticated; other hosts
        // ignore it and negotiate OAuth at the protocol level. `api_key` is
        // rejected — a plugin package cannot carry key material.
        let auth_value = server_config
            .get("auth")
            .or_else(|| server_config.get("auth_mode"))
            .and_then(|v| v.as_str());
        let auth_mode = match auth_value.map(str::to_ascii_lowercase).as_deref() {
            Some("oauth") => McpServerAuthMode::OAuth,
            Some("none") | None => McpServerAuthMode::None,
            Some(other) => {
                warnings.push(format!(
                    "MCP server '{server_name}': auth mode '{other}' is not supported for plugin servers and will be ignored"
                ));
                McpServerAuthMode::None
            }
        };

        // A plugin must never bind to an existing OAuth provider — that would
        // let third-party plugin content read tokens connected for other
        // providers (e.g. github). The host assigns the provider id at
        // install time (see knowledge/integrations/plugins.md).
        if server_config.get("oauth_provider_id").is_some() {
            warnings.push(format!(
                "MCP server '{server_name}': 'oauth_provider_id' cannot be set by a plugin and will be ignored"
            ));
        }

        servers.insert(
            server_name,
            ScopedMcpServer {
                transport_type: McpServerTransportType::Http,
                url,
                headers,
                auth_mode,
                ..ScopedMcpServer::default()
            },
        );
    }

    if servers.is_empty() {
        Ok(None)
    } else {
        Ok(Some(servers))
    }
}

fn compile_agent_plugins_v1_mcp(
    file_set: &PluginFileSet,
    manifest: &PluginManifest,
    warnings: &mut Vec<String>,
) -> Option<ScopedMcpServers> {
    let content = file_set.text_file("mcp.json")?;
    let value: serde_json::Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!(
                "mcp.json is invalid JSON and was disabled: {error}"
            ));
            return None;
        }
    };
    let Some(object) = value.as_object() else {
        warnings.push("mcp.json must contain a JSON object; MCP was disabled".to_string());
        return None;
    };
    if object.get("$schema").and_then(serde_json::Value::as_str)
        != Some(AGENT_PLUGINS_V1_MCP_SCHEMA)
    {
        warnings.push(format!(
            "mcp.json uses an unsupported or mismatched schema; expected {AGENT_PLUGINS_V1_MCP_SCHEMA}"
        ));
        return None;
    }
    let Some(raw_servers) = object
        .get("mcpServers")
        .and_then(serde_json::Value::as_object)
    else {
        warnings.push("mcp.json is missing the required 'mcpServers' object".to_string());
        return None;
    };

    let mut top_level = value.clone();
    top_level["mcpServers"] = serde_json::json!({});
    if let Err(error) =
        validate_json_schema(AGENT_PLUGINS_V1_MCP_SCHEMA_JSON, &top_level, "mcp.json")
    {
        warnings.push(format!("{error}; MCP was disabled"));
        return None;
    }

    let extension_servers = manifest
        .extensions
        .get("com.everruns")
        .and_then(|extension| extension.get("mcpServers"))
        .and_then(serde_json::Value::as_object);
    let mut servers = ScopedMcpServers::new();

    for (server_name, server_config) in raw_servers {
        let entry_document = serde_json::json!({
            "$schema": AGENT_PLUGINS_V1_MCP_SCHEMA,
            "mcpServers": { server_name: server_config }
        });
        if let Err(error) = validate_json_schema(
            AGENT_PLUGINS_V1_MCP_SCHEMA_JSON,
            &entry_document,
            "MCP server entry",
        ) {
            warnings.push(format!("MCP server '{server_name}' was skipped: {error}"));
            continue;
        }

        match server_config
            .get("type")
            .and_then(serde_json::Value::as_str)
        {
            Some("stdio") => {
                warnings.push(format!(
                    "MCP server '{server_name}': stdio transport is not supported and was skipped"
                ));
                continue;
            }
            Some("sse") => {
                warnings.push(format!(
                    "MCP server '{server_name}': legacy SSE transport is not supported and was skipped"
                ));
                continue;
            }
            Some("streamable-http") => {}
            _ => unreachable!("the Agent Plugins MCP schema validates the transport"),
        }

        let url = server_config
            .get("url")
            .and_then(serde_json::Value::as_str)
            .expect("validated Streamable HTTP URL");
        if let Err(error) = validate_agent_plugins_remote_url(url) {
            warnings.push(format!("MCP server '{server_name}' was skipped: {error}"));
            continue;
        }
        let headers = server_config
            .get("headers")
            .and_then(serde_json::Value::as_object)
            .map(|headers| {
                headers
                    .iter()
                    .map(|(name, value)| {
                        (
                            name.clone(),
                            value.as_str().expect("validated header value").to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let auth_mode = extension_servers
            .and_then(|entries| entries.get(server_name))
            .and_then(|entry| entry.get("auth"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_ascii_lowercase)
            .and_then(|auth| match auth.as_str() {
                "oauth" => Some(McpServerAuthMode::OAuth),
                "none" => Some(McpServerAuthMode::None),
                other => {
                    warnings.push(format!(
                        "MCP server '{server_name}': unsupported com.everruns auth mode '{other}' was ignored"
                    ));
                    None
                }
            })
            .unwrap_or(McpServerAuthMode::None);

        servers.insert(
            server_name.clone(),
            ScopedMcpServer {
                transport_type: McpServerTransportType::Http,
                url: url.to_string(),
                headers,
                auth_mode,
                ..ScopedMcpServer::default()
            },
        );
    }

    (!servers.is_empty()).then_some(servers)
}

fn validate_agent_plugins_remote_url(raw_url: &str) -> Result<(), String> {
    let url = url::Url::parse(raw_url).map_err(|error| format!("invalid URL: {error}"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("remote MCP URL cannot contain user information".to_string());
    }
    if url.fragment().is_some() {
        return Err("remote MCP URL cannot contain a fragment".to_string());
    }
    let host = url
        .host()
        .ok_or_else(|| "remote MCP URL must contain a host".to_string())?;
    let is_loopback = match host {
        url::Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
        url::Host::Ipv4(address) => address.is_loopback(),
        url::Host::Ipv6(address) => address.is_loopback(),
    };
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_loopback => Ok(()),
        "http" => Err("non-loopback remote MCP URLs must use HTTPS".to_string()),
        _ => Err("remote MCP URL must use HTTP or HTTPS".to_string()),
    }
}

enum McpConfigSource {
    File(String),
    Map(BTreeMap<String, serde_json::Value>),
}

/// Parse a `.mcp.json` file and return the `mcpServers` object as a flat map.
fn parse_mcp_json_file(
    content: &str,
    path: &str,
) -> Result<BTreeMap<String, serde_json::Value>, String> {
    let value: serde_json::Value =
        serde_json::from_str(content).map_err(|e| format!("failed to parse {path}: {e}"))?;

    // Top-level `mcpServers` key (standard .mcp.json format).
    if let Some(servers) = value.get("mcpServers").and_then(|v| v.as_object()) {
        return Ok(servers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect());
    }

    // Fallback: treat the root object itself as the servers map.
    if let Some(obj) = value.as_object() {
        return Ok(obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    }

    Ok(BTreeMap::new())
}

// ============================================================================
// Helpers
// ============================================================================

/// Resolve a `StringOrArray` component path override to a list of dir strings.
fn resolve_component_paths(field: &super::manifest::StringOrArray) -> Vec<String> {
    field.to_vec()
}

/// Normalize a component path: strip leading `./` and trailing `/`.
fn strip_dot_slash(path: &str) -> &str {
    let p = path.strip_prefix("./").unwrap_or(path);
    p.trim_end_matches('/')
}

/// Escape a string for use in an XML attribute value.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Minimal YAML-style frontmatter parser for `name` and `description` fields
/// only. Returns `(name, description, body)`.
///
/// We don't use the full `serde_yaml` parser here because plugin agent/command
/// files may use different frontmatter schemas; we only need two fields.
fn parse_simple_frontmatter(content: &str) -> (Option<String>, Option<String>, &str) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (None, None, content);
    }
    let after_first = &trimmed[3..];
    let Some(closing) = after_first.find("\n---") else {
        return (None, None, content);
    };

    let fm_text = &after_first[..closing];
    let body_start = closing + 4;
    let body = if body_start < after_first.len() {
        after_first[body_start..].trim_start_matches('\n')
    } else {
        ""
    };

    let mut name = None;
    let mut description = None;
    for line in fm_text.lines() {
        if let Some(rest) = line.strip_prefix("name:") {
            name = Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        } else if let Some(rest) = line.strip_prefix("description:") {
            description = Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }

    (name, description, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_microsoft_docs_fixture() {
        let fixture = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/plugins/microsoft-docs"
        ));
        let file_set = PluginFileSet::from_dir(fixture).expect("load fixture");
        let compiled = compile_plugin(&file_set).expect("compile fixture");

        // --- name and display name ---
        assert_eq!(compiled.definition.name, "microsoft-docs");
        assert_eq!(
            compiled.definition.display_name.as_deref(),
            Some("Microsoft Docs")
        );

        // --- description ---
        assert!(!compiled.definition.description.is_empty());

        // --- MCP server ---
        let mcp = compiled
            .definition
            .mcp_servers
            .as_ref()
            .expect("mcp_servers");
        let server = mcp.get("microsoft-learn").expect("microsoft-learn server");
        assert_eq!(server.url, "https://learn.microsoft.com/api/mcp");
        assert!(matches!(
            server.transport_type,
            McpServerTransportType::Http
        ));

        // --- skills ---
        let skill = compiled
            .definition
            .skills
            .iter()
            .find(|s| s.name == "microsoft-docs")
            .expect("microsoft-docs skill");
        assert!(!skill.instructions.is_empty());

        // --- commands (user-invocable skill) ---
        let command = compiled
            .definition
            .skills
            .iter()
            .find(|s| s.name == "ms-docs")
            .expect("ms-docs command skill");
        assert!(command.user_invocable);

        // --- agent → system_prompt ---
        let prompt = compiled
            .definition
            .system_prompt
            .as_ref()
            .expect("system_prompt");
        assert!(
            prompt.contains("docs-researcher"),
            "expected docs-researcher in system_prompt, got: {prompt}"
        );

        // --- interface warning ---
        assert!(
            compiled.warnings.iter().any(|w| w.contains("interface")),
            "expected interface warning, got: {:?}",
            compiled.warnings
        );
    }

    #[test]
    fn compile_first_party_portable_plugins() {
        for (name, url) in [
            ("everruns", "https://app.everruns.com/mcp"),
            ("everruns-dev", "https://dev.everruns.com/mcp"),
            ("resend", "https://mcp.resend.com/mcp"),
        ] {
            let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../plugins")
                .join(name);
            let file_set = PluginFileSet::from_dir(&fixture).expect("load first-party plugin");
            let compiled = compile_plugin(&file_set).expect("compile first-party plugin");

            assert!(compiled.manifest.is_agent_plugins_v1());
            assert_eq!(compiled.definition.name, name);
            let server = compiled
                .definition
                .mcp_servers
                .as_ref()
                .and_then(|servers| servers.get(name))
                .expect("portable MCP server");
            assert_eq!(server.auth_mode, McpServerAuthMode::OAuth);
            assert_eq!(server.url, url);
            assert!(server.oauth_provider_id.is_none());
        }
    }

    #[test]
    fn agent_plugins_v1_mcp_is_strict_and_isolates_invalid_entries() {
        let mut files = std::collections::BTreeMap::new();
        files.insert(
            "plugin.json".to_string(),
            serde_json::json!({
                "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
                "name": "portable-tools",
                "extensions": {
                    "com.everruns": {
                        "mcpServers": { "remote": { "auth": "oauth" } }
                    }
                }
            })
            .to_string()
            .into_bytes(),
        );
        files.insert(
            "mcp.json".to_string(),
            serde_json::json!({
                "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
                "mcpServers": {
                    "remote": {
                        "type": "streamable-http",
                        "url": "https://example.com/mcp",
                        "headers": { "X-Tenant": "public" }
                    },
                    "bad": {
                        "type": "streamable-http",
                        "url": "https://example.com/mcp",
                        "unexpected": true
                    },
                    "local": {
                        "type": "stdio",
                        "command": "node"
                    }
                }
            })
            .to_string()
            .into_bytes(),
        );
        let file_set = PluginFileSet::from_map("portable-tools", files).unwrap();

        let compiled = compile_plugin(&file_set).expect("valid siblings should compile");
        let servers = compiled
            .definition
            .mcp_servers
            .expect("portable MCP server");

        assert_eq!(servers.len(), 1);
        let remote = servers.get("remote").expect("remote server");
        assert_eq!(remote.url, "https://example.com/mcp");
        assert_eq!(remote.auth_mode, McpServerAuthMode::OAuth);
        assert_eq!(
            serde_json::to_value(remote).unwrap(),
            serde_json::json!({"type":"http","url":"https://example.com/mcp","auth_mode":"oauth","headers":{"X-Tenant":"public"}})
        );
        assert!(
            compiled
                .warnings
                .iter()
                .any(|warning| warning.contains("bad"))
        );
        assert!(
            compiled
                .warnings
                .iter()
                .any(|warning| warning.contains("local"))
        );
    }

    fn package(
        canonical: bool,
        manifest: serde_json::Value,
        files: &[(&str, &[u8])],
    ) -> PluginFileSet {
        let mut contents: BTreeMap<String, Vec<u8>> = files
            .iter()
            .map(|(p, b)| (p.to_string(), b.to_vec()))
            .collect();
        contents.insert(
            if canonical {
                "plugin.json"
            } else {
                ".claude-plugin/plugin.json"
            }
            .into(),
            serde_json::to_vec(&manifest).unwrap(),
        );
        PluginFileSet::from_map("ignored", contents).unwrap()
    }

    fn portable_manifest() -> serde_json::Value {
        serde_json::json!({"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"portable-tools"})
    }

    #[test]
    fn names_and_descriptions_follow_each_manifest_dialect() {
        for name in ["3.acme-tools".to_string(), "a".repeat(64)] {
            let mut manifest = portable_manifest();
            manifest["name"] = name.clone().into();
            let compiled = compile_plugin(&package(true, manifest, &[])).unwrap();
            assert_eq!(compiled.definition.name, name);
            assert_eq!(
                compiled.definition.description,
                format!("Agent plugin {name}")
            );
        }
        for name in [
            "a".repeat(65),
            "bad_name".into(),
            "bad..name".into(),
            "bad--name".into(),
            "Bad".into(),
        ] {
            let mut manifest = portable_manifest();
            manifest["name"] = name.into();
            assert!(compile_plugin(&package(true, manifest, &[])).is_err());
        }
        let legacy = |name: &str, description: serde_json::Value| {
            package(
                false,
                serde_json::json!({"name":name,"description":description}),
                &[],
            )
        };
        let accepted = "a".repeat(38);
        assert_eq!(
            compile_plugin(&legacy(&accepted, "description".into()))
                .unwrap()
                .definition
                .name,
            accepted
        );
        for name in [
            "a".repeat(39),
            "a".repeat(44),
            "../evil".into(),
            "a/b".into(),
            "a_".into(),
            "A".into(),
            "".into(),
        ] {
            assert!(
                compile_plugin(&legacy(&name, "description".into())).is_err(),
                "{name}"
            );
        }
        for description in [serde_json::Value::Null, "".into(), "   ".into()] {
            assert_eq!(
                compile_plugin(&legacy("valid", description)).unwrap_err(),
                "plugin manifest is missing a 'description' field"
            );
        }
        let mut manifest = portable_manifest();
        manifest["$schema"] = "https://agent-plugins.org/schemas/2.0.0/plugin.schema.json".into();
        assert_eq!(
            compile_plugin(&package(true, manifest, &[])).unwrap_err(),
            "unsupported Agent Plugins schema 'https://agent-plugins.org/schemas/2.0.0/plugin.schema.json'; supported schema is https://agent-plugins.org/schemas/1.0.0/plugin.schema.json"
        );
    }

    #[test]
    fn invalid_portable_mcp_keeps_valid_skill_content() {
        for (mcp, warning) in [
            ("{", "mcp.json is invalid JSON and was disabled:"),
            (
                "[]",
                "mcp.json must contain a JSON object; MCP was disabled",
            ),
            (
                r#"{"mcpServers":{}}"#,
                "mcp.json uses an unsupported or mismatched schema; expected",
            ),
            (
                r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/mcp.schema.json"}"#,
                "mcp.json is missing the required 'mcpServers' object",
            ),
        ] {
            let fs=package(true,portable_manifest(), &[("mcp.json",mcp.as_bytes()),("skills/useful/SKILL.md",b"---\nname: useful\ndescription: useful skill\n---\nKeep this instruction.")]);
            let compiled = compile_plugin(&fs).unwrap();
            assert!(compiled.definition.mcp_servers.is_none());
            assert_eq!(compiled.definition.skills.len(), 1);
            assert_eq!(compiled.definition.skills[0].name, "useful");
            assert_eq!(
                compiled.definition.skills[0].instructions,
                "Keep this instruction."
            );
            assert_eq!(compiled.warnings.len(), 1);
            assert!(
                compiled.warnings[0].starts_with(warning),
                "{:?}",
                compiled.warnings
            );
        }
    }

    #[test]
    fn legacy_mcp_preserves_public_config_and_rejects_provider_binding() {
        let fs=package(false,serde_json::json!({"name":"legacy","description":"legacy plugin"}),&[(".mcp.json",br#"{"mcpServers":{
            "oauth":{"url":"https://example.com/mcp","auth":"oauth","headers":{"X-Custom":"1","X-Bad":5},"oauth_provider_id":"github"},
            "unsupported":{"url":"https://example.com/other","auth":"api_key"},
            "alias":{"url":"https://example.com/alias","auth_mode":"OAUTH"},
            "local":{"type":"stdio","command":"npx"}
        }}"#)]);
        let compiled = compile_plugin(&fs).unwrap();
        assert_eq!(
            serde_json::to_value(compiled.definition.mcp_servers).unwrap(),
            serde_json::json!({
                "oauth":{"type":"http","url":"https://example.com/mcp","auth_mode":"oauth","headers":{"X-Custom":"1"}},
                "unsupported":{"type":"http","url":"https://example.com/other"},
                "alias":{"type":"http","url":"https://example.com/alias","auth_mode":"oauth"}
            })
        );
        assert_eq!(
            compiled.warnings,
            [
                "MCP server 'local': stdio transport is not supported in v1 and will be skipped",
                "MCP server 'oauth': header 'X-Bad' is not a string and will be ignored",
                "MCP server 'oauth': 'oauth_provider_id' cannot be set by a plugin and will be ignored",
                "MCP server 'unsupported': auth mode 'api_key' is not supported for plugin servers and will be ignored",
            ]
        );
    }

    #[test]
    fn bundled_icons_preserve_bytes_and_unsafe_inputs_use_fallback() {
        for (icon, asset, expected_warning) in [
            (None, None, None),
            (
                Some("assets/missing.svg"),
                None,
                Some("missing or is not UTF-8"),
            ),
            (
                Some("https://tracker.example/icon.svg"),
                None,
                Some("relative path"),
            ),
            (Some("../icon.svg"), None, Some("relative path")),
            (Some("data:image/svg+xml,test"), None, Some("relative path")),
            (
                Some("assets/icon.svg"),
                Some("not svg"),
                Some("expected an SVG root"),
            ),
            (
                Some("assets/icon.svg"),
                Some("<svg><script>alert(1)</script></svg>"),
                Some("active or external content"),
            ),
            (
                Some("assets/icon.svg"),
                Some("<svg onload='run()'></svg>"),
                Some("event handler"),
            ),
            (
                Some("assets/icon.svg"),
                Some("<svg><use href='#x'/></svg>"),
                Some("active or external content"),
            ),
        ] {
            let files = asset
                .map(|s| vec![("assets/icon.svg", s.as_bytes())])
                .unwrap_or_default();
            let compiled = compile_plugin(&package(
                false,
                serde_json::json!({"name":"icons","description":"icons","icon":icon}),
                &files,
            ))
            .unwrap();
            assert_eq!(compiled.definition.icon.as_deref(), Some("puzzle"));
            match expected_warning {
                Some(w) => {
                    assert_eq!(compiled.warnings.len(), 1);
                    assert!(compiled.warnings[0].contains(w), "{:?}", compiled.warnings);
                }
                None => assert!(compiled.warnings.is_empty()),
            }
        }
        let compiled = compile_plugin(&package(
            false,
            serde_json::json!({"name":"icons","description":"icons","icon":"./assets/icon.svg"}),
            &[("assets/icon.svg", b"<svg></svg>")],
        ))
        .unwrap();
        assert_eq!(
            compiled.definition.icon.as_deref(),
            Some("data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=")
        );
        assert!(compiled.warnings.is_empty());
    }

    #[test]
    fn component_overrides_preserve_prompts_commands_and_nested_skill_files() {
        let fs=package(false,serde_json::json!({"name":"components","description":"components","agents":["./custom-agents/"],"commands":"./custom-commands/","skills":["./custom-skills/"]}),&[
            ("custom-agents/b.md",b"Second agent."),
            ("custom-agents/a.md",b"---\nname: A & B\ndescription: <quoted>\n---\n First agent. \n"),
            ("custom-commands/run.md",b"---\nname: run\ndescription: Run a task\n---\nExecute $ARGUMENTS."),
            ("custom-skills/useful/SKILL.md",b"---\nname: useful\ndescription: Useful skill\nuser-invocable: false\ndisable-model-invocation: true\n---\nInstructions."),
            ("custom-skills/useful/nested/readme.txt",b"Reference bytes.\n"),
            ("custom-skills/useful/binary.bin",&[0xff]),
            ("agents/ignored.md",b"Ignored agent."),
            ("commands/ignored.md",b"Ignored command."),
        ]);
        let compiled = compile_plugin(&fs).unwrap();
        assert_eq!(
            compiled.definition.system_prompt.as_deref(),
            Some(
                "<agent name=\"A &amp; B\" description=\"&lt;quoted&gt;\">\nFirst agent.\n</agent>\n\n<agent name=\"b\">\nSecond agent.\n</agent>"
            )
        );
        assert_eq!(
            serde_json::to_value(&compiled.definition.skills).unwrap(),
            serde_json::json!([
                {"name":"useful","description":"Useful skill","instructions":"Instructions.","files":[{"path":"nested/readme.txt","content":"Reference bytes.\n"}],"user_invocable":false,"disable_model_invocation":true},
                {"name":"run","description":"Run a task","instructions":"Execute $ARGUMENTS.","files":[],"user_invocable":true,"disable_model_invocation":false}
            ])
        );
        assert_eq!(
            compiled.warnings,
            ["skill 'useful': binary file 'binary.bin' skipped (text only)"]
        );
    }

    #[test]
    fn portable_remote_urls_allow_loopback_but_reject_insecure_or_credentialed_hosts() {
        for (url, allowed) in [
            ("http://[::1]:8080/mcp", true),
            ("http://127.0.0.2/mcp", true),
            ("http://localhost:8080/mcp", true),
            ("https://example.com/mcp", true),
            ("http://[2001:db8::1]/mcp", false),
            ("http://example.com/mcp", false),
            ("https://user:pass@example.com/mcp", false),
            ("https://example.com/mcp#fragment", false),
            ("ftp://example.com/mcp", false),
        ] {
            let mcp = serde_json::json!({"$schema":"https://agent-plugins.org/schemas/1.0.0/mcp.schema.json","mcpServers":{"remote":{"type":"streamable-http","url":url}}});
            let fs = package(
                true,
                portable_manifest(),
                &[("mcp.json", mcp.to_string().as_bytes())],
            );
            let compiled = compile_plugin(&fs).unwrap();
            if allowed {
                assert_eq!(
                    serde_json::to_value(compiled.definition.mcp_servers).unwrap(),
                    serde_json::json!({"remote":{"type":"http","url":url}}),
                    "{url}"
                );
                assert!(
                    compiled.warnings.is_empty(),
                    "{url}: {:?}",
                    compiled.warnings
                );
            } else {
                assert!(compiled.definition.mcp_servers.is_none(), "{url}");
                assert_eq!(compiled.warnings.len(), 1);
                assert!(
                    compiled.warnings[0].contains("remote"),
                    "{:?}",
                    compiled.warnings
                );
            }
        }
    }
}

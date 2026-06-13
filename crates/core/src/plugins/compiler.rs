// Plugin compiler: PluginFileSet → CompiledPlugin
//
// Maps plugin directory components to a DeclarativeCapabilityDefinition per
// the table in specs/plugins.md.

use std::collections::BTreeMap;

use crate::capabilities::{
    CapabilityStatus, DeclarativeCapabilityDefinition, DeclarativeCapabilityFile,
    DeclarativeCapabilitySkill, DeclarativeCapabilitySkillFile,
    validate_declarative_capability_definition,
};
use crate::mcp_server::{McpServerTransportType, ScopedMcpServer, ScopedMcpServers};

use super::file_set::PluginFileSet;
use super::manifest::{McpServersField, PluginManifest};

// `plugin:` prefix is 7 bytes, leaving 43 bytes for the name within the
// VARCHAR(50) capability reference columns.
const PLUGIN_CAPABILITY_PREFIX: &str = "plugin:";
const MAX_PLUGIN_NAME_BYTES: usize = 50 - PLUGIN_CAPABILITY_PREFIX.len(); // 43

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
/// the component-mapping table in `specs/plugins.md`. Errors are returned when
/// compilation cannot produce a valid `DeclarativeCapabilityDefinition`.
pub fn compile_plugin(file_set: &PluginFileSet) -> Result<CompiledPlugin, String> {
    let (manifest, mut warnings) = file_set.manifest()?;

    // --- name ---
    let name = sanitize_plugin_name(&manifest.name)?;
    if name.len() > MAX_PLUGIN_NAME_BYTES {
        return Err(format!(
            "plugin name '{}' is {} bytes but must fit in {} bytes (plugin: prefix occupies {} bytes)",
            name,
            name.len(),
            MAX_PLUGIN_NAME_BYTES,
            PLUGIN_CAPABILITY_PREFIX.len()
        ));
    }

    // --- description (required) ---
    let description = manifest
        .description
        .clone()
        .filter(|d| !d.trim().is_empty())
        .ok_or_else(|| "plugin manifest is missing a 'description' field".to_string())?;

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

    // --- userConfig → config_schema / config_ui_schema ---
    let (config_schema, config_ui_schema) =
        compile_user_config(manifest.user_config.as_ref(), &mut warnings);

    // Warn about unsupported component fields in the manifest.
    for ignored_field in &["hooks", "lspServers", "monitors", "themes", "outputStyles"] {
        if manifest.extra.contains_key(*ignored_field) {
            warnings.push(format!(
                "plugin manifest: '{ignored_field}' is not supported in v1 and will be ignored"
            ));
        }
    }

    let definition = DeclarativeCapabilityDefinition {
        name: name.clone(),
        display_name,
        description,
        status: CapabilityStatus::Available,
        icon: Some("puzzle".to_string()),
        category: Some("Plugin".to_string()),
        system_prompt,
        mcp_servers,
        skills: all_skills,
        files: Vec::<DeclarativeCapabilityFile>::new(),
        dependencies: Vec::new(),
        features: Vec::new(),
        risk_level: crate::capabilities::RiskLevel::Low,
        config_schema,
        config_ui_schema,
    };

    // Run through declarative validation to catch size/count violations.
    validate_declarative_capability_definition(&definition)
        .map_err(|e| format!("compiled plugin failed declarative validation: {e}"))?;

    Ok(CompiledPlugin {
        manifest,
        definition,
        warnings,
    })
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

// ============================================================================
// Agents → system_prompt
// ============================================================================

/// Render agent files into a combined system prompt.
///
/// Each `.md` file under `agents/` (or the manifest-overridden path) is
/// rendered as a named `<agent>` XML section per specs/xml-prompt-formatting.md.
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

        // The URL is fixed at compile time and SSRF-validated at install.
        // `${user_config.*}` placeholders are only substituted in headers/env,
        // never the URL (a templated host would bypass SSRF validation), so
        // warn authors that a URL placeholder will not be filled in.
        if url.contains("${user_config.") {
            warnings.push(format!(
                "MCP server '{server_name}': '${{user_config.*}}' placeholders in 'url' are not substituted (only headers and env support them); the URL is used verbatim"
            ));
        }

        // Carry over `headers` and `env` string maps. Values may contain
        // `${user_config.KEY}` placeholders, preserved verbatim here and
        // substituted per-agent at hydration time.
        let headers = extract_string_map(&server_config, "headers");
        let env = extract_string_map(&server_config, "env");

        servers.insert(
            server_name,
            ScopedMcpServer {
                transport_type: McpServerTransportType::Http,
                url,
                headers,
                env,
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

// ============================================================================
// userConfig → config_schema / config_ui_schema
// ============================================================================

/// Convert a `userConfig` map from a plugin manifest into a JSON Schema object
/// and an optional react-jsonschema-form uiSchema.
///
/// Each entry's `type`, `title`, `description`, `default`, and `enum` fields
/// are carried into the schema's `properties`. `sensitive: true` is stripped
/// from the schema and becomes `{"ui:widget": "password"}` in the uiSchema.
/// Per-entry `required: true` is collected into the top-level `required` array.
fn compile_user_config(
    user_config: Option<&std::collections::HashMap<String, serde_json::Value>>,
    _warnings: &mut Vec<String>,
) -> (Option<serde_json::Value>, Option<serde_json::Value>) {
    let Some(config_map) = user_config else {
        return (None, None);
    };
    if config_map.is_empty() {
        return (None, None);
    }

    let mut properties = serde_json::Map::new();
    let mut required: Vec<serde_json::Value> = Vec::new();
    let mut ui_schema = serde_json::Map::new();

    // Sort keys for deterministic output.
    let mut keys: Vec<&String> = config_map.keys().collect();
    keys.sort();

    for key in keys {
        let descriptor = &config_map[key];
        let obj = match descriptor.as_object() {
            Some(o) => o,
            None => continue,
        };

        let mut prop = serde_json::Map::new();
        // Carry standard JSON Schema keywords.
        for kw in &["type", "title", "description", "default", "enum"] {
            if let Some(v) = obj.get(*kw) {
                prop.insert(kw.to_string(), v.clone());
            }
        }
        // Default type to "string" if absent.
        if !prop.contains_key("type") {
            prop.insert(
                "type".to_string(),
                serde_json::Value::String("string".to_string()),
            );
        }

        // `sensitive: true` → password widget in uiSchema.
        if obj
            .get("sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            ui_schema.insert(key.clone(), serde_json::json!({"ui:widget": "password"}));
        }

        // Per-entry `required: true` → top-level required array.
        if obj
            .get("required")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            required.push(serde_json::Value::String(key.clone()));
        }

        properties.insert(key.clone(), serde_json::Value::Object(prop));
    }

    let mut schema = serde_json::json!({
        "type": "object",
        "properties": properties,
    });
    if !required.is_empty() {
        schema["required"] = serde_json::Value::Array(required);
    }

    let ui = if ui_schema.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(ui_schema))
    };

    (Some(schema), ui)
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

/// Extract a `name → string` map from a JSON object field (e.g. `headers`,
/// `env`). Non-string values are coerced via `to_string()` of the JSON scalar;
/// nested objects/arrays are skipped. Returns an empty map when absent.
fn extract_string_map(
    server_config: &serde_json::Value,
    field: &str,
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    if let Some(obj) = server_config.get(field).and_then(|v| v.as_object()) {
        for (k, v) in obj {
            let s = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            out.insert(k.clone(), s);
        }
    }
    out
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

    // ---- fixture integration test ----

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

        // --- userConfig → config_schema / config_ui_schema ---
        let schema = compiled
            .definition
            .config_schema
            .as_ref()
            .expect("config_schema from userConfig");
        let props = schema["properties"].as_object().expect("properties");
        assert!(
            props.contains_key("preferred_language"),
            "preferred_language in schema"
        );
        assert!(props.contains_key("api_token"), "api_token in schema");
        // `sensitive` must not leak into the schema properties.
        assert!(
            props["api_token"].get("sensitive").is_none(),
            "sensitive must not appear in schema property"
        );

        let ui = compiled
            .definition
            .config_ui_schema
            .as_ref()
            .expect("config_ui_schema from sensitive field");
        assert_eq!(
            ui["api_token"]["ui:widget"].as_str(),
            Some("password"),
            "sensitive field maps to password widget"
        );
        // Non-sensitive field must not appear in uiSchema.
        assert!(
            ui.get("preferred_language").is_none(),
            "non-sensitive field must not appear in uiSchema"
        );
    }

    // ---- targeted unit tests ----

    #[test]
    fn traversal_rejection() {
        // The OS won't allow `..` in actual directory paths, so we test the
        // name validation logic that rejects traversal-like plugin names and
        // also verify the file_set traversal guard via file_set::tests.
        // `../evil` fails at the first char check (not lowercase).
        let err = sanitize_plugin_name("../evil").unwrap_err();
        assert!(err.contains("must start with a lowercase letter"), "{err}");
        // A name that starts with a letter but contains traversal separators.
        let err2 = sanitize_plugin_name("a/b").unwrap_err();
        assert!(err2.contains("only contain"), "{err2}");
    }

    #[test]
    fn stdio_mcp_produces_warning() {
        let mut warnings = Vec::new();
        let file_set_files = {
            let mut f = std::collections::BTreeMap::new();
            f.insert(
                ".claude-plugin/plugin.json".to_string(),
                serde_json::json!({
                    "name": "test-plugin",
                    "description": "A test plugin."
                })
                .to_string()
                .into_bytes(),
            );
            f.insert(
                ".mcp.json".to_string(),
                serde_json::json!({
                    "mcpServers": {
                        "my-server": {
                            "type": "stdio",
                            "command": "npx",
                            "args": ["-y", "@some/mcp-server"]
                        }
                    }
                })
                .to_string()
                .into_bytes(),
            );
            f
        };
        let file_set = PluginFileSet {
            files: file_set_files,
            dir_name: "test-plugin".to_string(),
        };
        let manifest = PluginManifest {
            name: "test-plugin".to_string(),
            display_name: None,
            version: None,
            description: Some("test".to_string()),
            author: None,
            homepage: None,
            repository: None,
            license: None,
            keywords: Vec::new(),
            skills: None,
            commands: None,
            agents: None,
            mcp_servers: None,
            user_config: None,
            extra: Default::default(),
        };
        let result = compile_mcp_servers(&file_set, &manifest, &mut warnings);
        assert!(result.is_ok());
        assert!(
            warnings.iter().any(|w| w.contains("stdio")),
            "expected stdio warning, got: {warnings:?}"
        );
        // No servers compiled since only one was stdio.
        assert!(result.unwrap().is_none());
    }

    fn mcp_test_manifest() -> PluginManifest {
        PluginManifest {
            name: "p".to_string(),
            display_name: None,
            version: None,
            description: Some("test".to_string()),
            author: None,
            homepage: None,
            repository: None,
            license: None,
            keywords: Vec::new(),
            skills: None,
            commands: None,
            agents: None,
            mcp_servers: None,
            user_config: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn http_mcp_carries_headers_and_env_with_placeholders() {
        let mut warnings = Vec::new();
        let mut files = std::collections::BTreeMap::new();
        files.insert(
            ".mcp.json".to_string(),
            serde_json::json!({
                "mcpServers": {
                    "api": {
                        "type": "http",
                        "url": "https://api.example.com/mcp",
                        "headers": {
                            "Authorization": "Bearer ${user_config.api_token}",
                            "X-Static": "constant"
                        },
                        "env": { "REGION": "${user_config.region}" }
                    }
                }
            })
            .to_string()
            .into_bytes(),
        );
        let file_set = PluginFileSet {
            files,
            dir_name: "p".to_string(),
        };
        let servers = compile_mcp_servers(&file_set, &mcp_test_manifest(), &mut warnings)
            .expect("ok")
            .expect("servers");
        let api = servers.get("api").expect("api server");
        assert_eq!(api.url, "https://api.example.com/mcp");
        // Header/env placeholders are preserved verbatim at compile time.
        assert_eq!(
            api.headers.get("Authorization").map(String::as_str),
            Some("Bearer ${user_config.api_token}")
        );
        assert_eq!(
            api.headers.get("X-Static").map(String::as_str),
            Some("constant")
        );
        assert_eq!(
            api.env.get("REGION").map(String::as_str),
            Some("${user_config.region}")
        );
        // A fixed URL produces no placeholder warning.
        assert!(!warnings.iter().any(|w| w.contains("url")), "{warnings:?}");
    }

    #[test]
    fn url_placeholder_produces_warning() {
        let mut warnings = Vec::new();
        let mut files = std::collections::BTreeMap::new();
        files.insert(
            ".mcp.json".to_string(),
            serde_json::json!({
                "mcpServers": {
                    "api": {
                        "type": "http",
                        "url": "https://${user_config.host}/mcp"
                    }
                }
            })
            .to_string()
            .into_bytes(),
        );
        let file_set = PluginFileSet {
            files,
            dir_name: "p".to_string(),
        };
        let servers = compile_mcp_servers(&file_set, &mcp_test_manifest(), &mut warnings)
            .expect("ok")
            .expect("servers");
        // URL is kept verbatim (never substituted) and a warning is emitted.
        assert_eq!(
            servers.get("api").unwrap().url,
            "https://${user_config.host}/mcp"
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("url") && w.contains("not substituted")),
            "expected url placeholder warning, got: {warnings:?}"
        );
    }

    #[test]
    fn user_config_compiles_to_schema() {
        use std::collections::HashMap;
        let mut config_map: HashMap<String, serde_json::Value> = HashMap::new();
        config_map.insert(
            "endpoint".to_string(),
            serde_json::json!({
                "type": "string",
                "title": "Endpoint URL",
                "description": "API endpoint",
                "required": true
            }),
        );
        config_map.insert(
            "token".to_string(),
            serde_json::json!({
                "type": "string",
                "title": "API Token",
                "sensitive": true
            }),
        );

        let mut warnings = Vec::new();
        let (schema, ui) = compile_user_config(Some(&config_map), &mut warnings);

        let schema = schema.expect("schema produced");
        assert_eq!(schema["type"].as_str(), Some("object"));
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("endpoint"));
        assert!(props.contains_key("token"));
        // `sensitive` does not appear in properties.
        assert!(props["token"].get("sensitive").is_none());
        // `required` at field level is hoisted to top-level array.
        let req = schema["required"].as_array().expect("required array");
        assert!(req.iter().any(|v| v.as_str() == Some("endpoint")));
        // Non-required field is not in required array.
        assert!(!req.iter().any(|v| v.as_str() == Some("token")));

        let ui = ui.expect("uiSchema produced");
        assert_eq!(ui["token"]["ui:widget"].as_str(), Some("password"));
        assert!(ui.get("endpoint").is_none());
    }

    #[test]
    fn user_config_none_produces_no_schema() {
        let mut warnings = Vec::new();
        let (schema, ui) = compile_user_config(None, &mut warnings);
        assert!(schema.is_none());
        assert!(ui.is_none());
    }

    #[test]
    fn missing_description_is_error() {
        let file_set_files = {
            let mut f = std::collections::BTreeMap::new();
            f.insert(
                ".claude-plugin/plugin.json".to_string(),
                serde_json::json!({
                    "name": "nodesc-plugin"
                })
                .to_string()
                .into_bytes(),
            );
            f
        };
        let file_set = PluginFileSet {
            files: file_set_files,
            dir_name: "nodesc-plugin".to_string(),
        };
        let err = compile_plugin(&file_set).unwrap_err();
        assert!(err.contains("description"), "error was: {err}");
    }

    #[test]
    fn oversized_name_is_error() {
        // A name longer than MAX_PLUGIN_NAME_BYTES (43) should fail.
        let long_name = "a".repeat(MAX_PLUGIN_NAME_BYTES + 1);
        let file_set_files = {
            let mut f = std::collections::BTreeMap::new();
            f.insert(
                ".claude-plugin/plugin.json".to_string(),
                serde_json::json!({
                    "name": long_name,
                    "description": "test"
                })
                .to_string()
                .into_bytes(),
            );
            f
        };
        let file_set = PluginFileSet {
            files: file_set_files,
            dir_name: "aaa".to_string(),
        };
        let err = compile_plugin(&file_set).unwrap_err();
        assert!(err.contains("bytes"), "error was: {err}");
    }
}

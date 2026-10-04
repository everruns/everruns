//! Portable agent files, folders and archives, shared by all Everruns hosts.
//!
//! Enabled with `agent-package`; disk reads and writes additionally require
//! `agent-package-fs`. Framework applications use `everruns::AgentPackage`.
//!
//! Part of the [Everruns](https://everruns.com) ecosystem.
//!
//! ```
//! use everruns_core::agent_package::{AgentPackage, Format};
//! let package = AgentPackage::parse("---\nname: helper\n---\nBe helpful.", Format::Markdown)?;
//! let archive = package.to_zip()?;
//! let restored = AgentPackage::from_zip(&archive)?;
//! assert!(package.diff(&restored)?.is_empty());
//! # Ok::<(), everruns_core::agent_package::PackageError>(())
//! ```
//!
//! Parsing never runs tools, starts MCP processes, reads credentials, or writes
//! platform resources. Hosts bind the validated values into their existing
//! builders and enforce their own capability, identity and transport policy.

mod assets;
#[cfg(feature = "agent-package-fs")]
mod fs;
pub use assets::{ALLOWED_DOT_ENTRIES, DENIED_DOT_ENTRIES};
mod diff;
pub use diff::Change;

use crate::{InitialFile, ScopedMcpServers, network_access::NetworkAccessList};
use everruns_contracts::{
    capability::CapabilityRef,
    model_spec::ModelSpec,
    tool_types::ToolDefinition,
    typed_id::{AgentId, HarnessId, ModelId},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::Path,
};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_PACKAGE_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
pub const MAX_FILES: usize = 100;
pub const MAX_CHANNELS: usize = 32;
pub const MAX_INITIAL_BYTES: usize = 5 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Auto,
    Markdown,
    Toml,
    Yaml,
    Json,
}

impl Format {
    pub fn from_extension(path: &Path) -> Self {
        match path.extension().and_then(|s| s.to_str()) {
            Some("md" | "markdown") => Self::Markdown,
            Some("toml") => Self::Toml,
            Some("yaml" | "yml") => Self::Yaml,
            Some("json") => Self::Json,
            _ => Self::Auto,
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Auto | Self::Markdown => "md",
            Self::Toml => "toml",
            Self::Yaml => "yaml",
            Self::Json => "json",
        }
    }
}

/// A field-addressed diagnostic, shared by local validation and import APIs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Diagnostic {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct PackageError(pub Vec<Diagnostic>);
impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, error) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, "; ")?;
            }
            write!(f, "{}: {}", error.path, error.message)?;
        }
        Ok(())
    }
}
impl std::error::Error for PackageError {}
pub type Result<T> = std::result::Result<T, PackageError>;
pub(crate) fn error(path: impl Into<String>, message: impl fmt::Display) -> PackageError {
    PackageError(vec![Diagnostic {
        path: path.into(),
        message: message.to_string(),
    }])
}

/// Portable channel intent. Authentication and installation are host bindings.
/// Channels default to disabled; enabled intent still requires host publication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Channel {
    #[serde(rename = "type")]
    pub channel_type: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "empty_object")]
    pub config: Value,
}
fn empty_object() -> Value {
    serde_json::json!({})
}

/// Embedded files and folder-relative sources share the same manifest field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum File {
    Pattern(String),
    Source(FileSource),
    Inline(InitialFile),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FileSource {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default = "yes")]
    pub is_readonly: bool,
}
fn yes() -> bool {
    true
}
fn version() -> u32 {
    SCHEMA_VERSION
}

/// Authored values only: no organisation, agent, model, harness or channel IDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Manifest {
    #[serde(default = "version")]
    pub schema_version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, alias = "system_prompt")]
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "model")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<PackageModel>))]
    pub model: Option<ModelSpec>,
    #[serde(
        default,
        alias = "harness_name",
        skip_serializing_if = "Option::is_none"
    )]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(
        default,
        deserialize_with = "capabilities",
        skip_serializing_if = "Vec::is_empty"
    )]
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<PackageCapability>))]
    pub capabilities: Vec<CapabilityRef>,
    #[serde(
        default,
        deserialize_with = "files",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub initial_files: Vec<File>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(
        default,
        rename = "mcpServers",
        alias = "mcp_servers",
        deserialize_with = "mcp_servers",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub mcp_servers: ScopedMcpServers,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "network_access")]
    pub network_access: Option<NetworkAccessList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "tools")]
    pub tools: Vec<ToolDefinition>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channels: BTreeMap<String, Channel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro_markdown: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starters: Vec<Value>,
    // Platform-specific environment descriptions remain data; other hosts
    // must explicitly bind them or reject them rather than discard them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environments: Option<Value>,
}

fn strict_object(value: &Value, fields: &[&str], path: &str) -> std::result::Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{path}: expected an object"))?;
    if let Some(key) = object.keys().find(|key| !fields.contains(&key.as_str())) {
        return Err(format!("{path}: unknown field {key}"));
    }
    Ok(())
}
fn model<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<ModelSpec>, D::Error> {
    let value = Option::<Value>::deserialize(d)?;
    if let Some(value) = &value {
        strict_object(value, &["provider", "model"], "model").map_err(serde::de::Error::custom)?;
    }
    serde_json::from_value(serde_json::json!(value)).map_err(serde::de::Error::custom)
}
fn network_access<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<NetworkAccessList>, D::Error> {
    let value = Option::<Value>::deserialize(d)?;
    if let Some(value) = &value {
        strict_object(value, &["allowed", "blocked"], "network_access")
            .map_err(serde::de::Error::custom)?;
    }
    serde_json::from_value(serde_json::json!(value)).map_err(serde::de::Error::custom)
}
fn tools<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Vec<ToolDefinition>, D::Error> {
    let values = Vec::<Value>::deserialize(d)?;
    for (i, value) in values.iter().enumerate() {
        strict_object(
            value,
            &[
                "type",
                "name",
                "display_name",
                "description",
                "parameters",
                "category",
                "deferrable",
                "hints",
                "full_parameters",
            ],
            &format!("tools[{i}]"),
        )
        .map_err(serde::de::Error::custom)?;
    }
    serde_json::from_value(serde_json::json!(values)).map_err(serde::de::Error::custom)
}

fn mcp_servers<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<ScopedMcpServers, D::Error> {
    let values = BTreeMap::<String, Value>::deserialize(deserializer)?;
    const FIELDS: &[&str] = &[
        "type",
        "transport_type",
        "url",
        "headers",
        "command",
        "args",
        "env",
        "auth_mode",
        "protocol_mode",
        "elicitation_policy",
        "oauth_provider_id",
        "tool_discovery",
        "use",
        "actsAs",
        "acts_as",
    ];
    for (name, value) in &values {
        let object = value.as_object().ok_or_else(|| {
            serde::de::Error::custom(format!("mcpServers.{name}: expected an object"))
        })?;
        if let Some(key) = object.keys().find(|key| !FIELDS.contains(&key.as_str())) {
            return Err(serde::de::Error::custom(format!(
                "mcpServers.{name}: unknown field {key}"
            )));
        }
    }
    serde_json::from_value(serde_json::json!(values)).map_err(serde::de::Error::custom)
}

fn files<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<File>, D::Error> {
    let values = Vec::<Value>::deserialize(deserializer)?;
    values
        .into_iter()
        .map(|value| {
            if let Some(object) = value.as_object() {
                let keys: &[&str] = if object.contains_key("source") {
                    &["source", "path", "is_readonly"]
                } else {
                    &["path", "content", "encoding", "is_readonly"]
                };
                if let Some(key) = object.keys().find(|key| !keys.contains(&key.as_str())) {
                    return Err(serde::de::Error::custom(format!(
                        "unknown initial file field {key}"
                    )));
                }
            }
            serde_json::from_value(value).map_err(serde::de::Error::custom)
        })
        .collect()
}

fn capabilities<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<CapabilityRef>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Entry {
        Name(String),
        Ref(Reference),
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Reference {
        #[serde(rename = "ref")]
        name: String,
        #[serde(default = "empty_object")]
        config: Value,
    }
    Vec::<Entry>::deserialize(deserializer).map(|entries| {
        entries
            .into_iter()
            .map(|entry| match entry {
                Entry::Name(name) => CapabilityRef::new(name),
                Entry::Ref(value) => CapabilityRef::with_config(value.name, value.config),
            })
            .collect()
    })
}

#[derive(Debug, Clone)]
pub struct AgentPackage {
    pub manifest: Manifest,
    /// Legacy IDs are input-only, never serialized in portable exports.
    pub legacy_id: Option<AgentId>,
    pub legacy_model_id: Option<ModelId>,
    pub legacy_harness_id: Option<HarnessId>,
}

impl AgentPackage {
    pub fn new(mut manifest: Manifest) -> Result<Self> {
        defaults(&mut manifest);
        let package = Self {
            manifest,
            legacy_id: None,
            legacy_model_id: None,
            legacy_harness_id: None,
        };
        package.validate()?;
        Ok(package)
    }

    pub fn parse(content: &str, format: Format) -> Result<Self> {
        if content.len() > MAX_PACKAGE_BYTES {
            return Err(error("package", "exceeds 10 MiB"));
        }
        let trimmed = content.trim_start();
        let format = if format == Format::Auto {
            if trimmed.starts_with("---") {
                Format::Markdown
            } else if trimmed.starts_with('{') {
                Format::Json
            } else if trimmed.lines().take(64).any(|line| {
                line.split_once('=').is_some_and(|(key, _)| {
                    matches!(
                        key.trim(),
                        "schema_version" | "name" | "instructions" | "system_prompt"
                    )
                })
            }) {
                Format::Toml
            } else if trimmed.lines().take(64).any(|line| {
                line.split_once(':').is_some_and(|(key, _)| {
                    matches!(
                        key.trim(),
                        "schema_version"
                            | "name"
                            | "instructions"
                            | "system_prompt"
                            | "display_name"
                    )
                })
            }) {
                Format::Yaml
            } else {
                Format::Markdown
            }
        } else {
            format
        };
        let mut body = None;
        let mut value: Value = match format {
            Format::Markdown | Format::Auto => {
                if trimmed.lines().next().map(str::trim) == Some("---") {
                    let start = trimmed
                        .find('\n')
                        .ok_or_else(|| error("frontmatter", "missing closing ---"))?
                        + 1;
                    let mut offset = start;
                    let mut end = None;
                    for line in trimmed[start..].split_inclusive('\n') {
                        // Indented separators belong to YAML literal file content,
                        // including embedded SKILL.md front matter.
                        if line.trim_end() == "---" {
                            end = Some((offset, offset + line.len()));
                            break;
                        }
                        offset += line.len();
                    }
                    let (end, after) =
                        end.ok_or_else(|| error("frontmatter", "missing closing ---"))?;
                    body = Some(trimmed[after..].to_string());
                    serde_yaml::from_str(&trimmed[start..end])
                        .map_err(|e| error("frontmatter", e))?
                } else {
                    serde_json::json!({"name": "agent", "instructions": content})
                }
            }
            Format::Json => serde_json::from_str(content).map_err(|e| error("json", e))?,
            Format::Yaml => serde_yaml::from_str(content).map_err(|e| error("yaml", e))?,
            Format::Toml => serde_json::to_value(
                toml::from_str::<toml::Value>(content).map_err(|e| error("toml", e))?,
            )
            .map_err(|e| error("toml", e))?,
        };
        let obj = value
            .as_object_mut()
            .ok_or_else(|| error("manifest", "must be an object"))?;
        let legacy = !obj.contains_key("schema_version");
        let legacy_id = take_id(obj, "id", legacy)?;
        let legacy_model_id = take_id(obj, "default_model_id", legacy)?;
        let legacy_harness_id = take_id(obj, "harness_id", legacy)?;
        if let Some(prompt) = obj.remove("system_prompt") {
            if let Some(instructions) = obj.get("instructions")
                && instructions != &prompt
            {
                return Err(error("instructions", "conflicts with legacy system_prompt"));
            }
            obj.insert("instructions".into(), prompt);
        }
        if let Some(body) = body
            && !body.trim().is_empty()
        {
            if obj
                .get("instructions")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty() && s.trim() != body.trim())
            {
                return Err(error("instructions", "conflicts with Markdown body"));
            }
            obj.insert("instructions".into(), Value::String(body));
        }
        if legacy {
            let name = obj
                .get("name")
                .or_else(|| obj.get("display_name"))
                .and_then(Value::as_str)
                .unwrap_or("agent");
            let slug = slug(name);
            obj.insert("name".into(), Value::String(slug));
        }
        let mut manifest = serde_json::from_value(value).map_err(|e| error("manifest", e))?;
        defaults(&mut manifest);
        let package = Self {
            manifest,
            legacy_id,
            legacy_model_id,
            legacy_harness_id,
        };
        package.validate()?;
        Ok(package)
    }

    pub fn validate(&self) -> Result<()> {
        let mut diagnostics = Vec::new();
        let mut fail = |path: String, message: &str| {
            diagnostics.push(Diagnostic {
                path,
                message: message.into(),
            })
        };
        let m = &self.manifest;
        if m.schema_version != SCHEMA_VERSION {
            fail("schema_version".into(), "unsupported version; expected 1");
        }
        if !valid_name(&m.name) {
            fail(
                "name".into(),
                "use 1-255 lowercase letters, digits or hyphens; start and end with a letter or digit",
            );
        }
        if !m.instructions.is_empty() && m.instructions_file.is_some() {
            fail(
                "instructions_file".into(),
                "choose inline instructions or instructions_file",
            );
        }
        if let Some(source) = &m.instructions_file
            && (!assets::safe_source(source) || !assets::allowed(Path::new(source)))
        {
            fail(
                "instructions_file".into(),
                "expected a safe relative package path; hidden credential sources are forbidden",
            );
        }
        if m.instructions.len() > MAX_FILE_BYTES {
            fail("instructions".into(), "exceeds 1 MiB");
        }
        if m.max_iterations == Some(0) || m.max_iterations.is_some_and(|n| n > 1000) {
            fail("max_iterations".into(), "must be between 1 and 1000");
        }
        if let Some(harness) = &m.harness
            && !valid_name(harness)
        {
            fail("harness".into(), "must be an addressable name, not an ID");
        }
        if let Some(model) = &m.model
            && (model.model.trim().is_empty() || model.provider.as_str().is_empty())
        {
            fail("model".into(), "provider and model must be non-empty");
        }
        let mut tools = BTreeSet::new();
        for (i, tool) in m.tools.iter().enumerate() {
            if !matches!(tool, ToolDefinition::ClientSide(_)) {
                fail(
                    format!("tools[{i}].type"),
                    "declare built-in tools through capabilities; portable tools must be client_side",
                );
            }
            if tool.name().is_empty()
                || tool.name().len() > 64
                || !tool
                    .name()
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                fail(
                    format!("tools[{i}].name"),
                    "expected 1-64 letters, digits, underscores or hyphens",
                );
            }
            if !tools.insert(tool.name()) {
                fail(format!("tools[{i}].name"), "duplicate tool name");
            }
            if !tool.parameters().is_object()
                || tool.parameters().get("type").and_then(Value::as_str) != Some("object")
            {
                fail(
                    format!("tools[{i}].parameters"),
                    "expected an object JSON schema",
                );
            }
        }
        let mut caps = BTreeSet::new();
        for (i, capability) in m.capabilities.iter().enumerate() {
            if everruns_contracts::validate_capability_id(capability.id()).is_err()
                || everruns_contracts::validate_capability_config(
                    capability.id(),
                    capability.config_value(),
                )
                .is_err()
            {
                fail(
                    format!("capabilities[{i}]"),
                    "invalid capability name or config",
                );
            }
            if capability.id().starts_with("skill:")
                || capability.id().starts_with("mcp:")
                || everruns_contracts::is_plugin_capability(capability.id())
            {
                fail(
                    format!("capabilities[{i}]"),
                    "use bundled skills or named MCP declarations instead of installed resource IDs",
                );
            }
            if !caps.insert(capability.capability_id()) {
                fail(format!("capabilities[{i}]"), "duplicate capability");
            }
        }
        // THREAT[TM-DOS-001]: imports may create a resource for each channel.
        if m.channels.len() > MAX_CHANNELS {
            fail("channels".into(), "at most 32 channel declarations");
        }
        for (name, channel) in &m.channels {
            let path = format!("channels.{name}");
            if !valid_name(name) {
                fail(path.clone(), "channel key must be an addressable name");
            }
            if !matches!(
                channel.channel_type.as_str(),
                "ag_ui"
                    | "public_chat"
                    | "slack"
                    | "fcp"
                    | "a2a"
                    | "api_endpoint"
                    | "schedule"
                    | "webhook"
            ) {
                fail(path.clone(), "unknown channel type");
            }
            if !channel.config.is_object() {
                fail(path.clone(), "config must be an object");
            }
            if contains_secret_or_id(&channel.config) {
                fail(
                    path,
                    "channel config must not contain credentials or resource IDs",
                );
            }
        }
        for (name, server) in &m.mcp_servers {
            if name.trim().is_empty() {
                fail("mcpServers".into(), "server name must not be empty");
            }
            if server
                .oauth_provider_id
                .as_deref()
                .is_some_and(|s| s.starts_with("mcp_"))
            {
                fail(
                    format!("mcpServers.{name}.oauth_provider_id"),
                    "use a catalog name instead of a resource ID",
                );
            }
            if server.preset.is_none() {
                match server.transport_type {
                    crate::McpServerTransportType::Http => {
                        let unsafe_url = match everruns_contracts::url_validation::validate_safe_url(
                            &server.url,
                        ) {
                            Ok(url) => {
                                !url.username().is_empty()
                                    || url.password().is_some()
                                    || url.query_pairs().any(|(key, _)| sensitive_key(&key))
                            }
                            Err(_) => true,
                        };
                        if unsafe_url {
                            fail(
                                format!("mcpServers.{name}.url"),
                                "expected a safe public HTTP(S) URL without credentials",
                            );
                        }
                    }
                    crate::McpServerTransportType::Stdio => {
                        if server
                            .command
                            .as_deref()
                            .is_none_or(|s| s.trim().is_empty())
                        {
                            fail(
                                format!("mcpServers.{name}.command"),
                                "stdio requires a command",
                            );
                        }
                    }
                }
            }
            for (key, value) in &server.headers {
                if sensitive_key(key) && !binding(value) {
                    fail(
                        format!("mcpServers.{name}.headers.{key}"),
                        "use a ${ENV_NAME} binding instead of a literal credential",
                    );
                }
            }
            for (key, value) in &server.env {
                if sensitive_key(key) && !binding(value) {
                    fail(
                        format!("mcpServers.{name}.env.{key}"),
                        "use a ${ENV_NAME} binding instead of a literal credential",
                    );
                }
            }
        }
        if m.initial_files.len() > MAX_FILES {
            fail("initial_files".into(), "at most 100 files");
        }
        let mut paths = BTreeSet::new();
        let mut bytes = 0;
        for (i, file) in m.initial_files.iter().enumerate() {
            match file {
                File::Inline(file) => {
                    match assets::workspace_path(&file.path) {
                        Ok(path) => {
                            if !paths.insert(path) {
                                fail(format!("initial_files[{i}].path"), "duplicate destination");
                            }
                        }
                        Err(_) => {
                            fail(format!("initial_files[{i}].path"), "invalid workspace path")
                        }
                    }
                    if !matches!(file.encoding.as_str(), "text" | "base64") {
                        fail(
                            format!("initial_files[{i}].encoding"),
                            "expected text or base64",
                        );
                    }
                    match crate::session_file::SessionFile::decode_content(
                        &file.content,
                        &file.encoding,
                    ) {
                        Ok(content) => {
                            if let Ok(path) = assets::workspace_path(&file.path)
                                && let Some(skill) = path.strip_prefix("/.agents/skills/")
                                && let Some(dir) = skill.strip_suffix("/SKILL.md")
                            {
                                let valid = !dir.contains('/')
                                    && std::str::from_utf8(&content)
                                        .ok()
                                        .and_then(|text| crate::skill::parse_skill_md(text).ok())
                                        .is_some_and(|skill| skill.name == dir);
                                if !valid {
                                    fail(
                                        format!("initial_files[{i}].content"),
                                        "invalid SKILL.md or name does not match its directory",
                                    );
                                }
                            }
                            bytes += content.len();
                            if content.len() > MAX_FILE_BYTES {
                                fail(format!("initial_files[{i}]"), "exceeds 1 MiB");
                            }
                        }
                        Err(_) => fail(format!("initial_files[{i}].content"), "invalid base64"),
                    }
                }
                File::Pattern(source) => {
                    if !assets::safe_source(source) {
                        fail(
                            format!("initial_files[{i}]"),
                            "source must stay inside the package",
                        );
                    }
                }
                File::Source(source) => {
                    if !assets::safe_source(&source.source) {
                        fail(
                            format!("initial_files[{i}].source"),
                            "source must stay inside the package",
                        );
                    }
                }
            }
        }
        if bytes > MAX_INITIAL_BYTES {
            fail("initial_files".into(), "decoded content exceeds 5 MiB");
        }
        if diagnostics.is_empty() {
            Ok(())
        } else {
            Err(PackageError(diagnostics))
        }
    }

    /// Require fully materialized assets before any execution or API import.
    pub fn files(&self) -> Result<Vec<InitialFile>> {
        self.validate()?;
        if self.manifest.instructions.trim().is_empty() {
            return Err(error(
                "instructions",
                "required; provide instructions or instructions.md",
            ));
        }
        if self.manifest.instructions_file.is_some() || !self.manifest.skills.is_empty() {
            return Err(error(
                "assets",
                "file references require a folder or ZIP package",
            ));
        }
        self.manifest
            .initial_files
            .iter()
            .enumerate()
            .map(|(i, file)| match file {
                File::Inline(file) => {
                    let mut file = file.clone();
                    file.path = assets::workspace_path(&file.path)?;
                    Ok(file)
                }
                _ => Err(error(
                    format!("initial_files[{i}]"),
                    "unresolved source; load the folder or ZIP package",
                )),
            })
            .collect()
    }

    pub fn to_string(&self, format: Format) -> Result<String> {
        self.files()?;
        let mut value = serde_json::to_value(&self.manifest).map_err(|e| error("manifest", e))?;
        match format {
            Format::Auto | Format::Markdown => {
                value
                    .as_object_mut()
                    .ok_or_else(|| error("manifest", "expected an object"))?
                    .remove("instructions");
                let yaml = serde_yaml::to_string(&value).map_err(|e| error("yaml", e))?;
                Ok(format!("---\n{yaml}---\n{}", self.manifest.instructions))
            }
            Format::Json => serde_json::to_string_pretty(&value).map_err(|e| error("json", e)),
            Format::Yaml => serde_yaml::to_string(&value).map_err(|e| error("yaml", e)),
            Format::Toml => toml::to_string_pretty(&self.manifest).map_err(|e| error("toml", e)),
        }
    }

    /// Resolve credential placeholders only through an explicit host binding.
    pub fn bind_mcp(&self, resolve: impl Fn(&str) -> Option<String>) -> Result<ScopedMcpServers> {
        let mut servers = self.manifest.mcp_servers.clone();
        for (name, server) in &mut servers {
            for value in server.headers.values_mut().chain(server.env.values_mut()) {
                if binding(value) {
                    let key = &value[2..value.len() - 1];
                    *value = resolve(key).ok_or_else(|| {
                        error(
                            format!("mcpServers.{name}"),
                            format!("missing credential binding {key}"),
                        )
                    })?;
                }
            }
        }
        Ok(servers)
    }
}

fn take_id<T: for<'de> Deserialize<'de>>(
    obj: &mut serde_json::Map<String, Value>,
    key: &str,
    legacy: bool,
) -> Result<Option<T>> {
    match obj.remove(key) {
        Some(value) if !value.is_null() => {
            if !legacy {
                return Err(error(
                    key,
                    "IDs are accepted only in unversioned legacy files",
                ));
            }
            serde_json::from_value(value)
                .map(Some)
                .map_err(|e| error(key, e))
        }
        _ => Ok(None),
    }
}
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && name
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}
fn slug(name: &str) -> String {
    name.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
pub fn binding(value: &str) -> bool {
    value.starts_with("${")
        && value.ends_with('}')
        && value.len() > 3
        && value[2..value.len() - 1]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
pub fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace('-', "_");
    key.contains("token")
        || key.contains("secret")
        || key.contains("password")
        || key.contains("api_key")
        || key == "authorization"
        || key == "cookie"
        || key == "provisioned_app"
}
pub fn contains_secret_or_id(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            sensitive_key(key)
                || key == "id"
                || key.ends_with("_id")
                || contains_secret_or_id(value)
        }),
        Value::Array(values) => values.iter().any(contains_secret_or_id),
        _ => false,
    }
}

/// Replace credential-bearing headers and environment values with explicit
/// requirements. Literal credentials never leave a host through package export.
pub fn redact_mcp(servers: &ScopedMcpServers) -> ScopedMcpServers {
    let mut servers = servers.clone();
    for (name, server) in &mut servers {
        for (key, value) in server.headers.iter_mut().chain(server.env.iter_mut()) {
            if sensitive_key(key) && !binding(value) {
                let name = format!("MCP_{name}_{key}")
                    .to_ascii_uppercase()
                    .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
                *value = format!("${{{name}}}");
            }
        }
    }
    servers
}

/// Whether a relative asset path can be included without hidden credentials.
pub fn is_package_asset_path(path: &str) -> bool {
    assets::safe_source(path) && assets::allowed(Path::new(path))
}

fn defaults(m: &mut Manifest) {
    if !m.initial_files.is_empty()
        && !m
            .capabilities
            .iter()
            .any(|c| c.id() == "session_file_system")
    {
        m.capabilities
            .push(CapabilityRef::new("session_file_system"));
    }
    if (!m.skills.is_empty()
        || m.initial_files
            .iter()
            .any(|f| matches!(f, File::Inline(file) if file.path.contains("/.agents/skills/"))))
        && !m.capabilities.iter().any(|c| c.id() == "skills")
    {
        m.capabilities.push(CapabilityRef::new("skills"));
    }
}

#[cfg(feature = "openapi")]
#[derive(utoipa::ToSchema)]
#[allow(dead_code)]
struct PackageModel {
    provider: String,
    model: String,
}
#[cfg(feature = "openapi")]
#[derive(serde::Serialize, utoipa::ToSchema)]
#[serde(untagged)]
#[allow(dead_code)]
enum PackageCapability {
    Name(String),
    Reference(PackageCapabilityReference),
}
#[cfg(feature = "openapi")]
#[derive(serde::Serialize, utoipa::ToSchema)]
#[allow(dead_code)]
struct PackageCapabilityReference {
    #[serde(rename = "ref")]
    name: String,
    #[serde(default)]
    config: Value,
}

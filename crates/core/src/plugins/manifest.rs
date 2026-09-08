// Plugin manifest types for the cross-host plugin.json dialect.
//
// This module provides tolerant deserialization of plugin.json files that are
// compatible with Claude Code, Codex, and Cursor plugin conventions.
// Unrecognized top-level fields are collected as warnings, not errors.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const AGENT_PLUGINS_V1_MANIFEST_SCHEMA: &str =
    "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json";
pub const AGENT_PLUGINS_V1_MCP_SCHEMA: &str =
    "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json";
pub const AGENT_PLUGINS_V1_MANIFEST_SCHEMA_JSON: &str =
    include_str!("schemas/1.0.0/plugin.schema.json");
pub const AGENT_PLUGINS_V1_MCP_SCHEMA_JSON: &str = include_str!("schemas/1.0.0/mcp.schema.json");

/// Author metadata in a plugin manifest.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginAuthor {
    /// Author's display name (required).
    #[serde(default)]
    pub name: String,
    /// Optional contact email.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Optional homepage URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// `mcpServers` in a plugin manifest can be:
///   - A string path to a `.mcp.json` file (e.g. `"./.mcp.json"`)
///   - An array of string paths
///   - An inline map of server name → server config objects (same shape as `.mcp.json`)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpServersField {
    /// A single path string to a `.mcp.json` file.
    Path(String),
    /// Multiple path strings to `.mcp.json` files.
    Paths(Vec<String>),
    /// Inline server configuration map.
    Inline(HashMap<String, serde_json::Value>),
}

/// A string or array-of-strings field for component path overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringOrArray {
    /// A single string value.
    Single(String),
    /// Multiple string values.
    Multiple(Vec<String>),
}

impl StringOrArray {
    /// Flatten to a vec of strings.
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            StringOrArray::Single(s) => vec![s.clone()],
            StringOrArray::Multiple(v) => v.clone(),
        }
    }
}

/// Parsed plugin manifest (`plugin.json`).
///
/// This is a tolerant representation: unrecognized top-level fields are
/// collected in `extra` for warning purposes instead of failing parsing.
/// This mirrors Claude Code's own loading policy for unrecognized fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    /// Canonical Agent Plugins schema identifier. Absent for legacy host manifests.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,

    /// Unique plugin name (kebab-case). Required.
    pub name: String,

    /// Human-readable display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// Semver string or freeform version tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Short description of the plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Author metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<PluginAuthor>,

    /// Plugin homepage URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,

    /// Source repository URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,

    /// SPDX license identifier or freeform string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,

    /// Search keywords.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,

    /// Relative path to a bundled SVG icon. Remote and data URLs are rejected
    /// during compilation so capability lists never become an asset-loading
    /// or tracking surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// Client-specific Agent Plugins extension data.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extensions: HashMap<String, serde_json::Value>,

    // --- Component path overrides ---
    /// Override for the skills directory or file list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<StringOrArray>,

    /// Override for the commands directory or file list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands: Option<StringOrArray>,

    /// Override for the agents directory or file list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents: Option<StringOrArray>,

    /// MCP server configuration: path, paths, or inline map.
    #[serde(
        rename = "mcpServers",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub mcp_servers: Option<McpServersField>,

    /// Unrecognized top-level fields, preserved for warning generation.
    /// The presence of any keys here becomes an install warning.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl PluginManifest {
    pub fn is_agent_plugins_v1(&self) -> bool {
        self.schema.as_deref() == Some(AGENT_PLUGINS_V1_MANIFEST_SCHEMA)
    }
}

pub fn parse_agent_plugins_v1_manifest(
    text: &str,
) -> Result<(PluginManifest, Vec<String>), String> {
    let mut value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| format!("failed to parse plugin.json: {error}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "plugin.json must contain a JSON object".to_string())?;

    let schema = object.get("$schema").and_then(serde_json::Value::as_str);
    if schema != Some(AGENT_PLUGINS_V1_MANIFEST_SCHEMA) {
        return Err(format!(
            "unsupported Agent Plugins schema '{}'; supported schema is {AGENT_PLUGINS_V1_MANIFEST_SCHEMA}",
            schema.unwrap_or("<missing>")
        ));
    }

    let permitted = [
        "$schema",
        "name",
        "version",
        "description",
        "author",
        "homepage",
        "repository",
        "license",
        "keywords",
        "extensions",
    ];
    let unknown: Vec<String> = object
        .keys()
        .filter(|key| !permitted.contains(&key.as_str()))
        .cloned()
        .collect();
    let mut warnings = Vec::new();
    for key in unknown {
        object.remove(&key);
        warnings.push(format!(
            "plugin.json: unrecognized field '{key}' was ignored"
        ));
    }

    if object
        .get("extensions")
        .is_some_and(|extensions| !extensions.is_object())
    {
        object.remove("extensions");
        warnings.push("plugin.json: non-object 'extensions' field was ignored".to_string());
    }

    validate_json_schema(AGENT_PLUGINS_V1_MANIFEST_SCHEMA_JSON, &value, "plugin.json")?;
    let manifest = serde_json::from_value(value)
        .map_err(|error| format!("failed to parse plugin.json: {error}"))?;
    Ok((manifest, warnings))
}

pub(crate) fn validate_json_schema(
    schema_json: &str,
    value: &serde_json::Value,
    label: &str,
) -> Result<(), String> {
    let schema: serde_json::Value = serde_json::from_str(schema_json)
        .map_err(|error| format!("embedded {label} schema is invalid: {error}"))?;
    let validator = jsonschema::draft202012::options()
        .build(&schema)
        .map_err(|error| format!("embedded {label} schema is invalid: {error}"))?;
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|error| {
            let path = error.instance_path().to_string();
            if path.is_empty() {
                error.to_string()
            } else {
                format!("{path}: {error}")
            }
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("invalid {label}: {}", errors.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_manifest_preserves_known_unknown_and_component_fields() {
        let input = serde_json::json!({
            "name":"microsoft-docs","displayName":"Microsoft Docs","version":"0.1.0",
            "description":"Search official Microsoft docs.","license":"MIT",
            "author":{"name":"Docs team","email":"docs@example.com","url":"https://example.com"},
            "homepage":"https://example.com","repository":"https://example.com/repo",
            "keywords":["docs","search"],"icon":"./assets/icon.svg",
            "interface":{"displayName":"Test","category":"Dev"},
            "skills":["./skills/","./extra/"],"commands":"./commands/","agents":["./agents/"],
            "mcpServers":{"docs":{"url":"https://example.com/mcp"}}
        });
        let manifest: PluginManifest = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(manifest.name, "microsoft-docs");
        assert_eq!(
            manifest.description.as_deref(),
            Some("Search official Microsoft docs.")
        );
        assert_eq!(
            manifest.skills.as_ref().unwrap().to_vec(),
            ["./skills/", "./extra/"]
        );
        assert_eq!(
            manifest.commands.as_ref().unwrap().to_vec(),
            ["./commands/"]
        );
        assert_eq!(manifest.agents.as_ref().unwrap().to_vec(), ["./agents/"]);
        assert_eq!(
            manifest.extra,
            HashMap::from([(
                "interface".into(),
                serde_json::json!({"displayName":"Test","category":"Dev"})
            )])
        );
        assert!(
            matches!(&manifest.mcp_servers,Some(McpServersField::Inline(map)) if map.get("docs")==Some(&serde_json::json!({"url":"https://example.com/mcp"})))
        );
        assert_eq!(serde_json::to_value(manifest).unwrap(), input);
        let minimal: PluginManifest = serde_json::from_str(r#"{"name":"minimal"}"#).unwrap();
        assert_eq!(
            serde_json::to_value(minimal).unwrap(),
            serde_json::json!({"name":"minimal"})
        );
    }

    #[test]
    fn component_unions_preserve_values_and_reject_wrong_shapes() {
        for (input, expected) in [
            (serde_json::json!("./skills/"), vec!["./skills/"]),
            (
                serde_json::json!(["./skills/", "./extra/"]),
                vec!["./skills/", "./extra/"],
            ),
            (serde_json::json!([]), vec![]),
        ] {
            let field: StringOrArray = serde_json::from_value(input.clone()).unwrap();
            assert_eq!(field.to_vec(), expected);
            assert_eq!(serde_json::to_value(field).unwrap(), input);
        }
        let path: McpServersField =
            serde_json::from_value(serde_json::json!("./.mcp.json")).unwrap();
        assert!(matches!(path,McpServersField::Path(value) if value=="./.mcp.json"));
        let paths: McpServersField =
            serde_json::from_value(serde_json::json!(["./a.json", "./b.json"])).unwrap();
        assert!(matches!(paths,McpServersField::Paths(value) if value==["./a.json","./b.json"]));
        for value in [
            serde_json::json!(42),
            serde_json::json!(true),
            serde_json::json!(["ok", 42]),
        ] {
            assert!(serde_json::from_value::<StringOrArray>(value.clone()).is_err());
            assert!(serde_json::from_value::<McpServersField>(value).is_err());
        }
        assert!(serde_json::from_str::<PluginManifest>(r#"{"description":"no name"}"#).is_err());
        assert!(serde_json::from_str::<PluginManifest>(r#"{"name":42}"#).is_err());
    }
}

// Plugin manifest types for the cross-host plugin.json dialect.
//
// This module provides tolerant deserialization of plugin.json files that are
// compatible with Claude Code, Codex, and Cursor plugin conventions.
// Unrecognized top-level fields are collected as warnings, not errors.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

    /// User-configurable fields prompted at plugin enable time.
    ///
    /// Each key is a config field name; each value is a descriptor with
    /// `type`, `title`, `description`, `default`, `enum`, `required`, and
    /// optionally `sensitive: true` (maps to a password widget in the UI).
    /// Compiled into `config_schema` / `config_ui_schema` on the capability.
    #[serde(
        rename = "userConfig",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub user_config: Option<HashMap<String, serde_json::Value>>,

    /// Unrecognized top-level fields, preserved for warning generation.
    /// The presence of any keys here becomes an install warning.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_manifest() {
        let json = r#"{
            "name": "microsoft-docs",
            "displayName": "Microsoft Docs",
            "version": "0.1.0",
            "description": "Search official Microsoft docs.",
            "license": "MIT"
        }"#;
        let manifest: PluginManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.name, "microsoft-docs");
        assert_eq!(manifest.display_name.as_deref(), Some("Microsoft Docs"));
        assert_eq!(manifest.version.as_deref(), Some("0.1.0"));
        assert_eq!(manifest.license.as_deref(), Some("MIT"));
        assert!(manifest.extra.is_empty());
    }

    #[test]
    fn unrecognized_fields_land_in_extra() {
        let json = r#"{
            "name": "test-plugin",
            "description": "A test plugin.",
            "interface": {"displayName": "Test", "category": "Dev"}
        }"#;
        let manifest: PluginManifest = serde_json::from_str(json).unwrap();
        assert!(manifest.extra.contains_key("interface"));
    }

    #[test]
    fn mcp_servers_path_string() {
        let json = r#"{"name": "x", "description": "y", "mcpServers": "./.mcp.json"}"#;
        let manifest: PluginManifest = serde_json::from_str(json).unwrap();
        assert!(matches!(
            manifest.mcp_servers,
            Some(McpServersField::Path(_))
        ));
    }

    #[test]
    fn skills_string_or_array() {
        let json = r#"{"name": "x", "description": "y", "skills": "./skills/"}"#;
        let manifest: PluginManifest = serde_json::from_str(json).unwrap();
        assert!(matches!(manifest.skills, Some(StringOrArray::Single(_))));
    }
}

//! Load MCP servers from a workspace `.mcp.json` (specs/runtime-mcp.md D8).
//!
//! Shape matches the `mcpServers` object every MCP client understands:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "docs": { "type": "http", "url": "https://example.com/mcp",
//!               "headers": { "Authorization": "Bearer t0ken" } }
//!   }
//! }
//! ```
//!
//! Missing file → no servers. Both remote HTTP and local stdio
//! (`{ "type": "stdio", "command": ..., "args": [...] }`) servers are
//! supported; the CLI builds with the runtime's `mcp-stdio` feature.

use std::path::Path;

use anyhow::{Context, Result};
use everruns_core::ScopedMcpServers;
use serde::Deserialize;

/// File name read from the workspace root.
pub const MCP_CONFIG_FILE: &str = ".mcp.json";

#[derive(Debug, Deserialize)]
struct McpConfigFile {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: ScopedMcpServers,
}

/// Read `<workspace_root>/.mcp.json` into scoped MCP servers. Returns an empty
/// set when the file is absent.
pub fn load_mcp_servers(workspace_root: &Path) -> Result<ScopedMcpServers> {
    let path = workspace_root.join(MCP_CONFIG_FILE);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ScopedMcpServers::default());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("reading {}", path.display()));
        }
    };

    let config: McpConfigFile =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    Ok(config.mcp_servers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::McpServerTransportType;

    fn write(dir: &Path, contents: &str) {
        std::fs::write(dir.join(MCP_CONFIG_FILE), contents).unwrap();
    }

    #[test]
    fn missing_file_yields_no_servers() {
        let dir = tempfile::tempdir().unwrap();
        let servers = load_mcp_servers(dir.path()).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn parses_http_server_with_headers() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            r#"{ "mcpServers": {
                "docs": {
                    "type": "http",
                    "url": "https://example.com/mcp",
                    "headers": { "Authorization": "Bearer t0ken" }
                }
            }}"#,
        );

        let servers = load_mcp_servers(dir.path()).unwrap();
        let docs = servers.get("docs").expect("docs server");
        assert_eq!(docs.transport_type, McpServerTransportType::Http);
        assert_eq!(docs.url, "https://example.com/mcp");
        assert_eq!(
            docs.headers.get("Authorization").map(String::as_str),
            Some("Bearer t0ken")
        );
        assert!(docs.tool_discovery, "tool discovery defaults on");
    }

    #[test]
    fn parses_multiple_servers() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            r#"{ "mcpServers": {
                "a": { "type": "http", "url": "https://a.example.com/mcp" },
                "b": { "url": "https://b.example.com/mcp" }
            }}"#,
        );
        let servers = load_mcp_servers(dir.path()).unwrap();
        assert_eq!(servers.len(), 2);
        // `type` is optional and defaults to http.
        assert_eq!(servers["b"].transport_type, McpServerTransportType::Http);
    }

    #[test]
    fn empty_object_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "{}");
        assert!(load_mcp_servers(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn parses_stdio_server() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            r#"{ "mcpServers": {
                "fs": {
                    "type": "stdio",
                    "command": "mcp-server-filesystem",
                    "args": ["/work"],
                    "env": { "RUST_LOG": "info" }
                }
            }}"#,
        );

        let servers = load_mcp_servers(dir.path()).unwrap();
        let fs = servers.get("fs").expect("fs server");
        assert_eq!(fs.transport_type, McpServerTransportType::Stdio);
        assert_eq!(fs.command.as_deref(), Some("mcp-server-filesystem"));
        assert_eq!(fs.args, vec!["/work".to_string()]);
        assert_eq!(fs.env.get("RUST_LOG").map(String::as_str), Some("info"));
    }
}

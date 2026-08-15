use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use everruns::McpServer;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpConfig {
    #[serde(default)]
    mcp_servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum McpServerConfig {
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
}

pub fn load(path: &Path) -> Result<Vec<McpServer>> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("read MCP configuration {}", path.display()))?;
    let config: McpConfig = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse MCP configuration {}", path.display()))?;
    let mut servers = Vec::with_capacity(config.mcp_servers.len());
    for (name, server) in config.mcp_servers {
        if name.trim().is_empty() {
            bail!("MCP server names must not be blank");
        }
        servers.push(match server {
            McpServerConfig::Http { url, headers } => headers
                .into_iter()
                .fold(McpServer::http(name, url), |server, (key, value)| {
                    server.header(key, value)
                }),
            McpServerConfig::Stdio { command, args, env } => env.into_iter().fold(
                McpServer::stdio(name, command).args(args),
                |server, (key, value)| server.env(key, value),
            ),
        });
    }
    Ok(servers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_and_stdio_servers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        std::fs::write(
            &path,
            r#"{
              "mcpServers": {
                "docs": {"type":"http","url":"https://example.test/mcp","headers":{"x-test":"yes"}},
                "local": {"type":"stdio","command":"python3","args":["server.py"],"env":{"MODE":"test"}}
              }
            }"#,
        )
        .unwrap();
        let servers = load(&path).unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name(), "docs");
        assert_eq!(servers[1].name(), "local");
    }
}

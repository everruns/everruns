//! Local-process MCP transport over stdio (knowledge/integrations/runtime-mcp.md D2).
//!
//! Compiled only with the `stdio` feature, which the hosted server/worker
//! builds never enable — this is the hard-off boundary. Each call spawns the
//! server process, performs the MCP `initialize` handshake, issues one
//! request, and tears the process down. Per-invocation spawn keeps lifecycle
//! management trivial and avoids shared-process concurrency hazards; it is
//! appropriate for single-tenant runtime/CLI hosts.

use crate::auth::McpCredential;
use crate::transport::{McpConnection, McpEndpoint, McpTransport};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use everruns_core::{McpToolCallResult, McpToolDefinition, McpToolsListResult};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const STDIO_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Default, Clone)]
pub struct StdioTransport;

impl StdioTransport {
    async fn run(&self, connection: &McpConnection, method: &str, params: Value) -> Result<Value> {
        let (command, args, env) = match &connection.endpoint {
            McpEndpoint::Stdio { command, args, env } => (command, args, env),
            _ => {
                return Err(anyhow!(
                    "StdioTransport received a non-stdio endpoint for server '{}'",
                    connection.name
                ));
            }
        };

        let mut child = Command::new(command)
            .args(args)
            .envs(env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            // Kill the child if this future is dropped/cancelled (e.g. on a
            // timeout) so it cannot outlive the call.
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn MCP server '{}': {e}", connection.name))?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("MCP stdio server '{}' has no stdin", connection.name))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("MCP stdio server '{}' has no stdout", connection.name))?;
        let mut reader = BufReader::new(stdout).lines();

        let outcome = tokio::time::timeout(STDIO_TIMEOUT, async {
            // MCP initialize handshake.
            write_message(
                &mut stdin,
                &json!({
                    "jsonrpc": "2.0",
                    "id": 0,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {},
                        "clientInfo": { "name": "everruns-mcp", "version": "0" }
                    }
                }),
            )
            .await?;
            read_response_for(&mut reader, 0).await?;
            write_message(
                &mut stdin,
                &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            )
            .await?;

            // The actual request.
            write_message(
                &mut stdin,
                &json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }),
            )
            .await?;
            read_response_for(&mut reader, 1).await
        })
        .await;

        // Terminate and reap the child so each invocation fully cleans up
        // (no lingering process, no zombie). `start_kill` sends SIGKILL, so
        // `wait` returns promptly.
        let _ = child.start_kill();
        let _ = child.wait().await;

        outcome.map_err(|_| anyhow!("MCP stdio server '{}' timed out", connection.name))?
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn list_tools(
        &self,
        connection: &McpConnection,
        _credential: Option<&McpCredential>,
    ) -> Result<Vec<McpToolDefinition>> {
        let result = self.run(connection, "tools/list", json!({})).await?;
        let parsed: McpToolsListResult = serde_json::from_value(result)?;
        Ok(parsed.tools)
    }

    async fn call_tool(
        &self,
        connection: &McpConnection,
        tool_name: &str,
        arguments: Value,
        _credential: Option<&McpCredential>,
    ) -> Result<McpToolCallResult> {
        let result = self
            .run(
                connection,
                "tools/call",
                json!({ "name": tool_name, "arguments": arguments }),
            )
            .await?;
        let parsed: McpToolCallResult = serde_json::from_value(result)?;
        Ok(parsed)
    }
}

async fn write_message<W: AsyncWriteExt + Unpin>(writer: &mut W, message: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(message)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// Read JSON-RPC messages until one matches `id`, returning its `result`.
/// Notifications and log lines (no matching `id`) are skipped.
async fn read_response_for<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut tokio::io::Lines<R>,
    id: i64,
) -> Result<Value> {
    while let Some(line) = reader.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            if let Some(error) = value.get("error") {
                return Err(anyhow!("MCP stdio error: {error}"));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
    Err(anyhow!(
        "MCP stdio server closed before responding to id {id}"
    ))
}

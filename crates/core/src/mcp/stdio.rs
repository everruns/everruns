//! Local-process MCP transport over stdio.
//!
//! Compiled only with the `stdio` feature, which the hosted server/worker
//! builds never enable — this is the hard-off boundary. Each call spawns the
//! server process, performs the MCP `initialize` handshake, issues one
//! request, and tears the process down. Per-invocation spawn keeps lifecycle
//! management trivial and avoids shared-process concurrency hazards; it is
//! appropriate for single-tenant runtime/CLI hosts.

use crate::mcp::auth::McpCredential;
use crate::mcp::discovery::{DiscoveryAccumulator, check_page_bytes};
use crate::mcp::transport::{McpConnection, McpEndpoint, McpTransport};
use crate::{McpToolCallResult, McpToolDefinition, McpToolsListResult};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const STDIO_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Default, Clone)]
pub struct StdioTransport;

impl StdioTransport {
    async fn list_tools_paginated(
        &self,
        connection: &McpConnection,
    ) -> Result<Vec<McpToolDefinition>> {
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
        let mut reader = BufReader::new(stdout);

        let outcome = tokio::time::timeout(crate::mcp::MCP_DISCOVERY_DEADLINE, async {
            write_message(&mut stdin, &json!({
                "jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "everruns-mcp", "version": "0"}}
            })).await?;
            read_response_for_bounded(&mut reader, 0, crate::mcp::MCP_DISCOVERY_MAX_BYTES)
                .await?;
            write_message(&mut stdin, &json!({"jsonrpc":"2.0", "method":"notifications/initialized"})).await?;

            let mut accumulator = DiscoveryAccumulator::new();
            let mut cursor: Option<String> = None;
            let mut id = 1_i64;
            loop {
                let mut params = serde_json::Map::new();
                if let Some(cursor) = cursor.as_ref() {
                    params.insert("cursor".into(), Value::String(cursor.clone()));
                }
                write_message(&mut stdin, &json!({"jsonrpc":"2.0", "id":id, "method":"tools/list", "params":params})).await?;
                let (result, response_bytes) = read_response_for_bounded(
                    &mut reader,
                    id,
                    crate::mcp::MCP_DISCOVERY_MAX_BYTES,
                )
                    .await
                    .with_context(|| format!("MCP tool discovery incomplete on page {id}"))?;
                check_page_bytes(response_bytes)?;
                accumulator.add_response_bytes(response_bytes)?;
                let page: McpToolsListResult = serde_json::from_value(result)?;
                cursor = accumulator.push_page(page.tools, page.next_cursor)?;
                if cursor.is_none() { break; }
                id += 1;
            }
            Ok::<_, anyhow::Error>(accumulator.finish())
        }).await;
        let _ = child.start_kill();
        let _ = child.wait().await;
        outcome.map_err(|_| {
            crate::mcp::McpDiscoveryError::DeadlineExceeded(crate::mcp::MCP_DISCOVERY_DEADLINE)
        })?
    }

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

        // `tools/call` honors the server's configured timeout; the other
        // methods are quick lookups and keep the fixed limit.
        let timeout = if method == "tools/call" {
            connection.tool_timeout()
        } else {
            STDIO_TIMEOUT
        };
        let outcome = tokio::time::timeout(timeout, async {
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
        self.list_tools_paginated(connection).await
    }

    async fn call_tool(
        &self,
        connection: &McpConnection,
        tool_name: &str,
        arguments: Value,
        _credential: Option<&McpCredential>,
        idempotency_key: Option<&str>,
    ) -> Result<McpToolCallResult> {
        let mut params = json!({ "name": tool_name, "arguments": arguments });
        if let Some(key) = idempotency_key {
            params["_meta"] = json!({ crate::mcp::protocol::IDEMPOTENCY_KEY_META_KEY: key });
        }
        let result = self.run(connection, "tools/call", params).await?;
        let parsed: McpToolCallResult = serde_json::from_value(result)?;
        Ok(parsed)
    }

    async fn request(
        &self,
        connection: &McpConnection,
        method: &str,
        params: Value,
        _credential: Option<&McpCredential>,
    ) -> Result<Value> {
        self.run(connection, method, params).await
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
            if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
                return Err(crate::mcp::McpRpcError {
                    code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .to_string(),
                    data: error.get("data").cloned(),
                }
                .into());
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
    Err(anyhow!(
        "MCP stdio server closed before responding to id {id}"
    ))
}

/// Read one JSON-RPC result from stdio while bounding each server message
/// before it is accumulated into an owned buffer.
async fn read_response_for_bounded<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    id: i64,
    max_line_bytes: usize,
) -> Result<(Value, usize)> {
    loop {
        let mut line = Vec::new();
        loop {
            let available = reader.fill_buf().await?;
            if available.is_empty() {
                if line.is_empty() {
                    return Err(anyhow!("MCP stdio server closed before response id {id}"));
                }
                break;
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let take = newline.map_or(available.len(), |position| position + 1);
            if line.len().saturating_add(take) > max_line_bytes {
                return Err(crate::mcp::McpDiscoveryError::LimitExceeded {
                    resource: "page bytes",
                    limit: max_line_bytes,
                }
                .into());
            }
            line.extend_from_slice(&available[..take]);
            reader.consume(take);
            if newline.is_some() {
                break;
            }
        }
        let text = std::str::from_utf8(&line)?.trim();
        if text.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
                return Err(crate::mcp::McpRpcError {
                    code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .to_string(),
                    data: error.get("data").cloned(),
                }
                .into());
            }
            return Ok((
                value.get("result").cloned().unwrap_or(Value::Null),
                line.len(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn oversized_stdio_message_is_rejected_before_json_parsing() {
        let mut input = vec![b'x'; crate::mcp::MCP_DISCOVERY_MAX_BYTES + 1];
        input.push(b'\n');
        let mut reader = BufReader::new(std::io::Cursor::new(input));
        let error = read_response_for_bounded(&mut reader, 1, crate::mcp::MCP_DISCOVERY_MAX_BYTES)
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<crate::mcp::McpDiscoveryError>(),
            Some(crate::mcp::McpDiscoveryError::LimitExceeded {
                resource: "page bytes",
                ..
            })
        ));
    }
}

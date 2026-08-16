# everruns-mcp

> Transport-agnostic Model Context Protocol (MCP) client for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-mcp.svg)](https://crates.io/crates/everruns-mcp)
[![Documentation](https://docs.rs/everruns-mcp/badge.svg)](https://docs.rs/everruns-mcp)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-mcp` is the shared [MCP](https://modelcontextprotocol.io) **client**
used across Everruns hosts. It owns the JSON-RPC client, credential acquisition,
result mapping, and tool-call routing so every host wires MCP the same way
without duplicating protocol logic, letting agents discover and call tools
exposed by any MCP server.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It plugs MCP tools into the
[`everruns-core`](https://crates.io/crates/everruns-core) `ToolRegistry` as
first-class tools.

## Features

- `McpTransport` with an always-on `HttpTransport` (Streamable HTTP, over the
  platform egress boundary with DNS-pinned SSRF validation).
- A feature-gated `StdioTransport` (`--features stdio`) for local-process MCP
  servers, for hosts that opt in to a non-HTTP transport.
- Pluggable `McpAuthProvider` (web OAuth, static tokens, env, …) so non-web
  (CLI) hosts can authenticate.
- `McpExecutor` (implements `everruns_core::McpToolInvoker`) so hosts register
  MCP tools as first-class `Tool`s in the regular `ToolRegistry`.

## Quick Example

```rust
use everruns_mcp::{McpClient, McpConnection};

# async fn run() -> anyhow::Result<()> {
let client = McpClient::direct();
let connection = McpConnection::http("docs", "https://example.com/mcp");
let tools = client.discover(&connection).await?;
let result = client
    .call(&connection, &tools[0].name, serde_json::json!({}))
    .await?;
# let _ = result;
# Ok(())
# }
```

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-mcp)
- [Equip agents with tools](https://docs.everruns.com/how-to/equip-agents-with-tools/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).

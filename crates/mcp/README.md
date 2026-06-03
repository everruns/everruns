# everruns-mcp

Transport-agnostic [MCP](https://modelcontextprotocol.io) (Model Context
Protocol) **client** shared by the Everruns runtime, worker, and server.

It owns the JSON-RPC client, credential acquisition, result mapping, and
tool-call routing so each host wires MCP the same way without duplicating the
protocol logic. See [`specs/runtime-mcp.md`](../../specs/runtime-mcp.md) for the
design.

## Features

- `McpTransport` with an always-on `HttpTransport` (Streamable HTTP, over the
  platform egress boundary with DNS-pinned SSRF validation).
- A feature-gated `StdioTransport` (`--features stdio`) for local-process MCP
  servers. **Not** enabled by the hosted server/worker builds — this is the
  hard-off boundary for non-HTTP transport.
- Pluggable `McpAuthProvider` (web OAuth, static tokens, env, …) so non-web
  (CLI) hosts can authenticate.
- `McpExecutor` / `CompositeToolExecutor` to route `mcp_*` tool calls.

## Example

```rust
use std::sync::Arc;
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

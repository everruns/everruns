---
title: "MCP resource reads support input continuations"
description: "The Framework HTTP MCP client resumes resource reads that request input, preserving consent, resource identity and one deadline across retries."
date: 2026-10-30
kind: fix
significance: minor
products: [framework]
release: "0.50.1"
surfaces: [code]
links:
  docs: [/features/mcp/#reading-a-servers-resources]
---

The Framework HTTP MCP client now completes resource reads that return
`input_required`. Calls through `McpClient::read_resource` can retry opaque
server state and use the host's configured URL or form elicitation handlers,
subject to the server's existing elicitation policy.

Enable `everruns-core`'s `mcp` feature and read through your configured client:

```rust
use everruns_core::mcp::{McpClient, McpConnection};

async fn read_document(client: &McpClient, server: &McpConnection) -> anyhow::Result<()> {
    let resource = client.read_resource(server, "docs://guide").await?;
    println!("Read {} content blocks", resource.contents.len());
    Ok(())
}
```

Retries preserve the requested URI and sign-in. Consent is scoped to that
resource, and the full read has one deadline across network requests and input
handling. Declining or cancelling a URL request stops the read without retrying.
See [Reading a server's resources](/features/mcp/#reading-a-servers-resources)
for the resource surface.

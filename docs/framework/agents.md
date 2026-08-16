---
title: Agents
description: Describe an agent with instructions, a model, tools, files, integrations, and an optional workspace.
---

An `Agent` is an immutable, validated application description. Pass it to an
application-owned engine to create independent sessions.

```rust
use everruns::{Agent, McpServer, Model};

let agent = Agent::builder()
    .name("researcher")
    .instructions("Research carefully and cite the evidence you used.")
    .model(Model::simulated("No network was used."))
    .file("brief.md", "Investigate the supplied question.")
    .readonly_file("policy.md", "Never expose secrets.")
    .mcp_server(McpServer::http("catalog", "https://example.com/mcp"))
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

Builder validation catches blank instructions, a missing model, duplicate
providers, tools, or capabilities, invalid tool schemas, invalid capability
IDs/configuration, implementation collisions, and invalid MCP configuration
before a session starts. Configure typed built-ins, code-defined packages, and
dynamic references through the single `capability(...)` entrypoint; see
[Configure and author capabilities](/framework/advanced-capabilities/).

## Files and workspaces

- `file(path, content)` seeds an editable file.
- `readonly_file(path, content)` seeds a file the agent may read but not change.
- `workspace(root)` exposes one trusted real-disk root as `/workspace`.

Choose workspace roots from trusted application configuration. Model output and
untrusted request fields must not select executable paths or host directories.
The underlying filesystem boundary rejects traversal and symlink escape. Use a
[`WorkspacePolicy`](/framework/workspace-security/) to configure portable read,
write, hidden-path, and recursive-delete restrictions.

## MCP and plugins

`McpServer::http` adds a remote Streamable HTTP server. Headers may be supplied
by the host and are redacted from `Debug`. Local-process MCP is separately
feature-gated with `mcp-stdio`; its command, arguments, and environment are
trusted host configuration.

`AgentBuilder::plugin(path)` loads a local plugin directory and returns a typed
error if it cannot be compiled. Non-fatal compiler warnings remain visible in
the application-facing session context.

## Inspect effective context

Inspect the next model call before or after a turn:

```rust
# use everruns::Engine;
let engine = Engine::new();
let session = engine.create(agent);
let context = session.inspect().await?;
println!("messages: {}", context.messages.len());
println!("tools: {}", context.tools.len());
# Ok::<(), everruns::RunError>(())
```

Inspection uses the same assembly path as execution, including MCP discovery,
plugin prompt contributions, message filters, and model selection.

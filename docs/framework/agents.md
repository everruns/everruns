---
title: Agents and Tools
description: Describe an agent with instructions, a model, tools, files, integrations, and an optional workspace, and add typed Rust functions as tools.
---

An `Agent` is an immutable, validated application description. Pass it to an
application-owned engine to create independent sessions.

```rust
use everruns::{Agent, McpServer, OpenAI};

let agent = Agent::builder()
    .name("researcher")
    .instructions("Research carefully and cite the evidence you used.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .file("brief.md", "Investigate the supplied question.")
    .readonly_file("policy.md", "Never expose secrets.")
    .mcp_server(McpServer::http("catalog", "https://example.com/mcp"))
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Builder validation catches blank instructions, a missing model, duplicate
providers, tools, or capabilities, invalid tool schemas, invalid capability
IDs/configuration, implementation collisions, and invalid MCP configuration
before a session starts. Configure typed built-ins, code-defined packages, and
dynamic references through the single `capability(...)` entrypoint; see
[Configure and author capabilities](/framework/advanced-capabilities/).

## Harness foundations

Choose `Harness::base()` for zero capabilities, `conversation()` for dialogue,
`worker_base()` for files and bash, or `worker()` for skills, long context,
budgeting and task coordination. These constructors share the hosted platform's
[preset definitions](/features/harnesses/).

```rust
# use everruns::{Agent, Engine, Harness, Model};
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
# let agent = Agent::builder().instructions("Tell dad jokes.").model(Model::simulated("An impasta.")).build()?;
let engine = Engine::new();
let session = engine
    .create(agent)
    .harness(Harness::conversation())
    .start()
    .await?;
let response = session.send_and_wait("Tell me a joke.").await?;
# Ok(())
# }
```

Enable optional host integrations for the tools your application needs. Worker
[delegation requires host backends](/built-ins/harnesses/worker/); the default
in-memory host does not supply them. `Harness::generic()` is deprecated and
preserves its legacy capabilities. Sessions without a bound harness retain their
existing empty foundation.

Presets preserve the framework's read-only workspace default. For an agent that
writes files, explicitly select
`Agent::builder().workspace_policy(WorkspacePolicy::read_write())`.

## Files and workspaces

- `file(path, content)` seeds an editable file.
- `readonly_file(path, content)` seeds a file the agent may read but not change.
- `workspace(root)` exposes one trusted real-disk root as `/workspace`.

Choose workspace roots from trusted application configuration. Model output and
untrusted request fields must not select executable paths or host directories.
The underlying filesystem boundary rejects traversal and symlink escape. Use a
[`WorkspacePolicy`](/framework/workspaces-and-environments/#workspace-security) to configure portable read,
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
# use everruns::{Agent, Engine, Model};
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
# let agent = Agent::builder().instructions("x").model(Model::simulated("ok")).build()?;
let engine = Engine::new();
let session = engine.create(agent);
let context = session.inspect().await?;
println!("messages: {}", context.messages.len());
println!("tools: {}", context.tools.len());
# Ok(())
# }
```

Inspection uses the same assembly path as execution, including MCP discovery,
plugin prompt contributions, message filters, and model selection.

## Tools

The default-enabled `everruns::tool` macro turns an async Rust function into a
typed agent tool. Parameter types produce JSON Schema and call arguments are
deserialized before the function runs.

```rust
use everruns::{Agent, OpenAI};

#[everruns::tool]
/// Add two integers.
async fn add(left: i64, right: i64) -> Result<i64, String> {
    Ok(left + right)
}

let agent = Agent::builder()
    .instructions("Use the add tool for arithmetic.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .tool(add())
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Use `#[everruns::tool(name = "…", description = "…")]` to override metadata,
or `#[tool(rename = "…")]` on a parameter to change its model-facing name.
Functions must be async, non-generic, and have plain named parameters.

The published `everruns-macros` package is an implementation crate. Its source
lives at `crates/macros`, but applications should use the re-exported
`everruns::tool` macro and should not depend on `everruns-macros` directly.

### Dynamic handlers

`FunctionTool::new` is available when a tool schema is determined at runtime:

```rust
use everruns::FunctionTool;
use serde_json::json;

let echo = FunctionTool::new(
    "echo",
    "Return the supplied text.",
    json!({
        "type": "object",
        "properties": { "text": { "type": "string" } },
        "required": ["text"]
    }),
    |args: serde_json::Value| async move {
        Ok::<_, String>(args["text"].clone())
    },
);
# let _ = echo;
```

Prefer the macro for normal typed application tools. Use the dynamic form for
schemas obtained from configuration or another protocol.

### Call context and progress

Make the first parameter a `ToolCallContext` (by value or reference) to learn
which session, turn, and tool call a handler is serving, and to report
progress. Progress arrives on `Session::events()` as
`SessionEventKind::ToolProgress`. The context is not a model argument, so it
never appears in the schema. The dynamic equivalent is
`FunctionTool::with_context`.

```rust
use everruns::ToolCallContext;

/// Export the report.
#[everruns::tool]
async fn export(ctx: &ToolCallContext, pages: u32) -> String {
    ctx.progress(format!("rendering {pages} pages")).await;
    format!("exported for session {}", ctx.session_id())
}
```

### Approval

Mark a tool with `needs_approval` to ask a host approver before it runs.
The bare flag asks on every call. `needs_approval = <rule>` asks only when
the rule returns `true`. The rule receives the generated arguments struct,
named after the function in PascalCase with an `Args` suffix (`run_sql`
becomes `RunSqlArgs`). The approver gets the session id and the tool call,
including its id and arguments, so a host serving many sessions can route
the request. A rejected call never runs, and the model receives a tool error
saying it was rejected. An agent with a gated tool but no `.approver(..)`
fails to build.

```rust
use everruns::approval::{ApprovalDecision, ToolApprover, async_trait};
use everruns::{Agent, Model, SessionId, ToolCall, ToolDefinition};

/// Run one SQL statement.
#[everruns::tool(needs_approval = |args: &RunSqlArgs| args.sql.contains("DROP"))]
async fn run_sql(sql: String) -> String {
    sql
}

struct Console;

#[async_trait]
impl ToolApprover for Console {
    async fn approve(&self, _: SessionId, call: &ToolCall, _: &ToolDefinition) -> ApprovalDecision {
        println!("approve {}? {}", call.name, call.arguments);
        ApprovalDecision::Reject
    }
}

let agent = Agent::builder()
    .instructions("Answer with SQL.")
    .model(Model::simulated("Done."))
    .tool(run_sql())
    .approver(Console)
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`FunctionTool::needs_approval(|args: &Value| ..)` and
`FunctionTool::always_needs_approval()` are the dynamic forms.

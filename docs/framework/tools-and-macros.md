---
title: Tools and Macros
description: Add typed async Rust functions or explicit JSON-schema handlers as Framework tools.
---

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

## Dynamic handlers

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

## Call context and progress

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

## Approval

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

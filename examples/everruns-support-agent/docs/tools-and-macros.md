---
title: Tools and Macros
description: Add typed async Rust functions or explicit JSON-schema handlers as Framework tools.
---

The default-enabled `everruns::tool` macro turns an async Rust function into a
typed agent tool. Parameter types produce JSON Schema and call arguments are
deserialized before the function runs.

```rust
use everruns::{Agent, Model};

#[everruns::tool]
/// Add two integers.
async fn add(left: i64, right: i64) -> Result<i64, String> {
    Ok(left + right)
}

let agent = Agent::builder()
    .instructions("Use the add tool for arithmetic.")
    .model(Model::simulated("The tool is registered."))
    .tool(add())
    .build()?;
# Ok::<(), everruns::BuildError>(())
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

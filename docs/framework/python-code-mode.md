---
title: Python Code Mode in Framework
description: Use Monty as an optional Python execution interface in an Everruns Framework agent, with tools, private persistence and approval resume.
---

Use Python Code Mode when Python is the agent's preferred programmatic tool
interface. Bashkit already provides Python; see [execution options](/framework/execution-options/)
for the differences. Both Framework and Server are platforms supporting these
execution choices.

## Build a Framework agent

Enable the `monty` feature and your provider:

```toml
[dependencies]
everruns = { version = "*", features = ["monty", "openai"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Install the [hardened worker and private journal](/capabilities/python-code-mode/#install-and-operate)
on your execution host. Set `OPENAI_API_KEY`, `EVERRUNS_MONTY_BIN` and
`EVERRUNS_MONTY_STATE_DIR` before starting the application. The journal directory
must be outside the agent's workspace and have mode 0700.

```rust
use everruns::{Agent, Engine, OpenAI, PythonCodeMode};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .provider(OpenAI::from_env()?)
    .model("gpt-6.2-sol")
    .instructions(
        "Prefer Python programs for this task. Discover file tools with tools('file'). \
         Use tool(name, arguments), checking its error field. If approval suspends \
         Python, resume with the returned token instead of rerunning the source."
    )
    .capability(PythonCodeMode)
    .capability("session_file_system")
    .build()?;

let turn = Engine::new().create(agent)
    .run("Calculate the squares of 1 through 12, save the JSON to /workspace/squares.json, and report their sum.")
    .await?;
println!("{}", turn.response);
# Ok(())
# }
```

The model writes the Python. Workspace tools still use the Framework's
configured filesystem. Your typed application tools are available through the
same bridge when their policy permits nested execution. The runnable
[`python_code_mode` example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/python_code_mode.rs)
uses a real model:

```bash
cargo run -p everruns --features monty,openai --example python_code_mode
```

## Compose with Bashkit

Enable both Cargo features and attach both capabilities when the application
needs both interfaces:

```rust
// Cargo: everruns features monty, bashkit, openai.
use everruns::{Agent, BashkitShell, OpenAI, PythonCodeMode};

# fn build() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .provider(OpenAI::from_env()?)
    .model("gpt-6.2-sol")
    .instructions("Prefer Python Code Mode for tool orchestration. Use bash when the requested workflow is a shell script.")
    .capability(PythonCodeMode)
    .capability(BashkitShell::new())
    .build()?;
# let _ = agent;
# Ok(())
# }
```

This does not reroute Bashkit's `python3` command to Monty or enable an automatic
fallback. The `python` tool and the executable named `python` inside `bash`
are different interfaces. Native tool schemas stay available; setting a
preferred interface does not grant new permissions.

## Approval and recovery

Nested tool calls run the active turn's approval and output policies. A hosted
approval pause returns a private-state reference named `python_resume`. After
the decision, the next `python` call supplies `resume` with that token. The
continuation rechecks permissions, reads recorded results for completed effects,
and refuses to repeat any effect whose outcome is unknown.

A persistent Python journal preserves the program, but the host must also
preserve the session and paused turn. `Engine::new()` uses in-memory session
hosting; use a durable hosting profile when recovery must survive application
restart. Custom hosts can inject the neutral
`everruns_contracts::runtime::durable_tools::CodeExecutionStore` service through
host tool-context extensions. The engine binds it to the session and active
input before exposing a journal to the interpreter bridge.

See [Python Code Mode](/capabilities/python-code-mode/) for operational limits,
network and credential behavior, runtime installation, and the exact recovery
boundary. Choose a full [session sandbox](/features/isolated-sandboxes/) when
your program needs packages, native processes or a complete development
workspace.

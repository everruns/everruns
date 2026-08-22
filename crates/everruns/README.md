# everruns

> Build durable, tool-using AI agents in Rust.

[![Crates.io](https://img.shields.io/crates/v/everruns.svg)](https://crates.io/crates/everruns)
[![Documentation](https://docs.rs/everruns/badge.svg)](https://docs.rs/everruns)
[![License](https://img.shields.io/crates/l/everruns.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

The [Everruns Framework](https://everruns.com) gives you the building blocks
for agents that do real work: model providers, typed tools, multi-turn
sessions, live events, cancellation, background work, files, workspaces,
lifecycle hooks, MCP, and durable local state. It runs inside your Rust
process, so you can start with one agent and grow into a custom runtime without
replacing the core programming model.

```text
Agent + Provider + Tools  ->  Engine  ->  Session  ->  Turns and Events
```

## Quick start

Create a project and add Everruns with the OpenAI provider:

```bash
cargo add everruns --features openai
cargo add tokio --features macros,rt-multi-thread
export OPENAI_API_KEY=sk-...
```

Define a typed tool, give it to an agent powered by GPT-5.6 Terra, and run a
turn:

```rust
use std::time::{SystemTime, UNIX_EPOCH};

use everruns::{Agent, Engine, OpenAI};

/// Return the current Unix time in seconds.
#[everruns::tool]
async fn current_time() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| error.to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .name("assistant")
        .instructions("Use current_time when asked about time. Be concise.")
        .provider(OpenAI::from_env()?)
        .model("gpt-5.6-terra")
        .tool(current_time())
        .build()?;

    let session = Engine::new().create(agent);
    let turn = session.send_and_wait("What time is it?").await?;

    println!("{}", turn.response);
    Ok(())
}
```

`#[everruns::tool]` derives the tool's JSON schema and adapter from the Rust
function. The model can call it during the turn, and the result is returned to
the model before the final response is produced.

## The programming model

Everruns keeps the core pieces explicit:

- **`Agent`** describes behavior: instructions, model, provider, tools,
  capabilities, files, and lifecycle hooks.
- **`Engine`** owns runtime resources and the session catalog. Keep it around
  when you want to resume sessions.
- **`Session`** is an isolated, multi-turn conversation. It exposes sending,
  steering, events, cancellation, history, and context inspection.
- **`Turn`** contains the response, status, iteration count, and tool-call
  count for one run.

Agents are immutable values. An engine snapshots an agent when it creates a
session, which makes ownership and isolation predictable even when many
sessions run concurrently.

## Give agents tools and capabilities

For a single operation, annotate an async Rust function with
`#[everruns::tool]` and add it with `.tool(...)`. Inputs are deserialized into
typed parameters, results are serialized for the model, and errors stay
explicit.

Capabilities are the next step when a feature needs several tools, shared
state, metadata, progress events, or call-scoped cancellation. Built-in and
custom capabilities share one builder API:

```rust
use everruns::{Agent, CompactionConfig, ToolSearch};

let agent = Agent::builder()
    .instructions("Find the right tool and keep long sessions focused.")
    .provider(everruns::OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .capability(ToolSearch::automatic())
    .capability(CompactionConfig::new().budget_percent(0.85))
    .build()?;
```

You can also define reusable capability packages in Rust or load open,
configuration-driven capability references. See [Tools and
macros](https://docs.everruns.com/framework/tools-and-macros/), [capability
integrations](https://docs.everruns.com/framework/capability-integrations/),
and [authoring advanced
capabilities](https://docs.everruns.com/framework/advanced-capabilities/).

## Sessions that go beyond request/response

Use `send_and_wait` for a simple turn. Use `send` when you want to subscribe to
events, steer a running agent, cancel work, or wait separately:

```rust
use everruns::{CancellationToken, RunOptions};

let mut events = session.events();
let pending = session.send("Research three options.").await?;

while let Some(event) = events.recv().await? {
    println!("{}", event.event_type());
    if event.kind.is_terminal() {
        break;
    }
}

let turn = pending.wait().await?;
println!("{}", turn.response);

let cancel = CancellationToken::new();
let options = RunOptions::new().cancel_token(cancel.clone());
cancel.cancel();
let stopped = session.run_with("Start another task.", options).await?;
assert!(!stopped.success);
```

The `local` feature adds a durable event log, session resume, scheduled work,
and Git-backed workspace heads. Sessions can bind to isolated mutable project
views and reopen the exact same workspace after a restart.

## Features

The default feature set includes typed tools, capabilities, built-ins, and the
session filesystem. Network providers and heavier runtime integrations are
opt-in.

| Feature | Adds |
| --- | --- |
| `openai` | OpenAI Responses API provider configuration |
| `bashkit` | Sandboxed shell execution |
| `web-fetch` | HTTP content fetching |
| `duckduckgo` | DuckDuckGo search |
| `lua` | Lua execution |
| `mcp` | Remote HTTP MCP servers |
| `mcp-stdio` | Local-process MCP servers, plus HTTP MCP |
| `local` | Durable local sessions, work, schedules, and Git workspace heads |
| `a2a` | Outbound Agent2Agent delegation; includes `local` |

Combine features as needed:

```bash
cargo add everruns --features openai,bashkit,web-fetch,mcp
```

## Examples

Every example imports only `everruns`. The [example catalog](examples/README.md)
includes the exact command for each one.

### Start here

- [`hello`](examples/hello.rs) — a small GPT-5.6 Terra agent with a typed tool
  and live events.
- [`production_agent`](examples/production_agent.rs) — defensive tool
  boundaries and a multi-turn support agent.
- [`engine_sessions`](examples/engine_sessions.rs) — engine ownership,
  isolated sessions, and resume.
- [`live_session`](examples/live_session.rs) — non-blocking sends, steering,
  and waiting.

### Tools, capabilities, and orchestration

- [`capability_configuration`](examples/capability_configuration.rs) — typed,
  code-defined, and dynamic capabilities through one API.
- [`advanced_capability`](examples/advanced_capability.rs) — reusable tools,
  metadata, progress, typed results, and structured errors.
- [`subagents`](examples/subagents.rs) — concurrent child agents coordinated by
  a parent agent.
- [`github_monitor`](examples/github_monitor.rs) — background work that wakes
  an agent when a pull request check completes.
- [`session_work`](examples/session_work.rs) — session-owned tasks, delivery,
  and completion wakes.

### Control, state, and observability

- [`observe_and_cancel`](examples/observe_and_cancel.rs) — event streaming and
  cooperative cancellation.
- [`canonical_events`](examples/canonical_events.rs) — bounded canonical event recording
  and typed rendering.
- [`lifecycle_hooks`](examples/lifecycle_hooks.rs) — awaited agent, turn, tool,
  and completion handlers.
- [`session_history`](examples/session_history.rs) — durable resume and bounded
  history pages.
- [`workspace_policy`](examples/workspace_policy.rs) — portable read/write
  scopes and trusted starter files.
- [`workspace_heads`](examples/workspace_heads.rs) — isolated Git heads,
  environments, and durable workspace binding.

## Documentation

- [Framework guide](https://docs.everruns.com/framework/)
- [Agents](https://docs.everruns.com/framework/agents/)
- [Models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [Sessions](https://docs.everruns.com/framework/sessions/)
- [Events and cancellation](https://docs.everruns.com/framework/events-and-cancellation/)
- [Persistence](https://docs.everruns.com/framework/persistence/)
- [Workspaces and environments](https://docs.everruns.com/framework/workspaces-and-environments/)
- [Custom providers](https://docs.everruns.com/framework/custom-providers/)
- [Custom backends](https://docs.everruns.com/framework/custom-backends/)
- [API reference](https://docs.rs/everruns)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).

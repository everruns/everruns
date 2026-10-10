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

Not every call needs all of that. When the work is one prompt and one answer,
call the model directly — same provider, same credentials, no agent loop:

```rust
use everruns::{Model, OpenAI};

let model = Model::new("gpt-5.6-terra", OpenAI::from_env()?);
let answer = model.complete("Name the three primary colors.").await?;
```

See [Direct model calls](https://docs.everruns.com/framework/direct-model-calls/).

Picking that model id is the same provider, asked a different question:

```rust
use everruns::{OpenAI, models};

for model in models::list(OpenAI::from_env()?).await? {
    println!("{} — {}", model.id(), model.display_name().unwrap_or("?"));
}
```

See [Model catalogs](https://docs.everruns.com/framework/models-and-providers/).

And when the answer is a number rather than prose — *does this hold, how severe
is it, which of these* — ask for a judgment instead of parsing one out of text:

```rust
use everruns::{Decisions, TypeSafeAI};

let judge = Decisions::new("jev-latest", TypeSafeAI::from_env()?);
let spam = judge.probability("Is this message spam?", text).await?;
```

The threshold stays in your code, so there is no written verdict to misparse.
See [Direct decision](https://docs.everruns.com/framework/direct-model-calls/).

## Credentials come from your vendor's own variables

Every driver declares the environment variables its vendor's SDK reads, so a
shell that already works with that vendor already configures the driver:

```rust
use everruns::{Agent, Model, OpenAI};

// OPENAI_API_KEY, and OPENAI_BASE_URL when set.
let openai = OpenAI::from_env()?;

// ANTHROPIC_API_KEY. Every driver module has the same entry point.
let anthropic = everruns::drivers::anthropic::from_env("anthropic")?;

// AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_REGION — a driver is not
// limited to one key.
let bedrock = everruns::drivers::bedrock::from_env("bedrock")?;
```

There is no Everruns naming scheme: the names belong to the drivers. A driver
that declares nothing is never configured from the environment.

This is for standalone, CLI, and development use. Server deployments resolve
credentials from encrypted storage and read no environment variables — drivers
only declare names, they never read them. See
[Credentials](https://docs.everruns.com/framework/models-and-providers/).

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
macros](https://docs.everruns.com/framework/agents/), [capability
integrations](https://docs.everruns.com/framework/capability-integrations/),
and [authoring advanced
capabilities](https://docs.everruns.com/framework/advanced-capabilities/).

## Choose a reusable harness foundation

`Harness::base()`, `Harness::conversation()`, `Harness::worker()` and
`Harness::bashkit_worker()` share the hosted platform's presets. Base carries
context management and tool-call robustness; Conversation adds chat
affordances; Worker adds files, project instructions, skills, long context,
budgeting and delegation without a shell; Bashkit Worker adds the Bashkit
shell. Enable optional host integrations for the tools your application uses.
Presets retain the read-only workspace default; file writes require an explicit
`Agent::builder().workspace_policy(WorkspacePolicy::read_write())`.

Bind and start one with
`engine.create(agent).harness(Harness::conversation()).start().await?`.
`Harness::generic()` is deprecated and preserves its legacy behavior. Sessions
without a bound harness retain their existing empty foundation.

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
| `chatgpt` | Personal ChatGPT plan driver and open-source OAuth helpers through `everruns::drivers::chatgpt` |
| `codex` | Legacy Codex driver through `everruns::drivers::codex` |
| `bedrock` | AWS Bedrock provider configuration: static keys, or the AWS default credential chain for IAM roles |
| `typesafe` | TypeSafe decisions provider and the `jev` capability |
| `bashkit` | Sandboxed shell execution |
| `web-fetch` | HTTP content fetching |
| `duckduckgo` | DuckDuckGo search |
| `lua` | Lua execution |
| `mcp` | Remote HTTP MCP servers |
| `mcp-stdio` | Local-process MCP servers, plus HTTP MCP |
| `local` | Durable local sessions, work, schedules, and Git workspace heads |
| `a2a` | Outbound Agent2Agent delegation; includes `local` |
| `ag-ui` | Serve a session to AG-UI 1.0 clients (CopilotKit, `@ag-ui/client`) with `Session::ag_ui` |
| `ag-ui-axum` | `ag-ui` plus `AgUiHandler`, a ready-made axum route with an authorizer, thread resolution and SSE framing |
| `channel-auth` | `channel_auth`: the everruns server's credential verifier for agent endpoints (OIDC/JWKS, OAuth 2.0 introspection, claim requirements) |

Combine features as needed:

```bash
cargo add everruns --features openai,bashkit,web-fetch,mcp
```

## Examples

Every example imports only `everruns`. The [example catalog](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/README.md)
includes the exact command for each one.

### Start here

- [`hello`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/hello.rs) — a small GPT-5.6 Terra agent with a typed tool
  and live events.
- [`production_agent`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/production_agent.rs) — defensive tool
  boundaries and a multi-turn support agent.
- [`engine_sessions`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/engine_sessions.rs) — engine ownership,
  isolated sessions, and resume.
- [`live_session`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/live_session.rs) — non-blocking sends, steering,
  and waiting.
- [`direct_llm`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/direct_llm.rs) — one-shot, builder, and streamed
  model calls with no agent.

### Tools, capabilities, and orchestration

- [`capability_configuration`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/capability_configuration.rs) — typed,
  code-defined, and dynamic capabilities through one API.
- [`advanced_capability`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/advanced_capability.rs) — reusable tools,
  metadata, progress, typed results, and structured errors.
- [`subagents`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/subagents.rs) — concurrent child agents coordinated by
  a parent agent.
- [`github_monitor`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/github_monitor.rs) — background work that wakes
  an agent when a pull request check completes.
- [`session_work`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/session_work.rs) — session-owned tasks, delivery,
  and completion wakes.

### Control, state, and observability

- [`observe_and_cancel`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/observe_and_cancel.rs) — event streaming and
  cooperative cancellation.
- [`canonical_events`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/canonical_events.rs) — bounded canonical event recording
  and typed rendering.
- [`lifecycle_hooks`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/lifecycle_hooks.rs) — awaited agent, turn, tool,
  and completion handlers.
- [`session_history`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/session_history.rs) — durable resume and bounded
  history pages.
- [`workspace_policy`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/workspace_policy.rs) — portable read/write
  scopes and trusted starter files.
- [`workspace_heads`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/workspace_heads.rs) — isolated Git heads,
  environments, and durable workspace binding.

## Documentation

- [Framework guide](https://docs.everruns.com/framework/)
- [Agents](https://docs.everruns.com/framework/agents/)
- [Models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [Direct model calls](https://docs.everruns.com/framework/direct-model-calls/)
- [Model catalogs](https://docs.everruns.com/framework/models-and-providers/)
- [Direct decision](https://docs.everruns.com/framework/direct-model-calls/)
- [Credentials](https://docs.everruns.com/framework/models-and-providers/)
- [Sessions](https://docs.everruns.com/framework/sessions/)
- [Events and cancellation](https://docs.everruns.com/framework/events-and-cancellation/)
- [Persistence](https://docs.everruns.com/framework/sessions/)
- [Workspaces and environments](https://docs.everruns.com/framework/workspaces-and-environments/)
- [Custom providers](https://docs.everruns.com/framework/models-and-providers/)
- [Custom backends](https://docs.everruns.com/framework/custom-backends/)
- [API reference](https://docs.rs/everruns)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).

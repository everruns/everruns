# everruns

> The application-facing crate for building and running agents with the Everruns Framework.

[![Crates.io](https://img.shields.io/crates/v/everruns.svg)](https://crates.io/crates/everruns)
[![Documentation](https://docs.rs/everruns/badge.svg)](https://docs.rs/everruns)
[![License](https://img.shields.io/crates/l/everruns.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns` provides value-first agents, plain model ids, open provider configuration,
typed tools, isolated multi-turn sessions, live events, cancellation, files,
typed lifecycle hooks, MCP, plugins, and context inspection without requiring
a server, worker, or database.

It is the primary library crate in the [Everruns](https://everruns.com)
ecosystem. Normal Rust applications should start here; focused core, engine,
host, provider, and platform crates support the implementation and advanced
execution hosts.

## Installation

```bash
cargo add everruns
```

Default features stay offline and include the typed tool macro plus the
host-contained session filesystem. Opt into execution/network integrations only
when needed:

```bash
cargo add everruns --features openai
cargo add everruns --features bashkit,web-fetch
cargo add everruns --features lua
cargo add everruns --features mcp        # HTTP MCP
cargo add everruns --features mcp-stdio  # also permits local processes
```

See [Capability integrations](https://docs.everruns.com/framework/capability-integrations/)
for feature boundaries and advanced-host composition.

## Offline Quickstart

```rust
use everruns::{Agent, Model};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("Answer in one short sentence.")
        .model(Model::simulated("Hello from Everruns."))
        .build()?;

    let turn = agent.session().send_and_wait("Say hello.").await?;
    assert_eq!(turn.response, "Hello from Everruns.");
    Ok(())
}
```

`Model::simulated` uses the normal provider/execution path and returns a fixed
response locally. It needs no credential or network connection.

## Open Provider Setup

With the `openai` feature, attach the provider while keeping model identity free
of credentials:

```rust
use everruns::{Agent, OpenAI};

let agent = Agent::builder()
    .instructions("You are concise.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Custom services use the same open boundary: attach a `Provider` backed by your
`ChatDriver`, then select its model with a plain string id. No `ModelSpec`,
closed model enum, or provider-specific application branch is required.

## Typed Tools

```rust
use everruns::{Agent, Model};

#[everruns::tool]
/// Add two integers.
async fn add(left: i64, right: i64) -> Result<i64, String> {
    Ok(left + right)
}

let agent = Agent::builder()
    .instructions("Use the add tool for arithmetic.")
    .model(Model::simulated("Tool registered."))
    .tool(add())
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

The default-enabled macro generates the argument schema and adapter. The
`everruns-macros` package is an implementation crate re-exported as
`everruns::tool`; applications do not need to depend on it directly.

## Unified Capability Configuration

Every capability uses one scalable builder entrypoint. Typed built-ins,
code-defined packages, open third-party values, plain default-config IDs, and
dynamic JSON references all implement `IntoCapability`:

```rust
use everruns::{
    Agent, CapabilityRef, CompactionConfig, Model, ToolSearch, capability,
};
use serde_json::json;

let weather_definition = capability::Definition::new(
    "weather",
    "Weather",
    "Application-defined weather tools.",
).tool(weather_handler);

let agent = Agent::builder()
    .instructions("Use configured capabilities when relevant.")
    .model(Model::simulated("Done."))
    .capability(CompactionConfig::new().budget_percent(0.85))
    .capability(ToolSearch::automatic())
    .capability(weather_definition)
    .capability(
        CapabilityRef::new("vendor.custom")
            .config(json!({ "mode": "database-driven" })),
    )
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

`CapabilityRef` is the explicit database/plugin escape hatch: the Framework
validates its stable open ID and JSON object at build time, and known built-ins
validate their own schemas. Duplicate IDs and code-implementation collisions
are errors, never silent overwrites. Third-party crates can implement the
non-sealed `IntoCapability` trait without importing `everruns-core`.

Keep ordinary functions on `#[everruns::tool]` and `.tool(...)`; a function
tool is not a capability reference.

The default `builtins` feature supplies the backend-neutral policy catalog and
typed `CompactionConfig` and `ToolSearch` values through `everruns-builtins`.
Build with `default-features = false` when supplying a completely custom
registry; that keeps the policy implementation bundle out of the dependency
graph.

## Sessions, Events, and Cancellation

An agent opens independent live sessions. `send` accepts a message immediately,
automatically steering an active turn or starting the next turn after
completion. `send_and_wait` is the request/response convenience. Subscribe
before sending to observe live events, or pass a cancellation token through
`RunOptions`.

```rust
use everruns::{CancellationToken, RunOptions};

let session = agent.session();
let mut events = session.events();
let first = session.send("Remember this turn.").await?;

let turn = first.wait().await?;

let cancel = CancellationToken::new();
let options = RunOptions::new().cancel_token(cancel.clone());
cancel.cancel();
let stopped = session.run_with("Do not start.", options).await?;

assert!(turn.success);
assert!(!stopped.success);
while let Some(event) = events.try_recv()? {
    println!("{}", event.event_type());
}
# Ok::<(), everruns::RunError>(())
```

`Session::inspect` exposes the application-facing context assembled for the
next model call without exposing backend records.

## Lifecycle Hooks

Register async handlers on `Agent::builder()` when application work must be
awaited at an agent, turn, tool, or completion boundary. Handlers receive owned,
typed Framework contexts and never require persisted hook records or runtime
imports. Use session events instead for non-blocking observation.

```rust
use everruns::{Agent, Model};

let agent = Agent::builder()
    .instructions("You are concise.")
    .model(Model::simulated("Ready."))
    .on_agent_start(|context| async move {
        println!("started {}", context.session_id);
    })
    .on_completion(|context| async move {
        println!("completed {}", context.turn.turn_id);
    })
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

## Persistence

By default, conversation history is offline, database-free, and retained for
the lifetime of an `Agent` and its clones. Keep a typed `SessionId` and call
`Agent::resume`; use `Session::history().page()` for bounded event-derived
reads. The `local` feature adds a crash-durable canonical event log and session
catalog alongside its real workspace and task/schedule state.

## What It Provides

- Value-first `Agent`, plain model ids, open `Provider`, and deterministic simulation
- Typed and dynamic function tools
- Independent multi-turn `Session`s, typed resume, bounded history, and next-turn context inspection
- Live typed events, lossless canonical envelopes, and cancellation
- Session-owned immediate and scheduled work with leased, at-least-once delivery
- Awaited, typed lifecycle hooks with explicit failure isolation
- Editable/read-only files, one trusted workspace, scoped MCP, and plugins
- Optional OpenAI and local profiles without enlarging the offline default

## Runnable Examples

The [example catalog](./examples/README.md) includes:

- `workspace_policy` — secure workspace scopes with an offline simulator
- `hello` — smallest live-provider program
- `production_agent` — production-style composition
- `github_monitor --simulate` — credential-free typed-tool flow
- `session_work` — offline background work and completion wakes
- `canonical_events` — offline lossless event recording and typed rendering
- `subagents` — public-facade delegation
- `observe_and_cancel` — events and cancellation
- `session_history` — offline durable resume and bounded history pages
- `lifecycle_hooks` — agent, turn, tool, and completion handlers

Examples are compiled in CI and import only `everruns`.

## Which Crate Should I Use?

| Need | Start with |
| --- | --- |
| Build and run agents in a Rust application | `everruns` |
| Implement or configure a focused model provider | `everruns` plus the provider crate |
| Call a remote Everruns deployment | an Everruns SDK |
| Compose low-level execution backends | `everruns` plus `everruns-host` and focused sibling crates |
| Operate durable server/worker/UI infrastructure | the Everruns Platform |

## Documentation

- [Everruns Framework](https://docs.everruns.com/framework/)
- [Framework quickstart](https://docs.everruns.com/framework/quickstart/)
- [Workspace security](https://docs.everruns.com/framework/workspace-security/)
- [Models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [Tools and macros](https://docs.everruns.com/framework/tools-and-macros/)
- [Sessions](https://docs.everruns.com/framework/sessions/)
- [Session history and resume](https://docs.everruns.com/framework/session-history/)
- [Persistence](https://docs.everruns.com/framework/persistence/)
- [Session work and wakes](https://docs.everruns.com/framework/background-work/)
- [Events and cancellation](https://docs.everruns.com/framework/events-and-cancellation/)
- [Lifecycle hooks](https://docs.everruns.com/framework/lifecycle-hooks/)
- [Canonical events](https://docs.everruns.com/framework/canonical-events/)
- [API reference](https://docs.rs/everruns)

## Extend agents

Use `#[everruns::tool]` for an ordinary typed async function. Reusable packages
that need multiple typed tools, capability metadata, execution context,
progress, or call-scoped cancellation use the curated `everruns::capability`
SPI and the same `AgentBuilder::capability` entrypoint.

See the [Framework capability-authoring guide](../../docs/framework/advanced-capabilities.md)
and the [runnable advanced example](examples/advanced_capability.rs).

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).

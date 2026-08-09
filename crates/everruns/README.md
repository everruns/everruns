# everruns

> The application-facing crate for building and running agents with the Everruns Framework.

[![Crates.io](https://img.shields.io/crates/v/everruns.svg)](https://crates.io/crates/everruns)
[![Documentation](https://docs.rs/everruns/badge.svg)](https://docs.rs/everruns)
[![License](https://img.shields.io/crates/l/everruns.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns` provides value-first agents, open model/provider configuration,
typed tools, isolated multi-turn sessions, live events, cancellation, files,
MCP, plugins, and context inspection without requiring a server, worker, or
database.

It is the primary library crate in the [Everruns](https://everruns.com)
ecosystem. Normal Rust applications should start here; focused core, engine,
runtime, provider, and platform crates support the implementation and advanced
execution hosts.

## Installation

```bash
cargo add everruns
```

Default features stay offline and include the typed tool macro. Opt into a live
provider only when needed:

```bash
cargo add everruns --features openai
```

## Offline Quickstart

```rust
use everruns::{Agent, Model};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("Answer in one short sentence.")
        .model(Model::simulated("Hello from Everruns."))
        .build()?;

    let turn = agent.session().run("Say hello.").await?;
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
    .model(OpenAI::from_env("gpt-5.6-terra")?)
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Custom services use the same open boundary: pair a credential-free `ModelSpec`
with a `Provider` backed by your `ChatDriver`. No closed model enum or
provider-specific application branch is required.

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

## Sessions, Events, and Cancellation

An agent opens independent sessions. Reuse one session for multi-turn history;
open another for isolation. Subscribe before a turn to observe its live event
projection, or pass a cancellation token through `RunOptions`.

```rust
use everruns::{CancellationToken, RunOptions};

let mut session = agent.session();
let mut events = session.events();
let first = session.run("Remember this turn.").await?;

let cancel = CancellationToken::new();
let options = RunOptions::new().cancel_token(cancel.clone());
cancel.cancel();
let stopped = session.run_with("Do not start.", options).await?;

assert!(first.success);
assert!(!stopped.success);
while let Some(event) = events.try_recv() {
    println!("{}", event.event_type());
}
# Ok::<(), everruns::RunError>(())
```

`Session::inspect` exposes the application-facing context assembled for the
next model call without exposing backend records.

## Persistence

Conversation history is in memory for the lifetime of a `Session`. The `local`
feature supplies a real-disk workspace plus local task/schedule state; it does
not imply durable conversation history. Writable JSONL message-store APIs remain
available only for existing 0.17.x compatibility and are not the persistence
model for new Framework applications.

## What It Provides

- Value-first `Agent`, open `ModelSpec`/`Provider`, and deterministic simulation
- Typed and dynamic function tools
- Independent multi-turn `Session`s and next-turn context inspection
- Live typed events, lossless unknown-event projection, and cancellation
- Editable/read-only files, one trusted workspace, scoped MCP, and plugins
- Optional OpenAI and local profiles without enlarging the offline default

## Runnable Examples

The [example catalog](./examples/README.md) includes:

- `hello` — smallest live-provider program
- `production_agent` — production-style composition
- `github_monitor --simulate` — credential-free typed-tool flow
- `subagents` — public-facade delegation
- `observe_and_cancel` — events and cancellation

Examples are compiled in CI and import only `everruns`.

## Which Crate Should I Use?

| Need | Start with |
| --- | --- |
| Build and run agents in a Rust application | `everruns` |
| Implement or configure a focused model provider | `everruns` plus the provider crate |
| Call a remote Everruns deployment | an Everruns SDK |
| Compose low-level execution backends or preserve 0.17.x code | `everruns-runtime` and focused host crates |
| Operate durable server/worker/UI infrastructure | the Everruns Platform |

## Documentation

- [Everruns Framework](https://docs.everruns.com/framework/)
- [Framework quickstart](https://docs.everruns.com/framework/quickstart/)
- [Models and providers](https://docs.everruns.com/framework/models-and-providers/)
- [Tools and macros](https://docs.everruns.com/framework/tools-and-macros/)
- [Sessions](https://docs.everruns.com/framework/sessions/)
- [Events and cancellation](https://docs.everruns.com/framework/events-and-cancellation/)
- [API reference](https://docs.rs/everruns)

## Extend agents

Use `#[everruns::tool]` for an ordinary typed async function. Reusable packages
that need multiple typed tools, capability metadata, execution context,
progress, or call-scoped cancellation use the curated `everruns::capability`
SPI and `AgentBuilder::advanced_capability`.

See the [Framework capability-authoring guide](../../docs/framework/advanced-capabilities.md)
and the [runnable advanced example](examples/advanced_capability.rs).

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).

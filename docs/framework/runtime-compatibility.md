---
title: Runtime Migration
description: Migrate existing everruns-runtime 0.17.x applications to the Everruns Framework or focused host crates.
---

> **`everruns-runtime` is a compatibility-only 0.17.x crate.** It remains
> published, usable, and supported throughout 0.17.x, and will be removed in
> Everruns 0.18. New applications should not add it.

Choose one of two replacement paths:

- **Ordinary applications:** depend on `everruns` and use the Framework's
  `Agent`, `Model`, and `Session` APIs.
- **Advanced system integrators:** depend on `everruns` plus `everruns-host`
  and only the focused siblings they need, such as `everruns-mcp` or provider
  and integration crates.

Success means there are no `everruns-runtime` dependencies or
`everruns_runtime` imports. A complete coding-agent host does not need to force
all focused host, MCP, provider, or integration APIs through one crate.

## Install the replacement

For an ordinary application:

```bash
cargo remove everruns-runtime
cargo add everruns
```

The default `everruns` build stays offline and has no database, server, worker,
network, or credential requirement. Provider and local-process integrations
remain opt-in features.

For an advanced host:

```bash
cargo remove everruns-runtime
cargo add everruns everruns-host
```

Then add only focused crates your host owns. Use application values from
`everruns` and host implementation contracts from `everruns-host`:

```rust
use everruns::{Agent, Model};
use everruns_host::{HostBackends, InProcessRuntimeBuilder};
# let _ = (Agent::builder, Model::simulated);
# let _ = (HostBackends::in_memory, InProcessRuntimeBuilder::new);
```

## Exact migration map

| Existing 0.17 runtime use | Ordinary Framework replacement | Advanced-host replacement or boundary |
| --- | --- | --- |
| `InProcessRuntimeBuilder::single_session`, `HarnessBuilder`, runtime `AgentBuilder`, and `SessionBuilder` | `everruns::Agent::builder()`, then `Agent::session()`; reuse the live `Session`, call `send` to start or steer without waiting, and use the receipt's `wait` method or `send_and_wait`/`run` for request-response turns | Import `InProcessRuntimeBuilder`, `HarnessBuilder`, `AgentBuilder`, and `SessionBuilder` from `everruns-host` when the host truly owns stored topology |
| `llm_sim`, `default_model(ResolvedModel)`, `model_spec`, `provider`, and `DriverRegistry` setup | Use `Model::simulated` or `Model::simulated_with_config` offline; for a live model, call `AgentBuilder::provider(Provider)` and select its provider-visible model with `AgentBuilder::model("model-id")` | Keep provider values and model ids at the Framework boundary; use focused provider crates and host builder registration only for host-owned resolution |
| Capability-registry setup for application tools | `#[everruns::tool]`, `FunctionTool`, or the curated `everruns::capability` authoring API | Keep platform capability registries in focused host/core composition when the integration owns the platform |
| `run_text_turn` plus `EventBus`/`EventEmitter` observation | `Session::send` plus its receipt/turn handle, `Session::events`, typed lifecycle hooks, and `Session::run_with(RunOptions)` plus `CancellationToken`; `send_and_wait` and `run` are waiting conveniences | Use `everruns_host::EventSink` for post-commit observation and `EventReader` for replay; the legacy `EventBus` shim is not an execution input |
| Runtime message/session persistence and JSONL history | Keep the typed `SessionId`, call `Agent::resume`, and traverse bounded event-derived pages with `Session::history`; the default is in-memory for the `Agent` lifetime, while `local` adds crash-durable canonical events and a session catalog | Use canonical `EventLog`/`EventHistory` from `everruns-host`; `JsonlEventLog` is a host event log, and messages/context are projections |
| `RuntimeBackends` and individual store factories | No application replacement: Framework agents use safe defaults | Rename to `everruns_host::HostBackends` and import the store/factory traits directly from `everruns-host` or their focused sibling crate |
| `RealDiskFileStore`, `RealDiskSessionFileSystemFactory`, mount, or multi-root plumbing | `AgentBuilder::workspace`, `file`, `readonly_file`, and `WorkspacePolicy` | Import filesystem implementations from `everruns-host`; preserve workspace containment and policy enforcement when composing them |
| Runtime MCP wiring and `with_plugin_dir` | `AgentBuilder::mcp_server(McpServer)` and `AgentBuilder::plugin`; enable `mcp-stdio` only for trusted local-process servers | Combine `everruns-host` with `everruns-mcp` for host-owned transport/auth/discovery topology |
| `load_context(session_id)` and raw assembled context inspection | `Session::inspect()` returns curated `SessionContext` with model, messages, tools, instructions, locale, and plugin warnings | Retain the low-level host context path only when an execution host needs internal records or phase inputs |
| `everruns-local` backend assembly, task registries, and schedule stores | Enable `everruns/local`, configure `LocalConfig`, and use `everruns::work` for application tasks, wakes, and schedules | Combine `everruns-host` and `everruns-local` when the integrator owns route claims, schedule delivery, restart, or runner lifecycle |
| Low-level host phases, durable turn state, custom platform runners, or worker adapters | No application replacement; these are deliberately below the Framework | Import the same neutral symbols from `everruns-host`, plus `everruns-engine`, `everruns-mcp`, provider, platform, or integration crates as required |

The `RuntimeBackends` name is the one intentional rename:

```rust
// Existing 0.17 compatibility path:
use everruns_runtime::{InProcessRuntimeBuilder, RuntimeBackends};

// Maintained advanced-host path:
use everruns_host::{HostBackends, InProcessRuntimeBuilder};
# let _ = RuntimeBackends::in_memory;
# let _ = HostBackends::in_memory;
# let _ = InProcessRuntimeBuilder::new;
```

Legacy mutable-history paths have no maintained write-path equivalent.
`EventBus` maps to separate post-commit observation and replay contracts, not
to another conversation store. Canonical events are authoritative.

## Representative application migration

An existing 0.17 application may keep running while it migrates:

```rust
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_runtime::InProcessRuntimeBuilder;

# async fn legacy() -> Result<(), Box<dyn std::error::Error>> {
let runtime = InProcessRuntimeBuilder::new()
    .llm_sim(LlmSimConfig::fixed("4"))
    .single_session(|session| {
        session
            .harness("math", "Answer arithmetic questions.")
            .agent("math-agent", "Return only the answer.")
    })
    .build()
    .await?;
let turn = runtime
    .run_text_turn(runtime.default_session_id().unwrap(), "What is 2 + 2?")
    .await?;
assert_eq!(turn.response, "4");
# Ok(())
# }
```

The ordinary Framework version is smaller and preserves the same canonical
behavior:

```rust
use everruns::{Agent, Model};

# async fn framework() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Return only the answer.")
    .model(Model::simulated("4"))
    .build()?;
let turn = agent.session().run("What is 2 + 2?").await?;
assert_eq!(turn.response, "4");
# Ok(())
# }
```

The repository tests these programs as real external consumers. Both run under
`-D warnings`; the Framework fixture also proves it has no runtime dependency
or import.

## Why the crate has no blanket compiler deprecation

The transition is explicit in the package description, README, docs.rs landing
page, and this guide. A blanket Rust `deprecated` attribute was evaluated in an
external consumer and rejected: deprecation warnings become hard errors for
ordinary applications that compile with `-D warnings`, breaking the promised
0.17 source-compatible bridge.

The already-isolated, non-authoritative legacy shims retain narrow symbol-level
warnings with exact replacements. Common compatibility imports and host
re-exports do not emit a compiler warning.

## Remaining 0.17 compatibility surface

`everruns-runtime` re-exports the canonical `everruns-host` implementation,
keeps the `RuntimeBackends` alias, retains the two isolated legacy shims, and
forwards its supported `lua` and `mcp-stdio` features. It owns no execution
algorithm or backend composition. It remains publishable after `everruns-host`
and before `everruns` in the 0.17.x publish order; it is not removed or yanked.

---
title: Everruns Framework
description: Build and run agents inside a Rust application with the application-facing everruns crate.
---

The **Everruns Framework** is the application-facing [`everruns`](https://docs.rs/everruns)
crate. Use it to describe agents, attach models and tools, run multi-turn sessions,
observe events, and embed agent execution directly in a Rust process.

```rust
use everruns::{Agent, Engine, OpenAI};

let agent = Agent::builder()
    .instructions("Answer in one short sentence.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .build()?;

let engine = Engine::new();
let turn = engine.create(agent).send_and_wait("Say hello.").await?;
println!("{}", turn.response);
# Ok::<(), Box<dyn std::error::Error>>(())
```

No database, server or worker is required — an agent runs inside your process.
A model provider is: pick one from
[Supported providers](/framework/supported-providers/), or use the
[test simulator](/framework/testing-and-simulation/) when writing tests.

## Choose the right surface

| Surface | Use it for |
| --- | --- |
| **Framework** | Rust applications that build and run agents in process through `everruns` |
| **Advanced host crates** | Low-level execution-host composition through `everruns-host` and focused siblings |
| **SDKs** | Remote clients that call a running Everruns server |
| **Platform** | The control plane, server, workers, UI, and durable deployment |

Normal library users should start with the Framework. Hosts that must replace
storage or orchestration cross into [custom backends](/framework/custom-backends/).

## Start here

- [Quickstart](/framework/quickstart/), install the crate and run an offline agent.
- [Architecture](/framework/architecture/), understand Agent, Engine, Session, and the shared immediate/durable execution kernel.
- [Agents](/framework/agents/), instructions, files, workspaces, MCP, plugins, and context inspection.
- [Workspace security](/framework/workspace-security/), configure portable read and write scopes with secure defaults.
- [Workspaces and Environments](/framework/workspaces-and-environments/), bind sessions to isolated or explicitly shared backend-owned heads.
- [Models and providers](/framework/models-and-providers/), the model/provider split and the open provider boundary.
- [Supported providers](/framework/supported-providers/), every driver that ships today and what each one supports.
- [Direct model calls](/framework/direct-model-calls/), one prompt and one answer without an agent.
- [Direct decision](/framework/direct-decision/), a calibrated number rather than prose, without an agent.
- [Model catalogs](/framework/model-catalogs/), ask a provider which models it offers and what each supports.
- [Credentials](/framework/credentials/), each driver's own vendor-standard environment variables.
- [Tools and macros](/framework/tools-and-macros/), typed function tools through `everruns::tool`.
- [Sessions](/framework/sessions/), independent, multi-turn conversations.
- [Session work and wakes](/framework/background-work/), immediate and scheduled work with explicit delivery and restart semantics.
- [Session History and Resume](/framework/session-history/), bounded transcript pages and typed continuation.
- [Events and cancellation](/framework/events-and-cancellation/), observe a live turn and stop work cooperatively.
- [Lifecycle hooks](/framework/lifecycle-hooks/), run awaited application behavior at execution boundaries.
- [Canonical events](/framework/canonical-events/), render or record bounded canonical event envelopes.
- [Persistence](/framework/persistence/), Engine-lifetime memory and crash-durable local state.

## Extend and operate

- [Custom providers](/framework/custom-providers/), attach a custom `ChatDriver` without changing a closed enum.
- [Capabilities](/framework/advanced-capabilities/), configure the optional standard policy bundle and open references, or package typed tools with stable metadata and lifecycle context.
- [Capability integrations](/framework/capability-integrations/), opt into filesystem, shell, web, Lua, and MCP implementation boundaries.
- [Portable and hosted capabilities](/framework/capability-boundaries/), understand the Framework/Platform implementation boundary.
- [Custom backends](/framework/custom-backends/), cross into low-level host composition deliberately.
- [Testing and simulation](/framework/testing-and-simulation/), deterministic tests without credentials.
- [Runnable examples](/framework/examples/), complete programs maintained with the crate.

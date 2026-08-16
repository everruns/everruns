---
title: Everruns Framework
description: Build and run agents inside a Rust application with the application-facing everruns crate.
---

The **Everruns Framework** is the application-facing [`everruns`](https://docs.rs/everruns)
crate. Use it to describe agents, attach models and tools, run multi-turn sessions,
observe events, and embed agent execution directly in a Rust process.

```rust
use everruns::{Agent, Engine, Model};

let agent = Agent::builder()
    .instructions("Answer in one short sentence.")
    .model(Model::simulated("Hello from Everruns."))
    .build()?;

let engine = Engine::new();
let turn = engine.create(agent).send_and_wait("Say hello.").await?;
assert_eq!(turn.response, "Hello from Everruns.");
# Ok::<(), Box<dyn std::error::Error>>(())
```

The default build runs this example offline. A database, server, worker, network
connection, and provider credential are not required.

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

- [Quickstart](/framework/quickstart/) — install the crate and run an offline agent.
- [Architecture](/framework/architecture/) — understand Agent, Engine, Session, and the shared immediate/durable execution kernel.
- [Agents](/framework/agents/) — instructions, files, workspaces, MCP, plugins, and context inspection.
- [Workspace security](/framework/workspace-security/) — configure portable read and write scopes with secure defaults.
- [Workspaces and Environments](/framework/workspaces-and-environments/) — bind sessions to isolated or explicitly shared provider-owned heads.
- [Models and providers](/framework/models-and-providers/) — simulation, OpenAI, and the open provider boundary.
- [Tools and macros](/framework/tools-and-macros/) — typed function tools through `everruns::tool`.
- [Sessions](/framework/sessions/) — independent, multi-turn conversations.
- [Session work and wakes](/framework/background-work/) — immediate and scheduled work with explicit delivery and restart semantics.
- [Session History and Resume](/framework/session-history/) — bounded transcript pages and typed continuation.
- [Events and cancellation](/framework/events-and-cancellation/) — observe a live turn and stop work cooperatively.
- [Lifecycle hooks](/framework/lifecycle-hooks/) — run awaited application behavior at execution boundaries.
- [Canonical events](/framework/canonical-events/) — render or record the lossless event protocol.
- [Persistence](/framework/persistence/) — Engine-lifetime memory and crash-durable local state.

## Extend and operate

- [Custom providers](/framework/custom-providers/) — attach a custom `ChatDriver` without changing a closed enum.
- [Capabilities](/framework/advanced-capabilities/) — configure the optional standard policy bundle and open references, or package typed tools with stable metadata and lifecycle context.
- [Capability integrations](/framework/capability-integrations/) — opt into filesystem, shell, web, Lua, and MCP implementation boundaries.
- [Portable and hosted capabilities](/framework/capability-boundaries/) — understand the Framework/Platform implementation boundary.
- [Custom backends](/framework/custom-backends/) — cross into low-level host composition deliberately.
- [Testing and simulation](/framework/testing-and-simulation/) — deterministic tests without credentials.
- [Runnable examples](/framework/examples/) — complete programs maintained with the crate.

---
title: Everruns Framework
description: Build and run agents inside a Rust application with the application-facing everruns crate.
---

The **Everruns Framework** is the application-facing [`everruns`](https://docs.rs/everruns)
crate. Use it to describe agents, attach models and tools, run multi-turn sessions,
observe events, and embed agent execution directly in a Rust process.

```rust
use everruns::{Agent, Engine, OpenAI};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Answer in one short sentence.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .build()?;

let engine = Engine::new();
let turn = engine.create(agent).send_and_wait("Say hello.").await?;
println!("{}", turn.response);
# Ok(())
# }
```

No database, server or worker is required: an agent runs inside your process.
A model provider is: pick one from
[Supported providers](/framework/models-and-providers/#supported-providers), or use the
[test simulator](/framework/testing-and-simulation/) when writing tests.

## Choose the right surface

| Surface | Use it for |
| --- | --- |
| **Framework** | Rust applications that build and run agents in process through `everruns` |
| **Advanced host crates** | Low-level execution-host composition through `everruns-core` (`host` feature) and focused siblings |
| **SDKs** | Remote clients that call a running Everruns server |
| **Platform (self-hosted)** | The control plane, server, workers, UI, and durable deployment you run yourself, for example with [Docker Compose](/getting-started/docker-compose/) |
| **Everruns Cloud** | The Platform operated for you at [app.everruns.com](https://app.everruns.com), with a built-in model provider and a starter credit; bring-your-own keys are optional. See the [Everruns Cloud quickstart](/getting-started/cloud/) |

Normal library users should start with the Framework. Hosts that must replace
storage or orchestration cross into [custom backends](/framework/custom-backends/).

## Start here

- [Quickstart](/framework/quickstart/), install the crate and run one turn against a live model provider.
- [Architecture](/framework/architecture/), understand Agent, Engine, Session, and the shared immediate/durable execution kernel.
- [Upgrade notes](/framework/upgrade-notes/), the code changes each breaking release needs.

## Core APIs

- [Agents and tools](/framework/agents/), instructions, files, workspaces, MCP, plugins, context inspection, and typed function tools through `everruns::tool`.
- [Models and providers](/framework/models-and-providers/), the model/provider split, every driver that ships today, each driver's environment variables, provider model catalogs, and custom providers.
- [Direct calls and decisions](/framework/direct-model-calls/), one prompt and one answer without an agent, or a calibrated number rather than prose.
- [Sessions](/framework/sessions/), independent multi-turn conversations, bounded history, typed resume, and engine-lifetime or crash-durable persistence.
- [Events and cancellation](/framework/events-and-cancellation/), observe a live turn, stop work cooperatively, and record bounded canonical event envelopes.
- [Observability](/framework/observability/), export every session to OpenTelemetry or Braintrust, or register your own event listeners.
- [Workspaces and environments](/framework/workspaces-and-environments/), bind sessions to isolated or shared heads and configure read and write scopes with secure defaults.
- [Lifecycle hooks](/framework/lifecycle-hooks/), run awaited application behavior at execution boundaries.
- [Answer agent questions](/framework/ask-user/), implement `AskUser` so your application answers the agent's structured questions.
- [Session work and wakes](/framework/background-work/), immediate and scheduled work with explicit delivery and restart semantics.

## Extend

- [Capabilities](/framework/advanced-capabilities/), configure the optional standard policy bundle and open references, or package typed tools with stable metadata and lifecycle context.
- [Capability integrations](/framework/capability-integrations/), opt into filesystem, shell, web, Lua, and MCP implementation boundaries.
- [Portable and hosted capabilities](/framework/capability-boundaries/), understand the Framework/Platform implementation boundary.
- [Agent blueprints](/framework/agent-blueprints/), contribute a code-defined specialist agent from a capability.
- [Custom backends](/framework/custom-backends/), cross into low-level host composition deliberately.
- [Testing and simulation](/framework/testing-and-simulation/), deterministic tests without credentials.
- [Runnable examples](/framework/examples/), complete programs maintained with the crate.

## Expose and deploy

- [Deploy a Framework app](/framework/deployment/), ship the binary or a container with a persistent data directory and environment-provided keys.
- [Move to Platform or Cloud](/framework/moving-to-platform/), what carries over to a hosted agent and what has to be rebuilt.
- [Serve](/framework/serve/) (experimental), attribute macros, file-layout discovery and a manifest, served over the Everruns server `/v1` API.
- [Serve AG-UI](/framework/ag-ui/), stream a session to CopilotKit or any AG-UI 1.0 client from your own HTTP server.
- [A2A](/framework/a2a/), serve Framework agents to other agents over A2A 1.0, and delegate work to remote A2A agents.
- [Serve on AgentCore](/framework/serve-agentcore/) (experimental), deploy a serve app to Amazon Bedrock AgentCore Runtime.
- [Serve on celld](/framework/serve-celld/) (experimental), run a serve app durably on celld, self-hosted Durable Objects.

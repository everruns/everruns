# Everruns

[![Website](https://img.shields.io/badge/Website-everruns.com-blue)](https://everruns.com)
[![Docs](https://img.shields.io/badge/Docs-docs.everruns.com-green)](https://docs.everruns.com)
[![CI](https://github.com/everruns/everruns/actions/workflows/ci.yml/badge.svg)](https://github.com/everruns/everruns/actions/workflows/ci.yml)
[![Repo: Agent Friendly](https://img.shields.io/badge/Repo-Agent%20Friendly-blue)](AGENTS.md)

> **Note:** Everruns is under active development. Expect rapid changes and experimental features.

**Build capable AI agents in Rust. Run them where they belong.**

Everruns is an open-source framework for building AI agents directly in Rust
applications. Define agents, attach models and typed tools, run multi-turn
sessions, and observe execution through one application-facing API.

Use the framework in your application. When you need a shared runtime and
production operations, run the Everruns platform yourself or use
[Hosted Everruns](https://app.everruns.com).

[Build with the framework](https://docs.everruns.com/framework/quickstart/) · [Read the docs](https://docs.everruns.com/framework/) · [Use Hosted Everruns](https://app.everruns.com)

## Build an agent

Add the application-facing crate:

```bash
cargo add everruns --features openai
cargo add tokio --features macros,rt-multi-thread
export OPENAI_API_KEY=sk-...
```

Then define a typed tool, give it to an agent, and run a turn:

```rust
use std::time::{SystemTime, UNIX_EPOCH};

use everruns::{Agent, Engine, OpenAI};

#[everruns::tool]
async fn current_time() -> Result<String, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    Ok(format!("{seconds} seconds since the Unix epoch"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .name("assistant")
        .instructions("Use current_time when asked about time. Be concise.")
        .provider(OpenAI::from_env()?)
        .model("gpt-5-mini")
        .tool(current_time())
        .build()?;

    let session = Engine::new().create(agent);
    let turn = session.send_and_wait("What time is it?").await?;
    println!("{}", turn.response);
    Ok(())
}
```

Everruns derives the tool schema from the Rust function, lets the model call it during the turn, and returns the result before producing the final response. [Continue the framework quickstart.](https://docs.everruns.com/framework/quickstart/)

## The agent framework

The [`everruns`](./crates/everruns) crate keeps the application-facing agent
loop explicit and embeddable:

- **Agents and sessions** — describe an agent once, then create independent,
  multi-turn conversations through an `Engine`.
- **Models and providers** — start offline, use OpenAI, or attach a custom
  provider without coupling application code to a closed provider enum.
- **Typed tools and capabilities** — give agents function tools and opt into
  filesystem, shell, web, Lua, and MCP boundaries deliberately.
- **Events, cancellation, and lifecycle hooks** — observe a live turn, add
  application behavior at execution boundaries, and stop work cooperatively.
- **Persistence and execution choices** — begin with engine-lifetime memory,
  add local crash-durable state, or cross deliberately into a distributed host.

[Framework overview](https://docs.everruns.com/framework/)

## Choose how you run Everruns

### Use the framework in your application

Start here when Everruns belongs inside the Rust application you are building.
You own the process, deployment, integrations, and data path.

[Framework quickstart →](https://docs.everruns.com/framework/quickstart/)

### Run the platform yourself

Use the open-source platform when you need a shared control plane, server,
workers, UI, and durable deployment in infrastructure you manage. It exposes an
Open Responses-compatible API for remote clients and SDKs.

[Docker Compose quickstart →](https://docs.everruns.com/getting-started/docker-compose/) · [Platform architecture →](https://docs.everruns.com/explanation/architecture/)

### Use Hosted Everruns

Use [Hosted Everruns](https://app.everruns.com) when you want the shared runtime
and production operations without operating the platform yourself.

[Open Hosted Everruns →](https://app.everruns.com)

## The Everruns platform

The platform is the optional runtime and control plane for agents that need to
run beyond one application process. It provides a server, stateless worker pool,
web UI, remote API, and PostgreSQL-backed durable execution. Workers communicate
with the control plane over gRPC and do not hold database credentials.

It also provides the operational surfaces for publishing agents to Slack, web
chat, A2A, webhooks, schedules, voice, HTTP, and MCP; managing organizations and
permissions; and observing, budgeting, and evaluating production agents.

[Platform capabilities](https://docs.everruns.com/features/capabilities/) · [Apps and channels](https://docs.everruns.com/features/apps/) · [Durable execution](https://docs.everruns.com/explanation/durable-execution/) · [Observability](https://docs.everruns.com/observability/)

## Documentation

Full documentation lives at **[docs.everruns.com](https://docs.everruns.com)**.

- [Everruns Framework](https://docs.everruns.com/framework/) — build and run agents inside a Rust application
- [Framework quickstart](https://docs.everruns.com/framework/quickstart/) — install the crate and configure a provider
- [Docker Compose quickstart](https://docs.everruns.com/getting-started/docker-compose/) — run the full platform stack
- [How-to guides](https://docs.everruns.com/how-to/) — give agents tools, stream events, publish to Slack, and enforce budgets
- [API reference](https://docs.everruns.com/api/) — OpenAPI 3.0
- [SDKs](https://docs.everruns.com/features/sdk/) — Rust, Python, and TypeScript clients

## Security

Everruns runs untrusted agent and tool code for multiple tenants, so security is
a core design goal. Threats are tracked with stable IDs across authentication,
tenant isolation, permissions, tool execution, LLM integration, sandboxes,
durable execution, and channel integrations, each with a documented mitigation
and, where feasible, test coverage.

- [Threat model](./knowledge/security/threat-model.md) — full analysis, mitigation status, and accepted risks
- [Security testing](./knowledge/security/security-testing.md) — threat-model tests, failure injection, DeepSec scanning, and supply-chain checks
- [Security policy](./SECURITY.md) — report a vulnerability

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) for local development setup and
[AGENTS.md](./AGENTS.md) for the conventions used by both human and AI
contributors.

## License

MIT

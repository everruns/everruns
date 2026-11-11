# Everruns

[![Website](https://img.shields.io/badge/Website-everruns.com-blue)](https://everruns.com)
[![Docs](https://img.shields.io/badge/Docs-everruns.com%2Fdocs-green)](https://everruns.com/docs)
[![Crates.io](https://img.shields.io/crates/v/everruns.svg)](https://crates.io/crates/everruns)
[![CI](https://github.com/everruns/everruns/actions/workflows/ci.yml/badge.svg)](https://github.com/everruns/everruns/actions/workflows/ci.yml)
[![Repo: Agent Friendly](https://img.shields.io/badge/Repo-Agent%20Friendly-blue)](AGENTS.md)

**Build capable AI agents in Rust. Run them on your own machines.**

Everruns is an open-source framework and durable runtime for AI agents in Rust.
Define agents, attach models and typed tools, run multi-turn sessions, and
observe execution through one application-facing API.

It is a free, non-commercial project by [Mykhailo Chalyi](https://chaliy.name)
and contributors, pre-1.0, and part of the
[Everruns community ecosystem](https://everruns.com/projects/) of open-source
AI projects. There is no company behind it and no paid edition. Talk to us on
[Discord](https://everruns.com/discord/everruns).

Use the framework in your application. When you need a shared runtime, run the
Everruns platform yourself on your machine, a server or Kubernetes.

[Build with the framework](https://everruns.com/docs/framework/quickstart/) · [Run it yourself](https://everruns.com/docs/getting-started/introduction/#self-hosted) · [Read the docs](https://everruns.com/docs/framework/)

<p align="center">
  <img src="./assets/readme/banner.png" alt="Everruns" width="100%" />
</p>

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
        .model("gpt-6.1-sol")
        .tool(current_time())
        .build()?;

    let session = Engine::new().create(agent);
    let turn = session.send_and_wait("What time is it?").await?;
    println!("{}", turn.response);
    Ok(())
}
```

Everruns derives the tool schema from the Rust function, lets the model call it during the turn, and returns the result before producing the final response. [Continue the framework quickstart.](https://everruns.com/docs/framework/quickstart/)

> **Note:** Everruns is under active development. Expect rapid changes and experimental features.

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

[Framework overview](https://everruns.com/docs/framework/)

## Choose how you run Everruns

| Framework | Self-hosted platform |
| --- | --- |
| Start here. Embed Everruns in the Rust application you are building; you own the process, deployment, integrations, and data path.<br><br>[Framework quickstart →](https://everruns.com/docs/framework/quickstart/) | Run the shared runtime in infrastructure you manage when you need a control plane, server, workers, UI, remote API, and durable execution.<br><br>[On your machine →](https://everruns.com/docs/getting-started/run-locally/) · [Docker Compose →](https://everruns.com/docs/getting-started/docker-compose/) · [Kubernetes →](https://everruns.com/docs/getting-started/kubernetes/) · [Architecture →](https://everruns.com/docs/explanation/architecture/) |

### Set it up with your agent

Example prompts a person gives their coding agent:

```text
Install Everruns on this machine with Docker and sign me in. Instructions:
https://everruns.com/.well-known/agent-skills/everruns-install/SKILL.md
```

```text
Install Everruns on my Kubernetes cluster. Instructions:
https://everruns.com/.well-known/agent-skills/everruns-install/SKILL.md
```

```text
Add an Everruns agent with a typed tool to this Rust project and run it once.
Instructions: https://everruns.com/.well-known/agent-skills/everruns-framework/SKILL.md
```

The [Everruns plugin](https://github.com/everruns/plugins) includes these
skills, so with it installed the links are optional.

No server to spare? Look around the [demo instance](https://app.everruns.com)
([quickstart](https://everruns.com/docs/getting-started/cloud/)). It runs the
same open-source code and is a demo, not a hosted service.

### Platform capabilities

The Platform, self-hosted or on the demo instance, adds durable execution, a stateless worker
pool, a web UI, and a remote API. It also publishes agents to Slack, web chat,
A2A, webhooks, schedules, voice, HTTP, and MCP; manages organizations and
permissions; and supports observation, budgeting, and evaluation.

[Platform capabilities](https://everruns.com/docs/features/capabilities/) · [Channels](https://everruns.com/docs/features/channels/) · [Durable execution](https://everruns.com/docs/explanation/durable-execution/) · [Observability](https://everruns.com/docs/observability/)

## How can I support this project?

Everruns is a free, non-commercial community project. There are many ways to help:

- Use Everruns and tell us how it went on [Discord](https://everruns.com/discord/everruns) or by email ([mykhailo@everruns.com](mailto:mykhailo@everruns.com))
- Take one of the [open research questions](https://everruns.com/research/) and share what you measured
- Report bugs and rough edges in [GitHub Issues](https://github.com/everruns/everruns/issues)
- Star the repository, it helps other people find it
- Write a blog post or share the agent you built
- Help with the [documentation](https://everruns.com/docs/) by asking questions or sending PRs
- Contribute code (see below, and say hi on [Discord](https://everruns.com/discord/everruns))
- Talk about Everruns around you

## Documentation

Full documentation lives at **[everruns.com/docs](https://everruns.com/docs)**.

- [Everruns Framework](https://everruns.com/docs/framework/) — build and run agents inside a Rust application
- [Framework quickstart](https://everruns.com/docs/framework/quickstart/) — install the crate and configure a provider
- [Run on your machine](https://everruns.com/docs/getting-started/run-locally/) — `everruns docker up` starts the full Platform in Docker
- [Docker Compose](https://everruns.com/docs/getting-started/docker-compose/) and [Kubernetes](https://everruns.com/docs/getting-started/kubernetes/) — run it on a server or a cluster
- [How-to guides](https://everruns.com/docs/how-to/) — give agents tools, stream events, publish to Slack, and enforce budgets
- [API reference](https://everruns.com/docs/api/) — OpenAPI 3.0
- [SDKs](https://everruns.com/docs/features/sdk/) — Rust, Python, and TypeScript clients
- [Docs as text](https://everruns.com/docs/llms.txt) — `llms.txt` index, per-topic Markdown sets, and the OpenAPI schema for agents

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

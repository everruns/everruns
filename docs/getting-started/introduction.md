---
title: Start here
description: Choose how you run Everruns, the Framework, a self-hosted Platform, or Everruns Cloud, and go to the right first page.
sidebar:
  label: Start here
---

Everruns is a durable agentic harness engine built on Rust. You can use it in
three ways. They run the same agent model (agents, sessions, capabilities,
events) and differ in what you operate.

| | Framework | Self-hosted | Everruns Cloud |
|---|---|---|---|
| **For** | Rust developers who want agents inside their own application | Teams that need the full Platform on their own infrastructure | Anyone who wants the Platform without operating it |
| **You operate** | Your application process. No server, database, or worker | The Platform: API server, workers, PostgreSQL, UI | Nothing. Everruns runs the Platform at [app.everruns.com](https://app.everruns.com) |
| **You talk to it through** | The `everruns` Rust crate, in process | The REST API, SDKs, CLI, UI, and MCP | The REST API, SDKs, CLI, UI, and MCP |
| **Cost** | Free and open source (MIT). You pay your model provider | Free and open source (MIT). You pay for your infrastructure and model providers | Prepaid credit for the built-in model provider, with $5 of starter credit. Your own provider keys are optional |
| **First page** | [Framework quickstart](/framework/quickstart/) | [Docker Compose](/getting-started/docker-compose/) | [Everruns Cloud quickstart](/getting-started/cloud/) |

Pages across these docs carry an **Applies to** row under the title that names
which of the three the page covers.

## Framework

Embed agents in a Rust process with the `everruns` crate. The agent runs inside
your application, so there is nothing else to deploy. Bring a model provider
key.

```rust
let agent = Agent::builder().instructions("Be concise.")
    .provider(OpenAI::from_env()?).model("gpt-5.6-terra").build()?;
let session = Engine::new().create(agent);
println!("{}", session.send_and_wait("Say hello.").await?.response);
```

Next: [Framework quickstart](/framework/quickstart/), which sets up the crate
and runs this program end to end.

## Self-hosted

Run the full Platform on infrastructure you control: the API server, durable
workers, PostgreSQL, and the management UI. Use it when you need remote
clients, multi-tenant organizations, or durable execution that survives a worker
restart, and want to keep the deployment and data in your own environment.

```bash
curl -o docker-compose.yaml https://raw.githubusercontent.com/everruns/everruns/main/examples/docker-compose-full.yaml
# write .env with the required secrets, see the Docker Compose guide
docker compose up -d   # UI and API on http://localhost:9300
```

Next: [Docker Compose](/getting-started/docker-compose/), which lists the
required secrets and walks through the first session. Operators continue with
[Environment variables](/sre/environment-variables/).

## Everruns Cloud

Use the Platform at [app.everruns.com](https://app.everruns.com) without running
it. A built-in model provider is ready on sign-up, and your first organization
gets $5 of starter credit, so you need no provider keys to start.

```bash
everruns login
SESSION=$(everruns sessions create -q)
everruns chat --session "$SESSION" "Say hello."
```

Next: [Everruns Cloud quickstart](/getting-started/cloud/), which covers sign-up,
tokens, the SDK, CLI and AI-tool setup, and how credit works.

## After the first page

- [Concepts](/getting-started/concepts/): agents, sessions, harnesses,
  capabilities, and events.
- [Use in AI Tools](/getting-started/use-in-ai-tools/): drive Everruns from
  Claude Code, Codex, or Cursor.
- [Build your first agent](/tutorials/building-agents-using-sdk/): a guided SDK
  lesson for the self-hosted Platform or Everruns Cloud.
- [Capabilities](/capabilities/): the tools an agent can use, each marked with
  where it is available.

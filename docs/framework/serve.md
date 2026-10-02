---
title: Serve (experimental)
description: Build a hosted agent app with attribute macros, file-layout discovery and a manifest, served over a subset of the Everruns server /v1 API.
---

> **Experimental.** serve is a proof of concept. Its APIs will change and it
> has no compatibility promise.

**serve** is an application framework on top of the Everruns Framework. You
write an app as a set of annotated functions, the build declares what the app
needs from its host, and the binary serves the same `/v1` session API as the
Everruns server, so the SDKs, CLI and UI chat view can drive it.

serve adds no runtime of its own. Agents, sessions, tools, approvals,
[`ask_user`](/framework/ask-user/), skills and the durable event log all come
from the `everruns` crate: serve is a thin layer over
[`Engine`](/framework/architecture/).

```rust
use serve::prelude::*;

#[agent]
fn analyst() -> Agent {
    Agent::builder()
        .model("anthropic/claude-sonnet-5")    // resolved by the host's model gateway
        .instructions(md!("instructions.md"))  // agent/instructions.md, hot-reloads in dev
        .build()
}

/// Run a read-only SQL query against the warehouse.
#[tool(needs_approval = |a: &RunSql| scan_gb(&a.sql) > 50.0)]
async fn run_sql(cx: &Cx, sql: String) -> Result<Rows> {
    let wh = cx.connection::<Warehouse>()?;   // credentials never reach the model
    cx.progress("querying…").await;
    Ok(wh.query(&sql)?.truncate(500))
}

serve::assets!();

#[tokio::main]
async fn main() -> serve::Result {
    serve::start(App::builder().discover().build()).await
}
```

## Install

```sh
cargo add everruns-serve tokio --features tokio/full
cargo add --build everruns-serve-build
```

The package is `everruns-serve` and the library is imported as `serve`. Add a
`build.rs` that calls `serve_build::embed()` so `agent/**` and `serve.toml` are
compiled into the binary.

## Try it

Both examples run offline against a scripted simulator, so no keys or network
are needed. From a checkout of the repository:

```sh
cargo run -p serve-example-revenue-analyst            # dev server on :3000 with a live console
cargo run -p serve-example-revenue-analyst -- eval    # run the evals in-process
cargo run -p serve-example-revenue-analyst -- manifest
```

Then talk to it with the same requests you would send an Everruns server:

```sh
ID=$(curl -s localhost:3000/v1/sessions -H 'content-type: application/json' -d '{}' | jq -r .id)
curl -s localhost:3000/v1/sessions/$ID/messages -H 'content-type: application/json' \
  -d '{"message":{"content":[{"type":"text","text":"What was revenue last week?"}]}}'
curl -N "localhost:3000/v1/sessions/$ID/sse?after_sequence=0"
```

To use a real model, set `OPENROUTER_API_KEY`, or point `SERVE_GATEWAY_URL`
and `SERVE_GATEWAY_KEY` at any OpenAI-compatible gateway. With serve's
`bedrock` feature, `bedrock/<model-id>` models (for example
`bedrock/us.anthropic.claude-sonnet-4-6`) call Amazon Bedrock on the AWS
default credential chain whenever `AWS_REGION` is set.

| Example | Shows |
| --- | --- |
| [`examples/serve/hello`](https://github.com/everruns/everruns/tree/main/examples/serve/hello) | The smallest app: one agent, one tool, one eval. |
| [`examples/serve/revenue-analyst`](https://github.com/everruns/everruns/tree/main/examples/serve/revenue-analyst) | A tool with approvals, a skill, Slack, a schedule, MCP and typed connections, a subagent, evals and the Bashkit sandbox. |
| [`examples/serve/ag-ui`](https://github.com/everruns/everruns/tree/main/examples/serve/ag-ui) | An agent streamed to `@ag-ui/client` and CopilotKit, with an approval as an interrupt. |
| [`examples/serve/agentcore`](https://github.com/everruns/everruns/tree/main/examples/serve/agentcore) | The same kind of app packaged for Amazon Bedrock AgentCore Runtime. |
| [`examples/serve/agentcore-workspace`](https://github.com/everruns/everruns/tree/main/examples/serve/agentcore-workspace) | An AgentCore workspace agent with a shell in the microVM and an approval. |
| [`examples/serve/a2a`](https://github.com/everruns/everruns/tree/main/examples/serve/a2a) | Two agents over A2A: a served `researcher`, and an `everruns` agent that delegates to it. |

## Project layout

The file layout says what each piece is:

```text
revenue-analyst/
  build.rs                 # serve_build::embed()
  serve.toml               # name, sandbox, extra secrets
  agent/
    instructions.md        # always-on prompt
    skills/sql-style/SKILL.md
  src/
    main.rs                # serve::assets!() + serve::start(...)
    agent.rs               # #[agent]
    tools/run_sql.rs       # #[tool]
    channels/slack.rs      # #[channel]
    schedules/weekly.rs    # #[schedule]
    connections/linear.rs  # #[connection]
    subagents/reviewer.rs  # #[agent(sub)]
  evals/revenue.rs         # #[eval]
```

The macros register each item at link time and `discover()` collects them;
`build.rs` embeds `agent/**` and `serve.toml` into the binary. `discover()`
reports every problem in the app at once, such as duplicate names, a bad cron
expression or a filter naming an unknown tool.

## The pieces

| Macro | On | Becomes |
| --- | --- | --- |
| `#[agent]`, `#[agent(default)]`, `#[agent(sub)]` | `fn() -> Agent` | An agent. New sessions use the default one; a subagent becomes an `ask_<name>` tool on the others. |
| `#[tool]`, `#[tool(needs_approval)]`, `#[tool(needs_approval = \|a: &Args\| …)]` | `async fn([cx: &Cx,] args…) -> Result<T>` | A tool named after the function, described by its doc comment. It also generates an arguments struct (`run_sql` → `RunSql`) with a JSON Schema. |
| `#[channel]` | `fn() -> impl Channel` | `POST /v1/channels/{name}`. Each external thread maps to one session, and replies are delivered back. |
| `#[schedule("0 9 * * MON")]` | `async fn(&Cx) -> Result` | A cron entry. |
| `#[connection]` | `fn() -> T` or `fn() -> Result<T>` | A value tools read with `cx.connection::<T>()`. An `McpServer` connection is attached to every agent. |
| `#[eval]` | `async fn(&mut EvalCx) -> Result` | A conversation with assertions, run in-process or against a deployment. |

`Cx` is the one context type. Inside a tool it wraps the Framework's
`ToolCallContext` (session, turn and tool call ids, and `progress`) and adds
typed connections, secrets and `start_session`. Approval rules are declared on
the tool and enforced by the runtime.

## Commands

The commands are built into the app binary, because only the linked binary
knows what it registered.

| Command | What it does |
| --- | --- |
| `dev` (default) | Serves the API with local SQLite under `.serve/` and a live console. Models fall back to the simulator, and Markdown prompts and skills hot-reload. |
| `start` | Production mode. Every model must route through a gateway, and every declared secret must be set. |
| `manifest` | Prints the host contract as JSON: agents, models, tool schemas, skills, channels, cron entries, secrets, sandbox, evals and a build id. |
| `eval [--against URL]` | Runs the evals in-process, or against a running deployment as a gate before promotion. |
| `deploy` | Prints what a host would provision from the manifest. The one supported deployment target is [Amazon Bedrock AgentCore](/framework/serve-agentcore/). |

## Wire API

A serve app speaks a subset of the Everruns server `/v1` API, with the same
request and response bodies:

- `POST /v1/sessions` and `GET /v1/sessions/{id}`
- `POST /v1/sessions/{id}/messages`, which also steers an active turn
- `POST /v1/sessions/{id}/cancel`
- `GET /v1/sessions/{id}/sse` (resume with `since_id` or `after_sequence`) and `GET /v1/sessions/{id}/events`
- `POST /v1/sessions/{id}/question-answers`, for [`ask_user`](/framework/ask-user/) questions

It adds a few routes of its own: `GET /health`, `GET /v1/agent` (the agent
card), channel webhooks, the [AG-UI](#ag-ui-and-copilotkit) and [A2A](#a2a) routes, and `POST /v1/sessions/{id}/approvals/{tool_call_id}`
to approve or deny a pending tool call. Errors are `application/problem+json`.

## AG-UI and CopilotKit

With the `ag-ui` feature, every top-level agent also serves
[AG-UI](https://docs.ag-ui.com) 1.0 clients such as CopilotKit and
`@ag-ui/client`:

```sh
cargo add everruns-serve --features ag-ui
```

The route is `POST /v1/e/{agent}/ag-ui`, the same shape as an Everruns
endpoint's AG-UI route, so a front end moves between a local serve app and
Everruns by base URL and id alone:

```ts
import { HttpAgent } from "@ag-ui/client";

const agent = new HttpAgent({ url: "http://localhost:3000/v1/e/analyst/ag-ui" });
```

Each AG-UI `threadId` maps to one session, which survives a restart. A
thread's first run records the input's earlier user and assistant messages as
the new session's history; after that, a run sends only the input's last user
message. Each run streams the turn as AG-UI events,
with reasoning, token usage and errors visible. A tool approval or an
[`ask_user`](/framework/ask-user/) question ends the run with an interrupt
(`tool_approval` or `everruns.ask_user`), and the next run's `resume` entries
answer it and stream the rest of the turn. These are the same pending requests
the approvals and `question-answers` routes answer, so either API can resolve
them. Input that is not a valid `RunAgentInput` gets `400`, and an unknown
agent `404`. The agent card lists each agent's route under `ag_ui`. See
[Serve AG-UI](/framework/ag-ui/) for the event and interrupt shapes.

## A2A

With the `a2a` feature, every top-level agent also serves other agents over
[A2A](https://a2a-protocol.org) 1.0 JSON-RPC:

```sh
cargo add everruns-serve --features a2a
```

The endpoint is `POST /v1/e/{agent}/a2a` and its Agent Card is
`GET /v1/e/{agent}/a2a/.well-known/agent-card.json`, the shape of an Everruns
A2A endpoint. Any A2A 1.0 client works, including the official `a2a` CLI and
another Everruns agent's `a2a_agent_delegation` capability:

```sh
a2a send -a http://localhost:3000/v1/e/researcher/a2a/.well-known/agent-card.json "Tide pools"
```

Each A2A `contextId` maps to one session, which survives a restart; each task
is one turn. A task streams `working`, then the final reply as a `response`
artifact, then `completed` (or `failed`). `GetTask`, `ListTasks`, `CancelTask`
and `SubscribeToTask` come from the A2A Rust SDK's request handler; tasks are
kept in memory, so old task ids are forgotten on restart while their context
continues. Requests need the `A2A-Version: 1.0` header. A pending approval or
`ask_user` question keeps the task `working` until the routes above answer it.
The agent card lists each agent's endpoint under `a2a`.

Sessions survive a restart: the binary rebuilds each agent and resumes the
session from the local store. A turn the old process left waiting on an
approval or an `ask_user` question waits again: the call runs again, so the
request is pending once more under the same tool call id, and answering it
finishes the turn. A turn cut off while a tool executed is not re-run. A
session pinned to a different build gets
`409 Conflict` with an `x-serve-build` header, so a host can route it to the
build that owns it.

> **No authentication.** The wire API has no authentication and no
> organizations. Run it locally, or behind a host that authenticates requests.

## Limitations

- After a restart, an approval answered "always" before it is asked again, and a
  request parked by a subagent is lost.
- A2A tasks are held in memory, and A2A 0.3 clients are not served.
- A deny note is not passed to the model.
- There is no generic OCI build or Postgres or NATS adapter. [AgentCore](/framework/serve-agentcore/) is the one supported deployment target.
- The server's agent, harness, workspace and tool-result routes are not served.

## Amazon Bedrock AgentCore

[`everruns-serve-agentcore`](https://docs.rs/everruns-serve-agentcore) runs a
serve app on AgentCore Runtime. Replace `serve::start` with
`serve_agentcore::start` in `main`. With no command, the binary then serves
AgentCore's contract on port 8080: `GET /ping` and `POST /invocations`, which
takes an AG-UI `RunAgentInput` or `{"prompt": "..."}` and streams AG-UI
events. serve's own commands keep working. See
[Serve on AgentCore](/framework/serve-agentcore/) for deployment, persistence on
session storage, tools and models.

## celld

[`everruns-serve-celld`](https://docs.rs/everruns-serve-celld) runs a serve app
durably on [celld](https://github.com/denoland/celld), self-hosted Durable
Objects. Replace `serve::start` with `serve_celld::start`; the binary then runs
in a container that a Durable Object supervises, and the object keeps a snapshot
and a request journal so a lost container is restored and its interrupted turn
replayed. See [Serve on celld](/framework/serve-celld/).

The full guide, wire reference and hosting contract live next to the crate in
[`crates/serve/docs`](https://github.com/everruns/everruns/tree/main/crates/serve/docs).

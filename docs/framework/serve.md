---
title: Serve (experimental)
description: Build a hosted agent app with attribute macros, file-layout discovery and a manifest, served over a subset of the Everruns server /v1 API.
sidebar:
  order: 1
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
and `SERVE_GATEWAY_KEY` at any OpenAI-compatible gateway.

| Example | Shows |
| --- | --- |
| [`examples/serve/hello`](https://github.com/everruns/everruns/tree/main/examples/serve/hello) | The smallest app: one agent, one tool, one eval. |
| [`examples/serve/revenue-analyst`](https://github.com/everruns/everruns/tree/main/examples/serve/revenue-analyst) | A tool with approvals, a skill, Slack, a schedule, MCP and typed connections, a subagent, evals and the Bashkit sandbox. |

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
| `deploy` | Prints what a host would provision from the manifest. There is no cloud target yet. |

## Wire API

A serve app speaks a subset of the Everruns server `/v1` API, with the same
request and response bodies:

- `POST /v1/sessions` and `GET /v1/sessions/{id}`
- `POST /v1/sessions/{id}/messages`, which also steers an active turn
- `POST /v1/sessions/{id}/cancel`
- `GET /v1/sessions/{id}/sse` (resume with `since_id` or `after_sequence`) and `GET /v1/sessions/{id}/events`
- `POST /v1/sessions/{id}/question-answers`, for [`ask_user`](/framework/ask-user/) questions

It adds a few routes of its own: `GET /health`, `GET /v1/agent` (the agent
card), channel webhooks, and `POST /v1/sessions/{id}/approvals/{tool_call_id}`
to approve or deny a pending tool call. Errors are `application/problem+json`.

Sessions survive a restart: the binary rebuilds each agent and resumes the
session from the local store. A session pinned to a different build gets
`409 Conflict` with an `x-serve-build` header, so a host can route it to the
build that owns it.

> **No authentication.** The wire API has no authentication and no
> organizations. Run it locally, or behind a host that authenticates requests.

## Limitations

- Pending approvals and questions are held in memory and do not survive a restart.
- A deny note is not passed to the model.
- There is no OCI build, cloud deploy, or Postgres or NATS adapter.
- The server's agent, harness, workspace and tool-result routes are not served.

The full guide, wire reference and hosting contract live next to the crate in
[`crates/serve/docs`](https://github.com/everruns/everruns/tree/main/crates/serve/docs).

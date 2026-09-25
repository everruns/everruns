# serve

> **Experimental.** serve is a proof of concept for an application framework
> on top of everruns. It is published to crates.io as `everruns-serve`, but its
> APIs will change and it is outside the everruns
> [stability policy](https://github.com/everruns/everruns/blob/main/knowledge/framework/api-stability.md).

serve takes its API shape from [Topcoat](https://github.com/tokio-rs/topcoat)
and its hosting model from [eve](https://vercel.com/blog/introducing-eve):

- **Topcoat's API shape.** An app is `serve::start(App::builder().discover().build())`
  plus attribute macros. Each piece gets what it needs through one `&Cx`.
- **eve's hosting model.** The file layout says what each piece is. The build
  declares what it needs (a manifest), and the host provides it, and sessions
  survive restarts.
- **The everruns server's wire API.** Every deployment serves a subset of the
  everruns server's `/v1` session API (sessions, messages, `/sse`, `/events`,
  cancel, question-answers), so the everruns SDK, CLI and UI chat view can
  drive a serve app.

It is a thin layer over `everruns::Engine` and adds no new runtime: agents,
sessions, tools, approvals, `ask_user` and the durable event log are the
existing `everruns` crate.

```rust
use serve::prelude::*;

#[agent]
fn analyst() -> Agent {
    Agent::builder()
        .model("anthropic/claude-sonnet-5")      // resolved by the host's model gateway
        .instructions(md!("instructions.md"))    // agent/instructions.md, hot-reloads in dev
        .build()
}

/// Run a read-only SQL query against the warehouse.
#[tool(needs_approval = |a: &RunSql| scan_gb(&a.sql) > 50.0)]
async fn run_sql(cx: &Cx, sql: String) -> Result<Rows> {
    let wh = cx.connection::<Warehouse>()?;   // credentials never reach the model
    cx.progress("querying…").await;
    Ok(wh.query(&sql)?.truncate(500))
}

#[schedule("0 9 * * MON")]
async fn weekly(cx: &Cx) -> Result {
    cx.start_session("Summarize last week's revenue")
        .deliver_to(slack::channel("C0123ABC"))
        .await
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

Both examples run offline. Without a model gateway they use a scripted
simulator, so you need no keys and no network.

```sh
cargo run -p serve-example-revenue-analyst            # dev server on :3000, live console
cargo run -p serve-example-revenue-analyst -- eval    # evals, in-process
cargo run -p serve-example-revenue-analyst -- manifest
```

In another terminal:

```sh
ID=$(curl -s localhost:3000/v1/sessions -H 'content-type: application/json' -d '{}' | jq -r .id)
curl -s localhost:3000/v1/sessions/$ID/messages -H 'content-type: application/json' \
  -d '{"message":{"content":[{"type":"text","text":"What was revenue last week?"}]}}'
curl -N "localhost:3000/v1/sessions/$ID/sse?after_sequence=0"   # replay, then live; resume with since_id
```

To use a real model, set `OPENROUTER_API_KEY` (it accepts `provider/model`
ids as they are). You can instead point `SERVE_GATEWAY_URL` and
`SERVE_GATEWAY_KEY` at any OpenAI-compatible gateway.

| Example | Shows |
|---|---|
| [`examples/serve/hello`](https://github.com/everruns/everruns/tree/main/examples/serve/hello) | The smallest app: one agent, one tool, one eval. |
| [`examples/serve/revenue-analyst`](https://github.com/everruns/everruns/tree/main/examples/serve/revenue-analyst) | The full layout: a tool with approvals, a skill, Slack, a schedule, MCP and typed connections, a subagent, evals, and the bashkit sandbox. |

## Project layout

```text
revenue-analyst/
  Cargo.toml
  build.rs                 # serve_build::embed()
  serve.toml               # name, sandbox, extra secrets, deploy target
  agent/
    instructions.md        # always-on prompt (hot-reloads in dev)
    skills/sql-style/SKILL.md
  src/
    main.rs                # serve::assets!() + serve::start(...)
    agent.rs               # #[agent]
    tools/run_sql.rs       # #[tool]          (fn name = tool name)
    channels/slack.rs      # #[channel]
    schedules/weekly.rs    # #[schedule]
    connections/linear.rs  # #[connection]
    subagents/reviewer.rs  # #[agent(sub)]
  evals/revenue.rs         # #[eval]
```

Rust cannot scan the filesystem at runtime the way eve does. Two things stand
in for that:

- The macros register items at link time, with
  [`inventory`](https://docs.rs/inventory), and `discover()` collects them.
- `build.rs` embeds `agent/**` and `serve.toml` into the binary.

The layout is a convention. `discover()` warns when an item lives somewhere
else, and it reports every error in the app at once: duplicate names, a bad
cron, a filter naming an unknown tool.

## The pieces

| Macro | On | Becomes |
|---|---|---|
| `#[agent]`, `#[agent(default)]`, `#[agent(sub)]` | `fn() -> Agent` | An agent. `/v1/sessions` serves the default one; a subagent becomes an `ask_<name>` tool on the others. |
| `#[tool]`, `#[tool(needs_approval)]`, `#[tool(needs_approval = \|a: &Args\| …)]` | `async fn([cx: &Cx,] args…) -> Result<T>` | A tool. The name is the fn name and the doc comment is the description. It also generates an `Args` struct (`run_sql` → `RunSql`) with a JSON Schema. |
| `#[channel]` | `fn() -> impl Channel` | `POST /v1/channels/{name}`. Each thread maps to one session, and replies are delivered back to it. It also generates `name::channel(target)`. |
| `#[schedule("0 9 * * MON")]` | `async fn(&Cx) -> Result` | A cron entry. |
| `#[connection]` | `fn() -> T` or `fn() -> Result<T>` | A value tools get with `cx.connection::<T>()`. An `McpServer` is also attached to every agent. |
| `#[eval]` | `async fn(&mut EvalCx) -> Result` | A conversation with assertions. It runs in-process or `--against <url>`. |

`Cx` is the one context type. Inside a tool it wraps the runtime's
`ToolCallContext` (session, turn and tool call ids, `progress(...)`) and adds
typed connections, secrets and `start_session(...)`. Approvals are declared on
the tool and enforced by the runtime; every agent also has the built-in
`ask_user` tool.

## Commands

The commands are part of the app binary, because only the linked binary knows
what it registered.

| Command | What it does |
|---|---|
| `dev` (default) | Runs the wire API with SQLite under `.serve/` and a live console of turns, tool calls and approvals. Models fall back to the simulator. Markdown prompts and skills hot-reload; for Rust changes use `cargo watch -x run`. `POST /dev/schedules/{name}` fires a schedule on demand. |
| `start` | Production mode. Every model must route through a gateway, and every declared secret must be set. |
| `manifest [--out f]` | Prints the host contract as JSON. |
| `eval [--against URL] [filter]` | Runs the evals. With `--against`, it runs them against a running deploy (a CI gate before promotion). |
| `deploy` | Prints what a host would provision from the manifest. This PoC has no cloud target. |

## Docs

- [Guide](https://github.com/everruns/everruns/blob/main/crates/serve/docs/guide.md): writing an app, piece by piece.
- [Wire API](https://github.com/everruns/everruns/blob/main/crates/serve/docs/wire-api.md): the `/v1` routes, the event stream, and resuming.
- [Manifest and hosting](https://github.com/everruns/everruns/blob/main/crates/serve/docs/hosting.md): what the build declares, what a host
  provides, and how build pinning works.
- [Design note](https://github.com/everruns/everruns/blob/main/knowledge/framework/serve.md): why it is shaped like this,
  and the open questions.

## Status

This PoC works end to end offline: discovery, the manifest, the
server-compatible wire API (driven by the everruns Rust SDK in tests) with
resumable SSE, approvals, `ask_user` answers, cancel, restart and resume,
Slack and webhook channels, schedules, subagents, skills, sandbox selection,
and evals both in-process and remote.

It does not have:

- a `serve` CLI wrapper or OCI image build;
- a real cloud deploy;
- Postgres or NATS adapters;
- `#[memoize]` on `Cx`;
- per-build routing (the manifest defines the contract, but nothing routes on it yet);
- approvals or questions that survive a restart, or a deny note that reaches
  the model;
- auth, organizations, or the server's agent, harness and workspace routes;
- Slack signature checks without `SLACK_SIGNING_SECRET` (they are skipped in dev).

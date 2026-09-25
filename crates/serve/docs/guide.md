# Writing a serve app

> Experimental. See the [README](../README.md) for status.

This guide builds an app one piece at a time. The complete result is
[`examples/serve/revenue-analyst`](../../../examples/serve/revenue-analyst).

## 1. The crate

```toml
# Cargo.toml
[dependencies]
everruns-serve = "0.32"   # the library is imported as `serve`
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[build-dependencies]
everruns-serve-build = "0.32"
```

```rust
// build.rs: embeds agent/** and serve.toml into the binary
fn main() {
    serve_build::embed();
}
```

```rust
// src/main.rs
mod agent;
mod tools;

serve::assets!(); // includes what build.rs embedded; call once

#[tokio::main]
async fn main() -> serve::Result {
    serve::start(serve::App::builder().discover().build()).await
}
```

Every module with a macro in it must be reachable from `main.rs` (`mod
tools;`). The macros register items at link time, so an item in a module
that is never declared is not compiled at all, and `discover()` cannot see it.

## 2. An agent

```rust
use serve::prelude::*;

/// Answers revenue questions from the warehouse.   // shown in the agent card
#[agent]
fn analyst() -> Agent {
    Agent::builder()
        .model("anthropic/claude-sonnet-5")
        .instructions(md!("instructions.md"))
        .build()
}
```

- **Model strings** are `provider/model`, and the host's gateway resolves them.
  Locally, the resolution order is `SERVE_GATEWAY_URL`, then
  `OPENROUTER_API_KEY`, then `OPENAI_API_KEY` (for `openai/…` models only).
  If none is set, `dev` and `eval` fall back to the agent's `.offline(...)`
  script, or to an echo simulator. `"sim"` always uses the simulator.
- **Instructions** come from `md!("…")`, a file under `agent/` that is
  compiled in and, in `dev`, re-read from disk for every new session. Without
  `.instructions(...)`, `agent/instructions.md` is used if it exists.
- **Tools.** Every agent gets every `#[tool]` unless you restrict it with
  `.tools(["run_sql"])`.
- **Escape hatch.** `.customize(|b| b.max_iterations(8))` adjusts the
  underlying `everruns::AgentBuilder` for hooks, capabilities and limits.

Several `#[agent]`s are allowed. `POST /v1/sessions` picks one by
`agent_name` (as the everruns server does), and uses the `#[agent(default)]`
one when the body names none. Keep agent names kebab-safe (`analyst`) if the
everruns SDK should reach them; it validates `agent_name` that way.

Every agent also gets the built-in `ask_user` tool. When the model asks, the
question set waits on the session until someone answers it through
`POST /v1/sessions/{id}/question-answers` (see
[Wire API](wire-api.md#questions-ask_user)).

### Offline scripts

```rust
use serve::sim;

.offline(sim::script([
    sim::call("run_sql", json!({ "sql": "SELECT …" })),
    sim::reply("Here is last week's revenue."),
]))
```

The script replays in order: a `call` is one model step that calls a tool, and
a `reply` ends the turn. It loops, and each new session starts it again from
the beginning. Scripts make `dev` and evals deterministic, and they need no
keys.

## 3. Tools

```rust
/// Run a read-only SQL query against the warehouse.
#[tool(needs_approval = |a: &RunSql| scan_gb(&a.sql) > 50.0)]
async fn run_sql(
    cx: &Cx,
    /// A single SELECT statement.        // becomes the schema field description
    sql: String,
) -> Result<Rows> {
    let wh = cx.connection::<Warehouse>()?;
    cx.progress("querying…").await;
    Ok(wh.query(&sql)?.truncate(500))
}
```

- The fn name is the tool name, the doc comment is its description, and the
  parameters are its JSON arguments. `Option<T>` parameters are optional.
- The optional first parameter `cx: &Cx` is not part of the schema.
- The macro generates `pub struct RunSql { pub sql: String }` next to the
  function. That is what the approval predicate receives.
- `Err(e)` goes back to the model with its full cause chain, so it can
  correct itself. `Ok(v)` is serialized to JSON.
- `#[tool(needs_approval)]` always asks. The closure form decides per call.
  Both become the runtime's per-tool gate (`FunctionTool::needs_approval`);
  serve does not block inside the tool. While a call waits, the session's
  status is `waitingfortoolresults` and the call is listed in its
  `pending_approvals` (see [Wire API](wire-api.md#approvals)).
- `cx.progress(…).await` emits a canonical `tool.progress` event. `Cx` also
  gives the call's `session_id()`, `turn_id()` and `tool_call_id()`, from the
  runtime's `ToolCallContext`.

## 4. Skills

Put a skill in `agent/skills/<name>/SKILL.md`, with frontmatter:

```markdown
---
name: sql-style
description: How to query the shop warehouse.
---
# Warehouse SQL style
…
```

Skills use the everruns built-in `Skills` capability: each one is seeded
read-only at `.agents/skills/<name>/SKILL.md` in the session workspace, and
every agent finds them with `list_skills` and loads one with `activate_skill`. You
write no Rust for a skill; `build.rs` picks it up.

## 5. Connections and secrets

```rust
#[connection]
fn warehouse() -> Result<Warehouse> { Warehouse::connect(Secret::named("WAREHOUSE_URL").value()?) }

#[connection]
fn linear() -> McpServer {
    McpServer::http("https://mcp.linear.app/mcp").auth(Secret::named("LINEAR_TOKEN"))
}
```

- Tools read a typed connection with `cx.connection::<Warehouse>()`. Two
  connections of the same type are a discovery error.
- An `McpServer` is attached to every agent. If its secret is missing, `dev`
  runs without it and `start` refuses to boot.
- `Secret::named` only declares a secret, and every declared secret is listed
  in the manifest. The value is read from the environment the host prepares.

## 6. Channels

```rust
#[channel]
pub fn slack() -> Slack {
    Slack::from_secrets().mention_only()
}
```

This serves `POST /v1/channels/slack`. Each Slack thread maps to one session,
so follow-ups keep their context, and each turn's reply is posted back to the
thread. Without `SLACK_BOT_TOKEN`, replies are printed instead. When
`SLACK_SIGNING_SECRET` is set, requests are verified.

`Webhook` is a generic JSON channel that takes `{"thread", "text",
"callback"?}`. To add another channel, implement the `Channel` trait.

The macro also generates a module of the same name, so code can address the
channel: `slack::channel("C0123ABC")`.

## 7. Schedules

```rust
use crate::channels::slack::slack;

#[schedule("0 9 * * MON")]
async fn weekly(cx: &Cx) -> Result {
    cx.start_session("Summarize last week's revenue")
        .deliver_to(slack::channel("C0123ABC"))
        .await
}
```

A five-field cron is interpreted in UTC. Awaiting `start_session` runs the
first turn and delivers its reply. In `dev`, you can fire a schedule
immediately with `curl -X POST localhost:3000/dev/schedules/weekly`.

## 8. Subagents

```rust
/// Reviews a SQL query before it is shared.
#[agent(sub)]
fn reviewer() -> Agent { … }
```

The other agents get an `ask_reviewer { task }` tool. It runs the subagent to
completion in a child session on the same engine and returns the answer. The
child's tool calls and progress are reported as `tool.progress` events of the
`ask_reviewer` call on the parent session, and its approvals and questions
wait on the parent session.

## 9. Evals

```rust
#[eval]
async fn answers_net_of_refunds(t: &mut EvalCx) -> Result {
    t.send("What was revenue last week?").await?;
    t.completed()?.called_tool("run_sql")?.reply_contains("net of refunds")?;
    Ok(())
}
```

Declare the evals module from `main.rs`
(`#[path = "../evals/mod.rs"] mod evals;`). Then:

```sh
cargo run -- eval                              # in-process, simulator fallback
cargo run -- eval --against https://preview…   # over the wire API
cargo run -- eval net_of_refunds               # filter by name
```

The available checks are `called_tool`, `did_not_call`, `asked_approval` and
`reply_contains`. Approvals are answered automatically; change that with
`t.on_approval(OnApproval::Deny)`. `ask_user` questions are answered with
their declared defaults. In-process, the eval reads the turn's outcome and the
engine's events directly; `--against` drives the same wire API the SDK uses
(post a message, poll the session, answer what is pending, read `/events`).

## 10. The sandbox

```toml
# serve.toml
[sandbox]
kind = "bashkit"   # none | local | bashkit | microvm
```

The code does not change between kinds. `bashkit` gives agents a virtual bash
over the session filesystem, and `local` gives them files under the data dir.
`microvm` is supplied by a host; in `dev`, bashkit stands in for it.

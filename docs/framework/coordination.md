---
title: Agents that start agents
description: Spawn agents for one-off calls, Threads for followed work, and Coordinator for agents that route work instead of doing it.
sidebar:
  order: 4
---

An agent starts other agents one of two ways, and never both:

| | Spawn agents | Threads |
|---|---|---|
| Shape | A call: a brief goes in, a result comes back, done | Followed work: the agent sends follow-ups and closes it when accepted |
| Builder | `SpawnAgents`, `AgentBuilder::spawns(agent)` | `threads::Threads`, `AgentBuilder::threads_with(agent)` |
| Model tools | `spawn_agent` + the task tools | `start_thread`, `message_thread`, `list_threads`, `get_thread`, `close_thread` |
| Host | An Engine with `SqliteHosting`, or a local profile | A local profile |

Threads already cover one-off work, so building an agent with both fails.
Add `threads::Coordinator` next to Threads to make the agent route work to
threads instead of doing it itself. These are the same capabilities the
Platform offers ([Spawn agents](/capabilities/spawn-agents/),
[Threads](/capabilities/threads/)), running inside your own process.

## Spawn agents

```rust
use everruns::{Agent, Engine, Model, SpawnAgents};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let reviewer = Agent::builder()
    .name("reviewer")
    .description("Reviews a draft answer for missing details and mistakes.")
    .instructions("Review the draft you are given. Return at most three corrections.")
    .model(Model::simulated("Looks good."))
    .build()?;
let lead = Agent::builder()
    .name("lead")
    .instructions("Spawn reviewer on your draft and wait for it before you answer.")
    .model(Model::simulated("Done."))
    .capability(SpawnAgents::new().max_open(3)) // optional: settings
    .spawns(reviewer)
    .build()?;
let turn = Engine::new().create(lead).run("Explain Rust ownership.").await?;
# Ok(())
# }
```

The lead gets `spawn_agent`. It passes `agent: "reviewer"` (or `"self"`, the
default, for a fresh copy of itself) and `wait: true` when it is blocked on
the result; otherwise the call runs in the background and the lead is told
when it finishes. The spawned agent keeps its own instructions, model and
capabilities, and runs as its own session in the same Engine.

`SpawnAgents` settings: `run_self(false)` stops the agent from spawning
itself, `max_depth`, `max_open`, `max_total`. Without it, `spawns` adds
Spawn agents with the defaults.

[`spawn_agents.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/spawn_agents.rs)
runs a lead on `gpt-6.2-sol` that spawns two copies of itself in parallel and
a reviewer on `gpt-6-luna`:

```bash
OPENAI_API_KEY=... cargo run -p everruns --features openai,local --example spawn_agents
```

## Threads

```rust
use everruns::threads::{self, Coordinator, ThreadStatus, Threads};
use everruns::{Agent, Engine, LocalConfig, Model, WorkspacePolicy};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let copywriter = Agent::builder()
    .name("copywriter")
    .description("Writes announcements and other public launch copy.")
    .instructions("Write the draft to the file the brief names.")
    .model(Model::simulated("Done."))
    .build()?;
let agent = Agent::builder()
    .name("launch-coordinator")
    .instructions("Start one thread per piece of work. Copy goes to copywriter.")
    .model(Model::simulated("On it."))
    .capability(Threads::new().max_open(4))
    .capability(Coordinator)
    .threads_with(copywriter)
    .workspace_policy(WorkspacePolicy::read_write())
    .local(LocalConfig::new("./launch/.everruns").workspace("./launch"))
    .build()?;

let session = Engine::new().create(agent);
session.run("Write the announcement and draft a pricing FAQ.").await?;
for thread in threads::list(&session).await? {
    if thread.status == ThreadStatus::ReadyForReview {
        println!("{}: {}", thread.title, thread.summary.unwrap_or_default());
    }
}
# Ok(())
# }
```

Threads need the `local` feature and an agent built with
[`.local(...)`](/framework/sessions/): threads, their work and checklists live
in the local profile's SQLite store.

### What the parent does

| Tool | Use |
|---|---|
| `start_thread` | Start a thread with a title, a brief that says what done looks like, and optionally `agent` (`self` or a name added with `threads_with`). |
| `message_thread` | Send a follow-up. After a thread finished, this starts new work on the same thread. |
| `list_threads` | See every thread and where it stands, and the agents a thread may run. |
| `get_thread` | Read one thread's work and recent messages. |
| `close_thread` | Close a thread once the person accepted its work, or reopen it. |

### What a thread does

A thread is an ordinary session of the same engine. It talks to its parent
with tools the host gives it: `report_progress` (its checklist, or something
the parent should know now), `ask_parent`, `finish` and `hand_back`. A thread
turn that ends without `finish` flags the work as needing attention, so the
parent always hears back. Open a thread like any other session with
[`Engine::resume`](/framework/sessions/) and its id from `threads::list`.

Threads talk to each other only through the parent.

### Automatic updates

When a thread finishes, asks or reports, the parent gets a user-role message
marked as an automatic update. If the parent is idle, the update starts a
turn; if it is mid-turn, the update steers that turn.

### Status

`threads::list` returns one entry per thread:

| Status | When |
|---|---|
| `NeedsYou` | The thread asked a question, or its work failed. |
| `Working` | The thread is working. |
| `ReadyForReview` | The thread finished; the parent has not closed it yet. |
| `Open` | The work was cancelled and the thread was left open. |
| `Closed` | The parent closed the thread. |

### Limits

- A thread runs the parent's own agent or one added with `threads_with`. A
  local profile has no agent catalog, so any other agent is refused.
- A turn your application starts on a thread directly is not settled against
  its work. Send follow-ups through the parent.
- Defaults: 8 threads open at once and 100 in total (`Threads::max_open`,
  `Threads::max_total`).

[`examples/coordinator-agent`](https://github.com/everruns/everruns/tree/main/examples/coordinator-agent)
runs a launch coordinator on `gpt-6.2-sol` with two threads, one on itself
and one on a copywriter, that write their drafts to a shared workspace.

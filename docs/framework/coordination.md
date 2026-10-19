---
title: Coordinator agents
description: One session hands focused work to threads, hears back from them, and resolves them.
sidebar:
  order: 4
---

A coordinator is an agent that does not do the focused work itself. The person
talks to one session. That session starts a **thread** for each piece of work,
relays follow-ups to it, and reports back when it finishes. Each thread keeps
a checklist and ends its work with a summary. Every report wakes the
coordinator with an automatic update, so nobody has to poll.

This is the same model as Platform Chat's threads, running inside your own
process.

```rust
use everruns::coordination::{self, Coordination, ThreadStatus};
use everruns::{Agent, Engine, LocalConfig, Model, WorkspacePolicy};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .name("launch-coordinator")
    .instructions("Start one thread per piece of work. Tell me when each is ready.")
    .model(Model::simulated("On it."))
    .capability(Coordination::new().max_active_threads(4))
    .workspace_policy(WorkspacePolicy::read_write())
    .local(LocalConfig::new("./launch/.everruns").workspace("./launch"))
    .build()?;

let engine = Engine::new();
let session = engine.create(agent);
session.run("Write the announcement and draft a pricing FAQ.").await?;

for thread in coordination::threads(&session).await? {
    if thread.status == ThreadStatus::ReadyForReview {
        println!("{}: {}", thread.title, thread.summary.unwrap_or_default());
    }
}
# Ok(())
# }
```

Coordination needs the `local` feature and an agent built with
[`.local(...)`](/framework/sessions/): threads, their assignments and their
checklists live in the local profile's SQLite store.

## What the coordinator does

The capability gives the coordinator five tools:

| Tool | Use |
|---|---|
| `start_thread` | Start a thread with a title and a brief that says what done looks like. |
| `message_thread` | Relay a follow-up. After a thread finished, this opens a new unit of work on the same thread. |
| `list_threads` | See every thread and where it stands. |
| `get_thread` | Read one thread's assignments and recent messages. |
| `resolve_thread` | Close a thread once the person accepted its work, or reopen it. |

Threads never talk to each other. Every route goes through the coordinator,
so routing has one owner and one record.

## What a thread does

A thread is an ordinary session of the same engine, running the coordinator's
agent. While it works an assignment it gets its own tools in place of the
coordinator's: `update_checklist`, `complete_assignment`, `ask_decision`,
`report_to_coordinator` and `redirect_to_coordinator`.

A thread turn that ends without `complete_assignment` flags the assignment as
needing attention, so the coordinator always hears back. You can open a
thread like any other session with
[`Engine::resume`](/framework/sessions/) and its id from
`coordination::threads`.

## Automatic updates

When a thread finishes, asks a question or reports, the coordinator gets a
user-role message marked as an automatic update. If the coordinator is idle,
the update starts a turn. If it is mid-turn, the update steers that turn. Your
application sees the coordinator's answer in its history and events like any
other reply.

## Status

`coordination::threads` returns one entry per thread, from its latest unit of
work:

| Status | When |
|---|---|
| `NeedsYou` | The thread asked a question, or its work failed. |
| `Working` | The thread is working. |
| `ReadyForReview` | The thread finished; the coordinator has not resolved it yet. |
| `Open` | The work was cancelled and the thread was left open. |
| `Resolved` | The coordinator resolved the thread. |

## Limits

- Threads run the coordinator's own agent. A local profile has no agent
  catalog, so another worker agent is refused.
- A turn your application starts on a thread directly is not settled against
  its assignment. Send follow-ups through the coordinator.
- The default caps are 8 threads working at once and 100 in total; set them
  with `max_active_threads` and `max_total_threads`.

## Example

[`examples/coordinator-agent`](https://github.com/everruns/everruns/tree/main/examples/coordinator-agent)
runs a launch coordinator with two threads that write their drafts to a shared
workspace. `--offline` runs it with a deterministic simulated model; without
it, the example uses OpenAI.

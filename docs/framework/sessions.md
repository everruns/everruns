---
title: Sessions
description: Keep conversation history across turns, read and resume Framework sessions, and choose how session state persists.
---

An `Agent` is immutable reusable behavior. An `Engine` owns session identity,
history, and runtime state. A `Session` is an engine-bound live conversation.

```rust
use everruns::{Agent, Engine, OpenAI};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Remember the conversation.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .build()?;

let engine = Engine::new();
let session = engine.create(agent);
let first = session.send_and_wait("My project is Atlas.").await?;
let second = session.send_and_wait("Continue with that project.").await?;

assert!(first.success);
assert!(second.success);
# Ok(())
# }
```

`Session` is always a live conversation. `send` accepts a message without
waiting for a response. If a turn is active, the message steers that turn; if
the previous turn has already finished, it starts a follow-up turn. The receipt
reports which case occurred, so applications do not need to race on session
state themselves:

```rust
# use everruns::{Agent, Engine, Model, SendDisposition};
# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let agent = Agent::builder().instructions("Remember input.").model(Model::simulated("Done.")).build()?;
# let engine = Engine::new();
let session = engine.create(agent);
let initial = session.send("Plan my trip.").await?;
let latest = session.send("Prefer trains.").await?;

match latest.disposition {
    SendDisposition::Steered => {
        assert_eq!(latest.turn_id, initial.turn_id);
    }
    SendDisposition::Started => {
        // The first turn completed before the second message was accepted.
    }
    _ => {}
}

let result = latest.wait().await?;
# let _ = result;
# Ok(())
# }
```

Waiting on the latest receipt works in both cases. `send_and_wait` (also
available as the shorter `run` alias) is request/response convenience over the
same live session, not a separate mode.

The first asynchronous operation materializes the in-process host. Later turns
reuse it and send accumulated history through the same context-assembly path.
Two sessions opened on one engine have different opaque IDs and isolated
histories. The engine retains each immutable Agent snapshot, so a session keeps
working after the original Agent handle is dropped. Volatile resume is
engine-scoped: another `Engine` rejects the id rather than guessing its
configuration. A local profile can be attached to another Engine with the
trusted Agent snapshot; live Engines for the same profile share its backend
bundle.

Conversation isolation does not imply filesystem isolation. The concise
`engine.create(agent)` path permanently selects the Agent's default head before its
first inspection or turn; call `session.start().await` to make that selection
observable earlier. To fix a session to an isolated project view, bind an
[`Environment`](/framework/workspaces-and-environments/) before execution. A
session can never switch heads after it starts.

`Session::inspect` returns the context assembled for the next model call. Use it
for application assertions and debugging rather than reaching into runtime
records or backend stores.

Keep `Session::session_id()` when the application may need to reopen a
conversation. [History and resume](#history-and-resume) below covers typed
resume, bounded transcript pages, and cursor snapshots, and
[Persistence](#persistence) covers engine-lifetime memory and the
crash-durable local profile. See
[Workspaces and Environments](/framework/workspaces-and-environments/) for
exact-head resume, isolation, sharing, and lifecycle, and [Events and
cancellation](/framework/events-and-cancellation/) to observe a turn in flight.

## History and resume

Every Framework session has a typed `SessionId`. Keep that value when an
application may need to reopen the conversation:

```rust
use everruns::{Agent, Engine, OpenAI, SessionId};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .instructions("Remember the conversation.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .build()?;

let engine = Engine::new();
let session = engine.create(agent);
let session_id: SessionId = session.session_id();
session.send_and_wait("My project is Atlas.").await?;

drop(session);
let resumed = engine.resume(session_id).await?;
resumed.send_and_wait("Continue with that project.").await?;
# Ok(())
# }
```

`resume` verifies the ID against the engine's session catalog. It
does not infer identity from a non-empty transcript: a valid session can have no
messages, and stray events do not create a resumable session. An unknown ID
returns a typed not-found error. The resumed session uses the immutable Agent
snapshot attached to that engine; it never reconstructs behavior from events.

### Read bounded history

`Session::history` creates an owned query. Calling `page` returns at most 100
messages by default in canonical event-sequence order, oldest first:

```rust
# use everruns::{Agent, Engine, Model};
# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let agent = Agent::builder()
#     .instructions("Be concise.")
#     .model(Model::simulated("Done."))
#     .build()?;
# let session = Engine::new().create(agent);
let page = session.history().page().await?;
for message in &page.messages {
    println!("{:?}: {}", message.role, message.text());
}
# Ok(())
# }
```

Set a smaller or larger page size with `limit`. The maximum is 256 messages;
an excessive value returns `HistoryError::InvalidLimit` with the allowed
maximum. A page never claims to contain the entire transcript. Continue from
its opaque cursor:

```rust
# use everruns::{Agent, Engine, Model};
# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let agent = Agent::builder()
#     .instructions("Be concise.")
#     .model(Model::simulated("Done."))
#     .build()?;
# let session = Engine::new().create(agent);
let first = session.history().limit(25)?.page().await?;
if let Some(cursor) = first.next_cursor {
    let second = session.history().limit(25)?.after(cursor)?.page().await?;
    // `second` continues the same stable snapshot.
}
# Ok(())
# }
```

`HistoryCursor` is opaque, session-bound, and safe to store as a string with
`Display` and restore with `FromStr`. A cursor fixes the snapshot's high-water
mark: events appended after the first page do not appear midway through that
page walk. Start a new query to see them. Passing a malformed, cross-session,
expired, or incompatible cursor returns a distinct typed history error.
History projection also applies a bounded raw-event replay safety limit; an
unusually lifecycle-heavy snapshot that exceeds it returns
`HistoryError::HistoryTooLarge` instead of performing an unbounded scan.

For callers that intentionally walk the whole snapshot, `pages` is a lazy
convenience that still reads one bounded page at a time:

```rust
# use everruns::{Agent, Engine, Model};
# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let agent = Agent::builder()
#     .instructions("Be concise.")
#     .model(Model::simulated("Done."))
#     .build()?;
# let session = Engine::new().create(agent);
let mut pages = session.history().limit(50)?.pages();
while let Some(page) = pages.next_page().await? {
    for message in page.messages {
        println!("{}", message.text());
    }
}
# Ok(())
# }
```

After the final page, `next_page` remains fused and returns `None`. It does not
re-read the backend or produce repeated empty terminal pages.

## Persistence

Framework history is a read-only projection of canonical events. Normal
execution has one write path, the engine's event log, so a resumed session and a
running session cannot disagree about the conversation.

![Framework persistence ladder: Engine::new() keeps volatile state in memory for one Engine; adding LocalConfig gives a crash-durable event log and SQLite task and schedule state for one application process; the Everruns Platform stores PostgreSQL checkpoints and canonical events across a distributed server and workers.](./persistence-ladder.svg)

| Deployment | Conversation state | Recovery boundary | Use when |
| --- | --- | --- | --- |
| `Engine::new()` | Volatile memory | One Engine in one process | Embedding, tests, and short-lived tools |
| `Engine` with `LocalConfig` | Crash-durable local canonical events and catalog | One trusted application process | Desktop apps, CLIs, and single-node services |
| Everruns Platform | PostgreSQL-backed durable workflow state and canonical events | Distributed server and workers | Restarts, retries, horizontal workers, and remote clients |

### Default: engine-lifetime memory

By default, `Engine` owns a volatile session catalog and event log. It
retains the immutable Agent snapshot associated with each session and requires
no database, server, network connection, credential, or filesystem access.

Dropping a `Session` does not immediately discard its committed history. Reopen
it by passing its typed `SessionId` to the engine that created it. A separate
engine cannot infer the session's Agent configuration, and process exit loses
volatile history.

This default fits tests, command-line tools, short-lived workers, and
applications that deliberately own a higher-level record elsewhere.

### Local: crash-durable events

For local applications, the feature-gated `LocalConfig` adds a crash-durable
event log under the configured application data directory. It also supplies a
trusted real-disk workspace plus SQLite-backed task and schedule state:

```rust
use everruns::{Agent, Engine, LocalConfig, OpenAI};

let local = LocalConfig::new(".everruns-data").workspace("./workspace");
let agent = Agent::builder()
    .instructions("Work inside the configured workspace.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .local(local)
    .build()?;
let engine = Engine::new();
let session = engine.create(agent);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Enable it with `cargo add everruns --features local`. Select both directories
from trusted application configuration. After a restart, rebuild the Agent
from trusted application configuration, attach it to a new engine, and resume
the committed session by ID. For a session created with an explicit Harness,
also deserialize its portable definition and call
`Engine::attach_with_harness`; `Engine::attach` remains the no-Harness path.

The local profile is designed for one embedded process at a time. Coordinate
process ownership before handing the directory to another application process.
Within one process, every live Engine configured with the same local data
directory shares one backend bundle, so concurrent Engine values cannot build
divergent JSONL indexes or SQLite handles for that profile.

The event-log file format and host backends are not Framework APIs. Do not edit
the log or build application writes around its representation. Use
`Session::history` for bounded reads and `Engine::resume` to continue a session;
see [History and resume](#history-and-resume) for the complete lifecycle.

Applications remain responsible for filesystem permissions, backups, retention,
and selecting a data directory that is not controlled by model or request
input. New local state files are created owner-only on Unix, but applications
must still protect copied files and backups. Message content is application data
and may be sensitive even though provider credentials are not written there by
Framework configuration.

### Resume after a process restart

Enable `local` and configure a trusted application data directory when sessions
must survive a new Agent or process:

```rust
use everruns::{Agent, Engine, LocalConfig, OpenAI};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
let build_agent = || -> Result<Agent, Box<dyn std::error::Error>> {
    Ok(Agent::builder()
        .instructions("Remember the conversation.")
        .provider(OpenAI::from_env()?)
        .model("gpt-5.6-terra")
        .local(LocalConfig::new(".everruns-data"))
        .build()?)
};

let first_engine = Engine::new();
let session = first_engine.create(build_agent()?);
session.start().await?;
let session_id = session.session_id();

// In a later process, rebuild trusted behavior before resuming persisted state.
let restarted_engine = Engine::new();
restarted_engine.attach(session_id, build_agent()?).await?;
let resumed = restarted_engine.resume(session_id).await?;
# Ok::<(), Box<dyn std::error::Error>>(())
# }
```

The local profile stores a durable session catalog and crash-durable canonical
event log alongside its workspace, task, and schedule state. After restarting,
build another Agent with the same trusted data directory, call
`engine.attach(session_id, agent)`, then `engine.resume(session_id)`. Attachment
rejects IDs absent from that Agent's configured local catalog. A new session is
made durable by its first async operation (`run`, `inspect`, or a history page
read); merely allocating a synchronous handle does not commit it.

A running turn, or one waiting on a person (a tool approval or an `ask_user`
question), lives inside the process, so a process exit leaves it in the log
without an end. After resuming, `resumed.interrupted_turn()` reports such a
turn and the tool calls it had not finished, and
`resumed.resume_interrupted_turn()` continues it in the same turn. A call runs
again only when that is safe: it waited on a person, which asks the approver
or `ask_user` responder again under the same tool call id, or its tool is
marked `FunctionTool::idempotent()`. Any other unfinished call is recorded as
interrupted (listed in `not_rerun`), so the model learns its outcome is
unknown and nothing runs twice. A turn cut off between steps reasons again.

For a session created with an explicit Harness, persist its serialized portable
definition, deserialize it after restart, and call
`engine.attach_with_harness(session_id, agent, harness)` instead. Harness
deserialization validates the definition and generates a new process-local
runtime identity.

The local profile is for one embedded process at a time. Do not write or edit
its files as application data: messages are a read-only projection of committed
events, and the storage formats are not Framework APIs.

History does not contain model credentials or application secrets unless an
application deliberately includes them in message content or event metadata.
Choose and protect the local data directory accordingly.

### Canonical host persistence

Durable conversation truth belongs to canonical events; history and context
are projections of that record. Advanced hosts use `EventLog` and
`EventHistory` from `everruns-core` (`host` feature), including `JsonlEventLog` when a local
append-only event log is appropriate. Framework applications continue sessions
with `Engine::resume` and traverse bounded event-derived pages from
`Session::history`.

A host that needs its own storage implements the public `EventLog`/`EventReader`
SPI and supplies it through `HostBackends::with_event_log`; see
[Implementing a custom event log](/framework/events-and-cancellation/#implementing-a-custom-event-log).

`JsonlEventLog` bounds startup recovery before indexing: the default accepts at
most 128 MiB and 1,000,000 canonical events. Oversize logs fail to open with a
typed recovery-limit error instead of allocating or scanning without bound.

Do not design new application persistence around a legacy storage
representation.

### Durable turns (experimental)

Persistence decides what survives; the execution backend decides how a turn
runs. By default a turn runs in process, on the task that awaits it. The
experimental `durable` Cargo feature adds `everruns::durable::Backend`, which
runs every turn step (input, reason, act) as a task on a durable queue and
checkpoints the turn's state after each step. A pool of workers in your
process runs the steps on the session's own runtime, so answers, steering,
cancellation, and the session's event sequence match the in-process backend.

```rust
use everruns::{Agent, Engine, Model, durable};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
let engine = Engine::builder()
    .backend(durable::Backend::memory().workers(4))
    .build();
let agent = Agent::builder()
    .instructions("You are concise.")
    .model(Model::simulated("4"))
    .build()?;

let turn = engine.create(agent).send_and_wait("What is 2 + 2?").await?;
assert_eq!(turn.response, "4");
# Ok(())
# }
```

- `Backend::memory()` keeps the queue in memory for as long as the engine
  lives. Nothing beyond the session's own event log survives the process.
- `Backend::postgres(store)` keeps the queue in PostgreSQL, which several
  processes may share; each engine claims only its own sessions' steps.
  `everruns::durable::PostgresWorkflowEventStore::connect(url)` connects and
  applies the durable schema, which is safe on every start-up. When a session
  is opened again after a process exit, its first turn ends the turn the old
  process left running, and `Session::resume_interrupted_turn` continues a turn
  cut off in its tool calls from the session log. That requires a session log
  that also outlives the process, such as the [local profile](#local-crash-durable-events).

The durable backend and the `TurnBackend` trait it implements are outside the
Framework's API stability promises and may change between releases. Each
durable step adds a few milliseconds over in process, and a round trip per
store write on PostgreSQL.

### Platform: distributed durable execution

The Everruns Platform uses the same `everruns-core` (`engine` feature) turn state machine as the
Framework and the same durable turn driver as the `durable` feature, from
`everruns-durable-engine` on the `everruns-durable` engine. The server schedules work,
workers claim each step over gRPC, execute it, and apply its effects, and PostgreSQL stores workflow
checkpoints and canonical events. A worker can disappear between phases and a
later worker can continue from the committed checkpoint.

The Platform is a deployment boundary, not another configuration mode on
`everruns::Engine`. Remote applications use the Platform API or an SDK; product
hosts compose the lower-level durable crates. See [Framework
Architecture](/framework/architecture/) for the layer map.

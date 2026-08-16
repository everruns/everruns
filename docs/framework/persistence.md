---
title: Persistence
description: Choose volatile memory, crash-durable local state, or the distributed durable Platform.
---

Framework history is a read-only projection of canonical events. Normal
execution has one write path—the engine's event log—so a resumed session and a
running session cannot disagree about the conversation.

| Deployment | Conversation state | Recovery boundary | Use when |
| --- | --- | --- | --- |
| `Engine::new()` | Volatile memory | One Engine in one process | Embedding, tests, and short-lived tools |
| `Engine` with `LocalConfig` | Crash-durable local canonical events and catalog | One trusted application process | Desktop apps, CLIs, and single-node services |
| Everruns Platform | PostgreSQL-backed durable workflow state and canonical events | Distributed server and workers | Restarts, retries, horizontal workers, and remote clients |

## Default: engine-lifetime memory

By default, `Engine` owns a volatile session catalog and event log. It
retains the immutable Agent snapshot associated with each session and requires
no database, server, network connection, credential, or filesystem access.

Dropping a `Session` does not immediately discard its committed history. Reopen
it by passing its typed `SessionId` to the engine that created it. A separate
engine cannot infer the session's Agent configuration, and process exit loses
volatile history.

This default fits tests, command-line tools, short-lived workers, and
applications that deliberately own a higher-level record elsewhere.

## Local: crash-durable events

For local applications, the feature-gated `LocalConfig` adds a crash-durable
event log under the configured application data directory. It also supplies a
trusted real-disk workspace plus SQLite-backed task and schedule state:

```rust
use everruns::{Agent, Engine, LocalConfig, Model};

let local = LocalConfig::new(".everruns-data").workspace("./workspace");
let agent = Agent::builder()
    .instructions("Work inside the configured workspace.")
    .model(Model::simulated("Ready."))
    .local(local)
    .build()?;
let engine = Engine::new();
let session = engine.create(agent);
# Ok::<(), everruns::BuildError>(())
```

Enable it with `cargo add everruns --features local`. Select both directories
from trusted application configuration. After a restart, rebuild the Agent
from trusted application configuration, attach it to a new engine, and resume
the committed session by ID.

The local profile is designed for one embedded process at a time. Coordinate
process ownership before handing the directory to another application process.
Within one process, every live Engine configured with the same local data
directory shares one backend bundle, so concurrent Engine values cannot build
divergent JSONL indexes or SQLite handles for that profile.

The event-log file format and host backends are not Framework APIs. Do not edit
the log or build application writes around its representation. Use
`Session::history` for bounded reads and `Engine::resume` to continue a session;
see [Session History and Resume](/framework/session-history/) for the complete
lifecycle.

Applications remain responsible for filesystem permissions, backups, retention,
and selecting a data directory that is not controlled by model or request
input. New local state files are created owner-only on Unix, but applications
must still protect copied files and backups. Message content is application data
and may be sensitive even though provider credentials are not written there by
Framework configuration.

## Canonical host persistence

Durable conversation truth belongs to canonical events; history and context
are projections of that record. Advanced hosts use `EventLog` and
`EventHistory` from `everruns-host`, including `JsonlEventLog` when a local
append-only event log is appropriate. Framework applications continue sessions
with `Engine::resume` and traverse bounded event-derived pages from
`Session::history`.

A host that needs its own storage implements the public `EventLog`/`EventReader`
SPI and supplies it through `HostBackends::with_event_log`; see
[Implementing a custom event log](/framework/canonical-events/#implementing-a-custom-event-log).

`JsonlEventLog` bounds startup recovery before indexing: the default accepts at
most 128 MiB and 1,000,000 canonical events. Oversize logs fail to open with a
typed recovery-limit error instead of allocating or scanning without bound.

Do not design new application persistence around a legacy storage
representation.

## Platform: distributed durable execution

The Everruns Platform uses the same `everruns-engine` turn state machine as the
Framework, but adapts it through `everruns-durable`. The server schedules work,
workers execute phases and apply effects, and PostgreSQL stores workflow
checkpoints and canonical events. A worker can disappear between phases and a
later worker can continue from the committed checkpoint.

This is a deployment boundary, not another configuration mode on
`everruns::Engine`. Remote applications use the Platform API or an SDK; product
hosts compose the lower-level durable crates. See [Framework
Architecture](/framework/architecture/) for the layer map.

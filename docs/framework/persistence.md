---
title: Persistence
description: Choose volatile or crash-durable Framework session and local application state.
---

Framework history is a read-only projection of canonical events. Normal
execution has one write path—the Agent's event log—so a resumed session and a
running session cannot disagree about the conversation.

## Default: Agent-lifetime memory

By default, each built `Agent` owns a volatile in-memory event log. It is fully
offline and requires no database, server, network connection, credential, or
filesystem access.

Dropping a `Session` does not immediately discard its committed history. Reopen
it by passing its typed `SessionId` to the Agent that issued it, or to a clone of
that Agent. A separately built Agent has a separate in-memory store, and process
exit loses volatile history.

This default fits tests, command-line tools, short-lived workers, and
applications that deliberately own a higher-level record elsewhere.

## Local: crash-durable events

For local applications, the feature-gated `LocalConfig` adds a crash-durable
event log under the configured application data directory. It also supplies a
trusted real-disk workspace plus SQLite-backed task and schedule state:

```rust
use everruns::{Agent, LocalConfig, Model};

let local = LocalConfig::new(".everruns-data").workspace("./workspace");
let agent = Agent::builder()
    .instructions("Work inside the configured workspace.")
    .model(Model::simulated("Ready."))
    .local(local)
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

Enable it with `cargo add everruns --features local`. Select both directories
from trusted application configuration. Another Agent—or a later process—can
open the same data directory and resume a committed session by ID.

The local profile is designed for one embedded process at a time. Coordinate
process ownership before handing the directory to another Agent process.

The event-log file format and host backends are not Framework APIs. Do not edit
the log or build application writes around its representation. Use
`Session::history` for bounded reads and `Agent::resume` to continue a session;
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
with `Agent::resume` and traverse bounded event-derived pages from
`Session::history`.

A host that needs its own storage implements the public `EventLog`/`EventReader`
SPI and supplies it through `HostBackends::with_event_log`; see
[Implementing a custom event log](/framework/canonical-events/#implementing-a-custom-event-log).

Do not design new application persistence around a legacy storage
representation.

For multi-process execution, use the reference-only
[`ScaleEngine`](/framework/scalable-engines/). It owns PostgreSQL canonical
events and exact Environment bindings rather than serializing an arbitrary
in-process Agent.

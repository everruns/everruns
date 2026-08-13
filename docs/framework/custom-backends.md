---
title: Custom Backends
description: Decide when a Framework application should cross into low-level execution-host composition.
---

Most applications should use `everruns::Agent`, `Model`, and `Session`. That
surface deliberately hides stored harness records, platform registries, backend
stores, worker phases, and durable scheduling topology.

Cross into low-level composition only when your application is itself an
execution host—for example, a server, evaluation harness, research runtime, or
specialized embedder that must replace storage or orchestration components.

## Host-level choices

The low-level crates expose focused contracts for:

- core agent, event, capability, and provider values;
- the sans-I/O turn engine;
- runtime host phases, canonical event history, and in-memory reference stores;
- local SQLite-backed task and schedule state;
- platform/control-plane entities and durable deployment components.

An advanced host depends on `everruns` plus `everruns-host` and the focused
crates it actually needs. `everruns-host` is the only low-level host boundary:
there is no separate runtime crate. It is healthy for such a host to use
low-level extension traits; the goal is not to re-export every backend through
one facade.

Conversation persistence is the one backend with a single write path. Replace it
by implementing the canonical `EventLog`/`EventReader` SPI and passing it to
`HostBackends::with_event_log`; the required snapshot, continuation, and polling
behavior is specified in
[Implementing a custom event log](/framework/canonical-events/#implementing-a-custom-event-log).

## Security boundary

Backend replacement does not relax tenant, credential, filesystem, or tool
execution boundaries. Preserve event ordering, credential redaction, workspace
containment, and cancellation behavior when adapting the host. A custom backend
must fail explicitly when it cannot satisfy a required contract.

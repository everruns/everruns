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
- runtime host phases and in-memory reference stores;
- local SQLite-backed task and schedule state;
- platform/control-plane entities and durable deployment components.

During 0.17.x, `everruns-runtime` remains the supported compatibility crate for
existing `InProcessRuntimeBuilder` and `RuntimeBackends` users. New ordinary
applications should not reproduce that topology merely to run a session.

An advanced host may depend on `everruns` plus the focused host crates it
actually needs. It is healthy for such a host to use low-level extension traits;
the goal is not to re-export every backend through one facade. See the
[`everruns-runtime` API reference](https://docs.rs/everruns-runtime) and
[Runtime compatibility](/framework/runtime-compatibility/) for that transitional path.

## Security boundary

Backend replacement does not relax tenant, credential, filesystem, or tool
execution boundaries. Preserve event ordering, credential redaction, workspace
containment, and cancellation behavior when adapting the host. A custom backend
must fail explicitly when it cannot satisfy a required contract.

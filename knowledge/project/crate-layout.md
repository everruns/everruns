---
type: Specification
title: "Crate Layout"
description: "Current boundaries and dependency rules for the Rust workspace's crates."
tags:
  - everruns
  - project
  - crates
  - release
---
# Crate Layout

## Abstract

The Rust workspace is organized around application, entry, building-block, and
foundation crates. A crate exists for an application contract, a kernel firewall, a
process boundary, or a selectable integration, and for nothing else. This follows the
boundary test in [Code Organization](../foundations/code-organization.md#crate-dependency-conventions).

This document describes the current layout and the dependency rules that preserve it.
Release-by-release migration history belongs in the changelog.

## Boundary Invariants

- Control-plane records (agent, harness, session rows, organizations, billing, audit)
  exist only in `crates/server/`.
- Only the server, `everruns-durable`, and the `everruns` facade open application
  database connections. `everruns-pg-embedded` may create and drop isolated DEV_MODE
  and test databases, but does not read application tables.
- A library host (yolop, `everruns-serve`, a user's app) builds against `everruns` or
  `everruns-core` plus `everruns-contracts`, and never sees a control-plane type.
- Integrations implement contracts and do not depend on core or platform crates.

## Crate Layout

Four layers. A crate depends only on crates in lower layers or beside it in its own
layer, never upward.

| Layer | Crate | Owns | Published |
|---|---|---|---|
| Apps and hosts | `everruns-server` | management API, business logic, control-plane records, the control-plane database | no |
| | `everruns-worker` | the execution process, orchestrated by the server | no |
| | `everruns-serve` (+ `-macros`, `-build`, `-agentcore`, `-celld`) | serving one app as a service | yes |
| | yolop | terminal coding agent, own repository | own repo |
| Entry crates | `everruns` | the Framework facade; its features pick drivers, integrations, and capabilities, and it owns the default "batteries" wiring | yes |
| | `everruns-durable-engine` | the durable execution backend: runs core turns as queued, checkpointed steps behind core's `TurnBackend`; the worker's only path into core | yes (experimental) |
| Building blocks | `everruns-core` | engine, host runtime, builtins, MCP, A2A, AG-UI | yes |
| | `everruns-capabilities` | hosted capabilities (subagents, session tasks, background runs, knowledge, container sandbox) | yes |
| | `everruns-drivers` (+ `everruns-llmsim`) | model drivers, one feature per vendor | yes |
| `crates/integrations/` | `everruns-integrations` | maintained integrations, selected by feature | yes |
| `crates/integrations-experimental/` | `everruns-integrations-experimental` | Deno and Sprites, selected by feature | yes |
| | `everruns-durable` | durable workflow primitives and their own Postgres store | yes |
| Foundation | `everruns-contracts` | provider and capability SPIs, model profiles, typed ids, runtime view types, connector, store, sandbox, and vector-store traits; the runtime SPI (capability, tool, tool context, session, message, event) behind its `runtime` feature | yes |

The supporting crates keep their boundaries: `everruns-macros` and the serve macros
(proc-macro crates must stand alone), `everruns-cli` and `everruns-cli-contract`,
`everruns-test-support`, `everruns-turbopuffer`, and `everruns-ard`.

Hosted integration composition belongs to `everruns-capabilities`, behind its
opt-in `hosted-integration-catalog` feature. The retired
`everruns-integrations-catalog` package published its final deprecated forwarding
release at 0.45.0; its source is removed in 0.46. Published versions remain usable.
See
[Hosted Integration Composition](../foundations/architecture.md#hosted-integration-composition).

`everruns-contracts` is what an extension author depends on. A driver, an integration,
or a store backend implements a contract trait and needs nothing else. Its typed ids
keep the optional `sqlx` feature so the server can store them directly; the orphan rule
allows that impl only in the crate that defines the ids. The feature is off by default.

## Rules

### Records stay in the server

A control-plane record carries ownership, versions, and API exposure: what an
organization stored, not what a turn needs. The server owns these records and maps them
to contract types at its edge. A library crate sees only the runtime view: ids,
`ExecutionSession`, the portable agent and harness definitions, and store traits phrased
in those types.

`PlatformStore` ([source](../../crates/capabilities/src/platform_store.rs)) now
returns portable execution views. The server resolves ownership, inheritance,
versions and lifecycle at its edge. All persistence/API aggregates, including
provider, model, virtual-user, skill, model-router, and MCP-server rows, live in [`server records`](../../crates/server/src/records/mod.rs).
The worker receives portable definitions and neutral dependency blockers.

[`check-agent-record-isolation.sh`](../../scripts/lib/check-agent-record-isolation.sh)
derives the record vocabulary from the server owner and rejects declarations
or imports elsewhere, including private worker code. Published crates cannot
depend on the server. Negative fixtures prove each boundary.

### Only the server, durable, and the facade touch a database

The server owns the control-plane database. `everruns-durable` owns its workflow state
(task queue, journal, timers) with its own schema and migrate. They may share one
Postgres instance, but neither reads the other's tables except through the other's API.
The worker reaches durable's store only through `everruns-durable-engine`.
[`check-database-driver-isolation.sh`](../../scripts/lib/check-database-driver-isolation.sh)
rejects normal and build dependencies on database drivers outside these owners,
including optional and renamed edges. Contracts' optional typed-id codecs are the
only exception and cannot construct a connection. `everruns-pg-embedded`, the
server's throwaway DEV_MODE and test cluster, may connect to create and drop databases;
it reads no tables. Embedded hosts (the facade's `local` feature, the serve
hosts) keep their own schemas and shared query callbacks, but open SQLite handles
through `everruns::sqlite`, behind the facade's `local` feature. It used to be
durable's `sqlite` module; owning it in durable made the facade's `local` feature
compile the durable engine just to open SQLite. A separate leaf crate for it was
tried and dropped: about 200 lines of utility code is not worth a published crate,
and the serve hosts already depend on the facade. `UpdateField` went back to
durable; server storage, which depends on durable anyway, uses it from there.
[`check-durable-isolation.sh`](../../scripts/lib/check-durable-isolation.sh) also keeps
durable generic (it has no `everruns-*` normal dependency)
and prevents worker dependencies from bypassing durable-engine.

### Turns run through one backend seam

Core's host owns `TurnBackend`
([source](../../crates/core/src/host/turn_backend.rs)), the one interface through
which a host starts, cancels, and observes a session's turns. The facade's session
actor runs every turn through it, on the in-process default today.
`everruns-durable-engine` is the durable implementation: published so an
application can choose it from the facade, it must therefore carry nothing
private to the platform. The gRPC stores and runner constructors, `tonic`, and
`everruns-internal-protocol` therefore live in the worker, which implements
durable-engine's store traits for its own gRPC client and builds `DurableRunner`
through the engine's transport-neutral `DurableRunner::from_store`;
[`check-durable-isolation.sh`](../../scripts/lib/check-durable-isolation.sh) rejects
`tonic` or the internal protocol in durable-engine's normal and build edges. Worker-only
core features (MCP, telemetry, OpenAI Agents API, tree-sitter outlines) are selected
by the worker's own `everruns-core` edge, which the guard allows for feature selection
only. The worker runs the shared turn driver (`TurnTaskDriver`) over its gRPC store. Why queue plus per-step checkpoint rather than
`Workflow` replay is recorded in [Execution Backends](../framework/execution-backends.md).

### Core stays wasm-safe by default

Folding the engine, host, builtins, and protocol crates into `everruns-core` must not
erase the kernel firewall that [WASM Kernel](../framework/wasm-kernel.md) and
[`check-core-kernel-dependencies.sh`](../../scripts/lib/check-core-kernel-dependencies.sh)
protect. Effects (HTTP, process, filesystem, MCP stdio, gRPC) sit behind features. The
default build stays wasm-safe, and the guard checks the default feature set rather than
the whole crate.

### Integrations depend on contracts, never on core

An integration implements the runtime SPI in `everruns_contracts::runtime`
([source](../../crates/contracts/src/runtime/mod.rs)) and ships against nothing else.
Core re-exports each runtime module at its old `everruns_core::` path, so hosts and
capabilities keep importing it from core. The `runtime` feature keeps provider-only
builds free of the SPI's dependencies. Tests may still take core as a dev-dependency
to drive a real host.
[`check-provider-isolation.sh`](../../scripts/lib/check-provider-isolation.sh) rejects
any normal or build edge from an integration to `everruns-core`, transitive ones
included.

## Rejected Options

- **One crate for everything.** It erases the kernel firewall, the driver isolation
  that keeps provider-only builds free of core, and the process boundary between
  server and worker.
- **An `everruns-protocols` crate for MCP, A2A, and AG-UI.** It groups crates by
  topic rather than by boundary. These protocols are runtime features of core, not
  something a separate consumer picks up alone.
- **Renaming `everruns-platform` and keeping its contents.** It keeps records next to
  library code, which is the leak this layout exists to remove.
- **Shims for two releases.** One release is enough for crates.io and docs.rs to show
  the move and for the compiler to warn dependents.

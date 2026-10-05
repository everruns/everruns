---
type: Specification
title: "Crate Layout"
description: "Target layout of the Rust workspace's crates, the rules that keep it, and the release-by-release plan that gets there from today's 52 published crates."
tags:
  - everruns
  - project
  - crates
  - release
---
# Crate Layout

## Abstract

Everruns publishes 52 crates at one platform version. Every one of them is a slot the
Crate Release cascade has to fill, and a failed slot burns the version for that crate,
which is how one day produced three sequential releases (0.34.0 to 0.34.2). Several of
those crates exist for history, not for a boundary: they forward another owner's code,
split one SPI across packages, or mix control-plane records into a library a host like
yolop consumes.

This concept fixes the target layout, the dependency rules that keep it, and the order
of the moves. The boundary test is the one [Code Organization](../foundations/code-organization.md#crate-dependency-conventions)
already sets: a crate exists for an application contract, a kernel firewall, a process
boundary, or a selectable integration, and for nothing else.

## Success Bars

- Published crates drop from 52 to about 38, with no capability removed.
- Control-plane records (agent, harness, session rows, organizations, billing, audit)
  exist only in `crates/server/`.
- Only the server and `everruns-durable` open a database connection.
- A library host (yolop, `everruns-serve`, a user's app) builds against `everruns` or
  `everruns-core` plus `everruns-contracts`, and never sees a control-plane type.
- Each removed crate ships one last release as a deprecated shim, then is deleted.

## Target Layout

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
| | `everruns-integrations-*` | one external service each | yes |
| | `everruns-durable` | durable workflow primitives and their own Postgres store | yes |
| Foundation | `everruns-contracts` | provider and capability SPIs, model profiles, typed ids, runtime view types, connector, store, sandbox, and vector-store traits; the runtime SPI (capability, tool, tool context, session, message, event) behind its `runtime` feature | yes |

The supporting crates keep their boundaries: `everruns-macros` and the serve macros
(proc-macro crates must stand alone), `everruns-cli` and `everruns-cli-contract`,
`everruns-test-support`, `everruns-turbopuffer`, `everruns-ard`, and
`everruns-integrations-catalog`.

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

### Only the server and durable touch a database

The server owns the control-plane database. `everruns-durable` owns its workflow state
(task queue, journal, timers) with its own schema and migrate. They may share one
Postgres instance, but neither reads the other's tables except through the other's API.
The worker reaches durable's store only through `everruns-durable-engine`.
[`check-database-driver-isolation.sh`](../../scripts/lib/check-database-driver-isolation.sh)
rejects normal and build dependencies on database drivers outside these two owners,
including optional and renamed edges. Contracts' optional typed-id codecs are the
only exception and cannot construct a connection. Embedded hosts keep their own
schemas and shared query callbacks, but open SQLite handles through durable's API.
[`check-durable-isolation.sh`](../../scripts/lib/check-durable-isolation.sh) also keeps
durable generic and prevents worker dependencies from bypassing durable-engine.

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
only. The worker next runs the shared turn driver over its gRPC store. Why queue plus per-step checkpoint rather than
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

### Integrations never depend on core's host

Today `everruns-host` depends on five integrations (bashkit, filesystem, lua, web-fetch,
duckduckgo) and on the drivers, while those integrations depend on `everruns-core`. If
the host folds into core unchanged, that is a cycle. The default integration and driver
wiring therefore moves up into the `everruns` facade. Core's host takes integrations
and drivers through contract traits and registries, and ships with none attached.

## Current to Target

| Today | Target | How |
|---|---|---|
| `everruns-anthropic`, `-bedrock`, `-fireworks`, `-gemini`, `-mai`, `-meta`, `-openai`, `-openrouter` | `everruns-drivers` features | drivers merged (#4035); shims retired after their 0.35 release |
| `everruns-provider`, `everruns-capability`, `everruns-model-profiles` | `everruns-contracts` | merge, shim |
| `everruns-platform` connector, session sandbox, vector store, knowledge store, SQL database, sandbox checkpoint traits | `everruns-contracts` | move |
| `everruns-platform` capabilities and container sandbox | `everruns-capabilities` | rename, shim `everruns-platform` |
| `everruns-platform` agent, harness, session, org, app, audit, payment, reporting, email, Slack, feature flags, eval, budget, triggers | `crates/server/` | move; neutral Slack action identity lives in contracts and is re-exported by internal protocol (avoids a published-to-private dependency) |
| `everruns-engine`, `everruns-host`, `everruns-builtins`, `everruns-mcp`, `everruns-ag-ui` | `everruns-core` features | merge, shim |
| A2A protocol client inside `everruns-platform`'s `a2a_delegation` capability | `everruns-core` `a2a` feature, beside MCP | move; the delegation capability stays in `everruns-capabilities` and calls it |
| the worker's direct core, engine, and durable wiring | `everruns-durable-engine` | new; published as the experimental durable backend |

## Migration Order

One step per platform release, because a removed crate needs one shim release before
it can be deleted (see [Release Process](release-process.md)). Steps that remove no
published name can land in any release.

1. **Delete the driver shims.** After 0.35 ships the eight shims, delete them.
2. **Create `everruns-contracts`.** Merge provider, capability, and model profiles.
   Move the connector and store traits out of platform, which frees every integration,
   `everruns-ard`, and `everruns-turbopuffer` from depending on platform. Shim the three
   merged names.
3. **Phrase `PlatformStore` in runtime terms.** No crate is renamed. The hosted
   capabilities stop seeing records, and yolop can drop its subagents override.
   [`PlatformStore`](../../crates/capabilities/src/platform_store.rs) reuses the
   existing portable definitions, resolved harness configuration, and
   `ExecutionSession`; server command adapters own record projection and
   authorization. The [external runtime host fixture](../../crates/everruns/tests/fixtures/external-consumer/platform-store/src/lib.rs)
   executes the stock subagents capability through that seam.
4. **Split platform.** Records go to the server, and the rest becomes
   `everruns-capabilities`. Shim `everruns-platform`, then widen the record guard.
5. **Fold the kernel into core.** Engine, host, builtins, MCP, and AG-UI become core
   modules behind features. The batteries move to the facade. Shim the five names and
   make the kernel guard feature-aware.
6. **Introduce `everruns-durable-engine`.** It carries the worker's turn-on-workflow
   wiring, and the worker depends on it instead of core and durable directly. Add the
   database guard. Delete the five core shims only after their final platform
   release is fully published; canonical feature modules retain every capability.
7. **Migrate yolop in one batch** onto `everruns-contracts`, `everruns-core`, and
   `everruns-capabilities`. Yolop is pinned to 0.33.0 and moves once, not per step.
8. **Publish the durable backend.** Add the `TurnBackend` seam to core and route the
   facade through its in-process default. Move the gRPC stores and constructors out of
   `everruns-durable-engine` into the worker and ban `tonic` and the internal protocol
   there; move the worker's turn driver in; implement `TurnBackend` on the durable
   runner; then add the crate to the publish set and the facade's `durable` feature.
   The crate is in the publish set from the release after 0.41.0, its first crates.io
   name; the facade feature is still to come.

The isolation guards in [`scripts/lib/`](../../scripts/lib/) name today's crates, so
each step updates the ones it touches in the same change: provider isolation (step 2),
hosted-capability and agent-record isolation (step 4), core-kernel, portable-builtins,
and environment-capability isolation (step 5), and durable isolation (step 6). A guard
is re-pointed at the new owner, never deleted.

Each step keeps the release preflight
([`scripts/release-preflight.py`](../../scripts/release-preflight.py)) and the publish
set ([`.github/crates-publish-set.txt`](../../.github/crates-publish-set.txt)) in step:
a shim stays in the set for its last release, and leaves it when deleted.

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

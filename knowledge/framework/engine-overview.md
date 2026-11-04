---
type: Concept
title: "How Sessions Run: a One-Page Overview"
description: "Plain-language map of who runs an agent session, where its state lives and what survives a crash, with links to the detailed decisions."
tags:
  - everruns
  - framework
  - durable
  - overview
---

# How Sessions Run: a One-Page Overview

One piece of Rust runs an agent session, and every place that runs agents uses
it: the platform, the Framework you embed, `serve`, and Cloudflare or celld
deployments. What changes between those places is only where the session is
stored and who wakes it up. The detailed decisions are linked at the end; this
page is the map.

## Three layers

| Layer | What it is | Who writes it |
| --- | --- | --- |
| Agent | Instructions, model, tools, hooks. Plain data plus closures. | The application |
| Engine | Runs the agent: the turn loop, approvals, cancellation, hooks, crash recovery and the session commands that check them. | Everruns, once, in Rust |
| Hosting | Stores the session and wakes it up. Never decides how the agent behaves. | Everruns or a third party |

The same Agent code works with every hosting choice. Hosting is chosen when the
[Engine](../../crates/everruns/src/engine.rs) is built, not inside the Agent.

## The session actor

Each session has exactly one owner at a time. The owner holds a short lease,
runs turns, appends events to the session's log and renews the lease. If the
owner dies, the lease runs out, the next owner reads the log and continues.
Every write carries the owner's epoch, so a stale owner that wakes up late is
refused. This driver lives in
[core](../../crates/core/src/host/session_actor/mod.rs) and is shared by every
host.

## Where a session can live

| Hosting | Survives a restart | Typical use |
| --- | --- | --- |
| Memory (Engine default) | No | Tests, scripts, short-lived apps |
| SQLite on disk | Yes, on one machine | `serve dev`/`start`, desktop and CLI apps |
| Durable Object | Yes, managed by Cloudflare | Engine compiled to Wasm inside the object, or a native executor driven by it |
| celld | Yes, replicated fleet | Self-hosted object fleet |
| PostgreSQL (platform) | Yes | app.everruns.com; uses the same core driver directly |
| Your own | Your choice | Implement the [hosting SPI](../../crates/everruns/src/hosting/spi.rs) and pass the [conformance suite](../../crates/everruns/src/hosting/conformance.rs) |

## What a durable host promises

- A message is saved together with its wake-up before the caller hears
  "accepted". Retrying with the same id returns the same receipt.
- If the process running a turn is killed, the host wakes the session by itself
  and finishes the original turn. No client has to resend anything.
- A tool that had started and might have caused a side effect is reported as
  interrupted, not silently run again. There is no exactly-once promise for
  external effects; tools carry their own idempotency.
- A session stays pinned to the agent build it started with. If that build is
  missing, the session pauses visibly instead of running newer code.
- The journal is durable; workspaces, secrets, channels and schedules are
  separate services each host declares or refuses up front.

## The old Framework path

`AgentBuilder::local` and `AgentBuilder::backends` select the older facade
lane, which keeps a weaker recovery boundary. Both are deprecated. Existing
sessions move to SQLite hosting with `Engine::cut_over_legacy_session`; the
lane is removed once the [retirement gate](local-authority-cutover.md) passes.

## How it is tested

Storage rules run as one conformance suite on memory, SQLite and workerd. Crash
recovery is proven by killing real processes (workerd, Docker containers, a
three-node celld fleet, the Cloudflare service) and requiring completion with
no client traffic. The
[recovery workflow](../../.github/workflows/durable-object.yml) runs on related
changes and nightly, and the nightly run includes the real Cloudflare service.
The platform has its own [chaos harness](../operations/load-testing.md).

## Read next

- [Engine-owned durable sessions](engine-durable-sessions.md): the boundary and who owns what.
- [Execution backends](execution-backends.md): hosting profiles and remaining internal turn backends.
- [Durable Object authority](durable-object-native.md): Cloudflare and celld guarantees and test limits.
- [Durable execution engine operations](../operations/durable-execution-engine.md): platform actors, leases and watchdogs.

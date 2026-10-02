---
type: Decision
title: "The Execution Kernel in a JavaScript Isolate"
description: "Why everruns-provider, -core and -engine build for wasm32-unknown-unknown, how time and task spawning port, and why the celld engine cell commits one engine step at a time."
tags:
  - everruns
  - framework
  - wasm
  - celld
  - experimental
---

# The Execution Kernel in a JavaScript Isolate

**Status: experimental.** The kernel crates build for
`wasm32-unknown-unknown` with default features off, and
`examples/celld-engine` runs the engine inside a celld Durable Object. No
crate promises wasm support yet; the CI guard
(`scripts/lib/check-wasm-portability.sh`) keeps the build from regressing.

## Problem

A Durable Object platform (celld, Cloudflare) gives each object a single
owner, replicated SQLite and durable alarms: a natural home for one agent
session. Hosting `serve` next to the object in a container
([serve](serve.md)) keeps durability at turn granularity, because the object
can only replay whole requests into a process it cannot see inside. Running
the engine in the object itself lets every engine event land in the object's
own storage.

## Decisions

- **Port the kernel, not the host.** Only `everruns-provider`,
  `everruns-core` and `everruns-engine` (no default features) build for
  wasm32. The host, stores, HTTP drivers and the server stay native. The
  engine's `tokio` dependency is narrowed to `rt`, `sync`, `time` and
  `macros`; `getrandom` and `uuid` take their JavaScript backends on that
  target only.
- **Time and tasks go through `everruns_provider::rt`.** On native targets
  it re-exports Tokio's types unchanged, so paused-clock tests keep working.
  On wasm32 it uses the host clock (`web-time`), `setTimeout`
  (`gloo-timers`) and the microtask queue (`wasm-bindgen-futures`).
  `std::time::Instant::now` panics in an isolate, so kernel code uses
  `web_time::Instant` instead.
- **`Send` contracts stay.** Engine traits require `Send` so native hosts can
  use the multi-thread runtime. JavaScript futures are `!Send`; on the
  single-threaded wasm target, `rt::AssertSend` (and `worker::send` in the
  example) marks them `Send` there and only there.
- **The cell commits per engine step, not per event.** One step is one atom:
  a Reason (one model call) or an Act (one tool batch). The events a step
  emits are written uncommitted, and the step commits them, the next turn
  position and the consumed inbox entry in one storage transaction. A lost
  step is discarded and re-run; a committed one never repeats. That bounds
  the cost of losing a node at one model call or one tool batch, against the
  whole turn for the container shape.
- **The alarm drives turns.** A request only queues a message and arms the
  alarm, and each step re-arms it one lease ahead, so a turn resumes on
  whichever node owns the object next with no client involvement.
- **Messages wait in an inbox.** A message posted mid-turn enters the event
  log only when its turn opens, so it never lands between a tool call and its
  result.

## Open

- A tool with side effects outside the cell can run twice if the node is
  lost after it ran and before its step committed. Idempotency keys per tool
  call would close that.
- The cell's model driver is a non-streaming Chat Completions call through
  `fetch`; the provider crate's drivers need `reqwest` and do not build for
  the isolate.
- Built-in tools that need a filesystem or processes have no isolate
  equivalent; the example ships one pure tool.

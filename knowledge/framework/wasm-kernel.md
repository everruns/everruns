---
type: Decision
title: "The Execution Kernel in a JavaScript Isolate"
description: "Why core execution and its optional host build for Wasm, how time and tasks port, and how the original celld step-machine example differs."
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
`examples/durable-object-agent` runs the ordinary host and shared actor driver
inside an object. The smaller `examples/celld-engine` remains a custom step-machine
example. These surfaces remain experimental; the build guard
(`scripts/lib/check-wasm-portability.sh`) and workerd tests protect their boundaries.

## Problem

A Durable Object platform (celld, Cloudflare) gives each object a single
owner, replicated SQLite and durable alarms: a natural home for one agent
session. Hosting `serve` next to the object in a container
([serve](serve.md)) keeps durability at turn granularity, because the object
can only replay whole requests into a process it cannot see inside. Running
the engine in the object itself lets every engine event land in the object's
own storage.

## Decisions

- **Port orchestration over injected services.** `everruns-contracts` and
  `everruns-core` with default features disabled, including the optional
  `engine`, `execution-host` and `actor-runtime` features, build for wasm32.
  The portable host composes the same runtime and recovery implementation files
  as native hosts. Native filesystem/process backends, default socket/TLS transports and server
  hosting stay native. See [Shared Durable Object Execution](durable-object-native.md)
  for the authority profile and its separate qualification evidence. The
  engine's `tokio` dependency is narrowed to `rt`, `sync`, `time` and
  `macros`; `getrandom` and `uuid` take their JavaScript backends on that
  target only.
- **Time and tasks go through `everruns_contracts::rt`.** On native targets
  it re-exports Tokio's types unchanged, so paused-clock tests keep working.
  On wasm32 it uses the host clock (`web-time`), `setTimeout`
  (`gloo-timers`) and the microtask queue (`wasm-bindgen-futures`).
  `std::time::Instant::now` panics in an isolate, so kernel code uses
  `web_time::Instant` instead.
- **`Send` contracts stay.** Engine traits require `Send` so native hosts can
  use the multi-thread runtime. JavaScript futures are `!Send`; on the
  single-threaded wasm target, `rt::AssertSend` (and `worker::send` in the
  example) marks them `Send` there and only there.
- **Provider protocols use host-owned HTTP.** The shared Chat Completions and
  Responses protocols and OpenAI/OpenRouter chat drivers accept an injected
  transport on native and wasm32. Only the native default links sockets/TLS;
  the isolate host supplies fetch, incremental response bytes, redirect/egress
  policy and cancellation. Protocol encoding, options, error mapping and retry
  semantics remain shared. The celld example uses this boundary rather than a
  local Chat Completions implementation. The
  [workerd integration](../../tests/wasm-providers/README.md) proves model/tool/model
  exchanges in both Worker and Durable Object contexts, including incremental
  SSE and portable deadlines. Native non-chat services and WebSockets are
  outside this boundary.

## Original celld-engine example

The following decisions describe the original custom step machine. The shared
actor host instead uses the canonical event log and the ordinary tool recovery
policies described in [Execution Backends](execution-backends.md).

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
- Built-in tools that need a filesystem or processes have no isolate
  equivalent; the example ships one pure tool.

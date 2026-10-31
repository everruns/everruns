---
type: Decision
title: "The Execution Kernel in a JavaScript Isolate"
description: "Why core execution and its optional host build for Wasm, how time and tasks port, and how the Engine Worker stays within its upload budget."
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
inside an object. `examples/celld-engine` reuses the same Framework Agent and
Engine application with celld hosting. These surfaces remain experimental; the build guard
(`scripts/lib/check-wasm-portability.sh`) and workerd tests protect their boundaries.

## Problem

A Durable Object platform (celld, Cloudflare) gives each object a single
owner, replicated SQLite and durable alarms: a natural home for one agent
session. A native Engine beside the object and a Wasm Engine inside it both
write canonical events through the same fenced authority and recover through
independent wakeups. Placement changes host services, transport overhead and
failure domains; it does not select a different recovery protocol. See
[serve](serve.md) for native hosting and
[shared object execution](durable-object-native.md) for both placements.

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
  single-threaded wasm target, `rt::AssertSend` marks them `Send` there and only
  there.
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

## Artifact budget

The maintained isolate example uses `default-features = false`, size-oriented
release optimization and full LTO. Its ordinary Engine includes shared planning,
recovery and command policy. Its deployment and recovery qualification minify the
JavaScript glue and omit Wasm debugging name/producer sections to reserve headroom
for application behavior. Runtime exports and retained Agent identity are unchanged. The authority
bootstrap installs reducer exports
from that same Wasm module; uploading the standalone authority reducer too would
duplicate Rust code and exceed the project budget. Native executor placement still
uses the standalone reducer inside the authority transaction.

The [size guard](../../examples/durable-object-agent/check-size.mjs) measures the
complete pinned Wrangler dry-run upload, including all JS/Wasm modules, against
3,000,000 compressed bytes. The recovery CI enforces it on the built application;
this is an application budget, not a guarantee for every feature/tool combination.
Retained Agent definitions share one compatible Engine implementation. Extra
behaviors, provider protocols and tools consume the remaining budget; their final
bundle must pass the same measurement. Do not move recovery policy into JavaScript
to meet size targets.

Cloudflare's [limits](https://developers.cloudflare.com/workers/platform/limits/#worker-size)
checked on 2026-10-19 specify 64 MiB uncompressed with no compressed limit; the
project's smaller compressed budget remains deliberate. Raw Engine Wasm is larger
than 3 MB and does not claim to meet a 3 MB uncompressed deployment restriction.
Runtime startup time and memory are separate platform constraints.

## Multiplayer chat host

`examples/cloudflare-durable-chat` uses ordinary Framework Agents and Engine
with the source-only `ObjectHosting` adapter and shared OpenRouter driver. Every
conversation and history-search child has its own native Cloudflare authority,
canonical journal, execution epoch and alarm. There is no celld runtime,
sibling-example dependency, external executor or custom turn loop. Its
[hosting audit](../../examples/cloudflare-durable-chat/test/hosting-contract.mjs)
checks that dependency boundary. The Worker installs the authority command
exports from its Engine Wasm and imports the storage service directly, avoiding
a second reducer upload.
The chat also minifies its JS glue and strips Wasm name/producer sections;
these build changes preserve the Agent policy identity.

The Agent identity hashes immutable application policy instead of Framework
sources and locks. Historical live/review registrations retain that same
verified factory; a digest guard prevents silently retargeting their keys to
changed Agent code. Future behavior revisions need retained factories and
compatible room tool contracts. Model selection is retained per session.
Pre-pinning memberships capture a separately declared historical model rather
than the new-session default; custom deployments must supply their original
selection when upgrading that old shape. Engine
checks runtime compatibility and the exact registration; unsupported code pauses
execution without hiding history. Room snapshots expose those diagnostics and
the browser shows a paused state. Framework provides explicit checked retry
after code restoration; the chat does not yet expose a browser retry control.

A separate room DO owns membership, presence, forks, receipts, reminders and a
durable delivery outbox. Stable Framework message identities deduplicate lost
cross-object acknowledgements. Tool effects and child ownership use Framework's
execution identity, including the assistant tool-call message. Search-derived
answers and search children are excluded from evidence; fork imports preserve
that provenance without copying pending tools or execution ownership.

The old room payload is backed up before importing settled history, and queued
messages retain their identities. Room application state is still one JSON row
with small-demo bounds, no automatic compaction or per-user budget controls.
The chat gateway is public with no browser access gate; individual identity and
room-level authorization are not implemented. Internal room/session calls trust
private Cloudflare DO bindings; the public gateway allowlist excludes their
routes, so this example needs no internal bearer secret. Do not expose these
handlers through another public gateway. Shared HTTP authority authentication
remains unchanged for service deployments.
These remain application concerns; native SQLite host services do not implicitly
provide equivalent ports for an object adapter.

The [walkthrough](../../docs/framework/examples/durable-chat.md) owns setup and
deployment procedures. Its [workerd suite](../../examples/cloudflare-durable-chat/test/chat-e2e.mjs)
and [CI](../../.github/workflows/cloudflare-durable-chat.yml) cover retained
registration/model selection, forks, lost acknowledgements and autonomous
search/reminder recovery. Its optional previous-package mode writes journals
with an earlier Engine before upgrading at a recovery restart, so runtime
compatibility is tested across actual Wasm artifacts. Platform limits and measured
bundle size must be reported separately from any project artifact budget.

## Host requirements

Native filesystem/process tools require explicit compatible host services, or
native Engine placement over the same authority. The shared
[Engine-owned architecture](engine-durable-sessions.md) and
[qualification evidence](durable-object-native.md) own these guarantees. A journal
alone never supplies exactly-once external effects, durable files or user access
control.

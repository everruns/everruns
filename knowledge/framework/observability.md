---
type: Specification
title: "Framework Event Listeners and Observability"
description: "Engine-level push listeners for the everruns crate, with planned OpenTelemetry and Braintrust integrations."
tags:
  - everruns
  - framework
  - observability
  - rust
---

# Framework Event Listeners and Observability

Status: Implemented by EVE-1100 and EVE-1101.
Scope: the `everruns` crate (`Engine`, `Session`).

## Problem

Before EVE-1100, an application built on the `everruns` crate could observe events only by pulling:
`Session::events()` per session (`crates/everruns/src/session.rs`), or through the
awaited lifecycle hooks on `AgentBuilder` (`crates/everruns/src/agent.rs`).

It could not register a push listener. OpenTelemetry and Braintrust exist as
`everruns_core::EventListener` implementations in `everruns_core::host::observability`
(`crates/core/src/host/observability/mod.rs`), but only the server wires them
(`crates/server/src/app_builder/listeners.rs`). A framework user has to pump each
session's stream by hand, re-deserialize core events, and remember to do it again for
every resumed or spawned session. Short-lived programs also lose the last traces,
because Braintrust flushes only on its interval ticker (`crates/core/src/host/observability/braintrust.rs`) and has
no flush or shutdown call.

## Goals

- Register listeners once per process, and have them cover every session: new,
  resumed, and child.
- Enable OTel and Braintrust through a feature flag and one builder call, with no
  hand-written glue.
- Never slow down or fail a turn because of an observer. This matches the
  `EventSink` contract (`crates/core/src/host/events.rs`).
- Keep core and host event and store types off the public surface, as
  `crates/everruns/src/events.rs` requires.
- Let short-lived programs flush before they exit.

Non-goals: replaying history into listeners on resume (that would export a trace
twice); changing the server's `EventService` fan-out.

## Design

### 1. Register on the Engine, not the Agent or Session

```rust
let engine = Engine::builder()
    .listener(MyAudit)                                   // app-defined
    .on_event(|e: SessionEvent| async move { … })        // closure sugar
    .observe(everruns::observability::OpenTelemetry::from_env())
    .observe(everruns::observability::Braintrust::from_env()?)
    .build();
// Engine::new() stays and equals Engine::builder().build().
```

Why the Engine:
- Exporters are a deployment concern. They hold one process-wide client and batcher each.
- An `Agent` is an immutable snapshot that a host may rebuild per deployment. Putting
  exporters in it would duplicate clients and mix deployment config into agent
  identity.
- `create`, `resume`, and `attach` all go through the Engine
  (`crates/everruns/src/engine.rs`), so every session is covered with no
  extra step.

### 2. Two listener kinds behind one registry

- **App listeners** implement a new facade trait over the reviewed projection:
  ```rust
  #[async_trait]
  pub trait EventListener: Send + Sync + 'static {
      async fn on_event(&self, event: &SessionEvent);
      fn filter(&self) -> EventFilter { EventFilter::all() } // by canonical type string
      async fn flush(&self) {}
  }
  ```
  `SessionEvent` already carries the typed kind plus the complete
  `canonical_json()`, so nothing is lost and no core types leak.
- **Built-in integrations** (`OpenTelemetry`, `Braintrust`) are opaque facade
  values. Internally they wrap the host listeners, which need the lossless core
  `Event`. The bus already holds that `Event` before it projects it
  (`crates/everruns/src/events.rs`), so the built-ins get it directly with no JSON round trip.
  The registry represents these private slots as
  `enum Slot { App(Arc<dyn everruns::EventListener>), Host(Arc<dyn everruns_core::EventListener>) }`.
  `Host` is never constructible by applications.

### 3. Delivery: per-listener bounded queues, drained in order

- `FacadeEventBus::observe` (sync, on the commit path) already fans out to the
  broadcast channel. It holds an `Arc<Dispatcher>` handed down from the
  Engine when a session binds (`crates/everruns/src/session.rs`). `observe` calls
  `try_send` on each slot's queue and never awaits.
- Each listener gets its own bounded mpsc queue (default 8192) and its own drain
  task. A slow Braintrust upload therefore cannot starve OTel, and each listener
  sees events in commit order. OTel needs that order: its span state expects
  `turn.started` before `tool.*`.
- On overflow the event is dropped and counted. `Engine::observer_stats()` reports
  drops per listener, and a rate-limited `tracing::warn` fires. This is the same
  behavior as `EventSinkError::Full`.
- Each `on_event` runs in a spawned task. A panic is isolated to that delivery
  and counted without stopping the listener's drain task.
- The `output.message.delta` accumulated prefix stays stripped for app listeners
  (TM-DOS-040).

### 4. Lifecycle: explicit flush

- `Engine::shutdown(&self, deadline) -> ObserverReport` stops intake,
  drains the queues, then calls `flush()` on every listener. Concurrent callers
  await that same operation and report; the first caller supplies its deadline.
- Host listener flushes are fallible. OTel uses an explicit SDK provider off
  runtime threads, so exporter failures reach the observer report and the async
  deadline remains enforceable. An opaque global tracer cannot claim successful
  provider flushing. See `crates/core/src/event_listeners.rs`,
  `crates/everruns/src/observers.rs`, and the OTel listener in
  `crates/core/src/host/observability/otel.rs`.
- `Drop` for the last Engine clone does a best-effort non-blocking drain. The
  docs point short-lived programs at `shutdown`.

### 5. OTel globals belong to the application

The framework never installs a global tracer or `tracing` subscriber on its own.
- `OpenTelemetry::global()` uses the provider the application installed.
  `OpenTelemetry::with_tracer_provider(p)` takes an explicit provider.
  `OpenTelemetry::from_env()` reads the content-capture and convention env vars,
  as `OtelEventListener::new` does today.
- `everruns::observability::install_otlp_from_env() -> TelemetryGuard` is an
  opt-in convenience over `init_telemetry` (`crates/core/src/host/observability/telemetry.rs`). Its docs say it
  installs globals.
- Message content stays off by default:
  `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` or `.record_content(true)`
  turns it on. Braintrust keeps its payload mode.

### 6. Semantics

- Listeners see the same projected post-commit durable and live ephemeral events
  as `Session::events()`. Engine listeners also receive child-session events;
  each individual session stream remains isolated to its own session. A missing
  or failing listener never creates, rolls back, or reorders history (the
  contract in `crates/everruns/src/events.rs`).
- History is not replayed into observers on resume. A committed turn trace anchor
  is restored before execution continues, so a recovered phase can link to its
  original turn even when no live parent span remains.
- Sessions spawned through the Engine (subagents, tasks) inherit the dispatcher.
  Each OTel turn remains its own trace. Child delivery and task wakes carry
  bounded causal links to real exporter-created turn anchors; they do not force
  parent and child turns into one trace.

### Durable causal context

The exporter reserves a short `durable.turn` anchor and the host retains its carrier
with the committed turn start. The invocation span belongs to that turn trace;
input causes are links rather than remote parents. OTel 0.33 owns span ID generation,
so anchors use the configured tracer directly rather than synthesizing identities.
No exporter/provider or sampler is installed or replaced by this path.

The neutral [carrier contract](../../crates/contracts/src/runtime/causal.rs)
accepts only validated, nonzero W3C identities, inspects and retains at most sixteen
entries, and deduplicates causes. It carries no baggage, tracestate, content, or
remote sampling decisions. Observer hooks can return only an anchor carrier; they
cannot edit execution data. New metadata is optional and old journals remain readable.

[Shared host boundaries](../../crates/core/src/host/host/causal.rs) transfer input
causes onto turn/reason events. Reasons include visible batched and steered inputs
since the original input, bounded to the original plus 255 recent messages. The
original admission wins on duplicate message IDs even if retry telemetry changes.
Framework work requests and wakes retain the same bounded value through delivery
leases; idempotency comparisons exclude telemetry while retaining the first context.
Task registries retain bounded causal identities beside their existing internal
origin, including recovery outboxes, while preserving the existing event hierarchy;
child delegates and wake delivery reuse those identities. Custom delegate/observer adapters
must forward the optional causal methods to retain links across their own boundaries.

Anchor completion/export occurs after commit, off the admission path. Observer
loss never replays work or changes its identity. Missing/disabled/sampled-out
observers and queue overflow can leave incomplete traces or unresolved links. The anchor registry is bounded to
4096 live/abandoned turns per listener. History is not re-exported to repair gaps.

### 7. Features and stability

- Independent `everruns` features `otel` and `braintrust` forward to their
  matching `everruns-core` exporter features. Default features stay offline.
- The new surface is marked `Stability: Alpha` (see `crates/everruns/src/stability.rs`).
- Update `knowledge/framework/application-api.md`, which lists promoted concerns,
  and `docs/observability/*` with a framework section.

## EVE-1100 tests

- An app listener sees turn, tool, and output events for a session run on the
  offline simulator provider.
- A resumed session keeps flowing to the same listener, with no replay. A fresh
  Engine can attach and resume the persisted session with the same behavior.
- A listener that sleeps does not change turn latency. Overflow increments the
  drop count and the turn still succeeds.
- A panicking listener does not affect the other listeners or the turn.
- `shutdown` delivers everything queued, calls `flush`, and cancels an in-flight
  callback when its deadline expires. Concurrent Engine clones receive the same
  terminal report.
- A child Session spawned during a parent tool call reaches the Engine listener
  but not the parent session stream.
- `--no-default-features` still builds.

## EVE-1101 tests

- A Braintrust listener against a mock HTTP server sends the final batch before
  `shutdown` returns.
- OTel with an in-memory exporter yields `invoke_agent`, `chat`, and
  `execute_tool` spans from a framework run.
- The `otel` and `braintrust` features each build alone.

## Delivery plan

1. EVE-1100: `Engine::builder`, the facade `EventListener`, the per-listener
   dispatcher, `shutdown`, and the core `flush` default.
2. EVE-1101: the `otel` and `braintrust` features, `OpenTelemetry` and
   `Braintrust` values, the Braintrust flush, an `examples/` program, and docs.

## Open questions

- Should app listeners also get an unreviewed raw-core escape hatch (for example
  `everruns::advanced::HostListener`) so server-grade listeners such as
  Prometheus can be reused? Recommendation: not in v1. `canonical_json()`
  already covers auditing.
- Should drops also be reported as a `SessionEvent`? Recommendation: no, keep
  them as stats plus a warning so observers don't observe themselves.

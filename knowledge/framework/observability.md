---
type: Proposal
title: "Framework Event Listeners and Observability"
description: "Engine-level push listeners for the everruns crate, with OpenTelemetry and Braintrust as opt-in integrations that never slow a turn."
tags:
  - everruns
  - framework
  - observability
  - rust
---

# Framework Event Listeners and Observability

Status: proposed. Scope: the `everruns` crate (Engine, Session). Nothing below is implemented yet.

## Problem

Today an application built on the `everruns` crate can observe events only by pulling:
`Session::events()` per session (`crates/everruns/src/session.rs`), or through the
awaited lifecycle hooks on `AgentBuilder` (`crates/everruns/src/agent.rs`).

It cannot register a push listener. OpenTelemetry and Braintrust exist as
`everruns_core::EventListener` implementations in `everruns-host/observability`
(`crates/host/src/observability/mod.rs`), but only the server wires them
(`crates/server/src/app_builder.rs`). A framework user has to pump each
session's stream by hand, re-deserialize core events, and remember to do it again for
every resumed or spawned session. Short-lived programs also lose the last traces,
because Braintrust flushes only on its interval ticker (`crates/host/src/observability/braintrust.rs`) and has
no flush or shutdown call.

## Goals

- Register listeners once per process, and have them cover every session: new,
  resumed, and child.
- Enable OTel and Braintrust through a feature flag and one builder call, with no
  hand-written glue.
- Never slow down or fail a turn because of an observer. This matches the
  `EventSink` contract (`crates/host/src/events.rs`).
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
    .on_event(|e: &SessionEvent| async move { … })       // closure sugar
    .observe(everruns::observability::OpenTelemetry::from_env())   // feature "otel"
    .observe(everruns::observability::Braintrust::from_env()?)     // feature "braintrust"
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
      fn filter(&self) -> EventFilter { EventFilter::All }   // by SessionEventKind or type string
      async fn flush(&self) {}
  }
  ```
  `SessionEvent` already carries the typed kind plus the complete
  `canonical_json()`, so nothing is lost and no core types leak.
- **Built-in integrations** (`OpenTelemetry`, `Braintrust`) are opaque facade
  values. Internally they wrap the host listeners, which need the lossless core
  `Event`. The bus already holds that `Event` before it projects it
  (`crates/everruns/src/events.rs`), so the built-ins get it directly with no JSON round trip.
  The registry stores
  `enum Slot { App(Arc<dyn everruns::EventListener>), Host(Arc<dyn everruns_core::EventListener>) }`.
  `Host` is never constructible by applications.

### 3. Delivery: per-listener bounded queues, drained in order

- `FacadeEventBus::observe` (sync, on the commit path) already fans out to the
  broadcast channel. It gains an `Option<Arc<Dispatcher>>` handed down from the
  Engine when a session binds (`crates/everruns/src/session.rs`). `observe` calls
  `try_send` on each slot's queue and never awaits.
- Each listener gets its own bounded mpsc queue (default 8192) and its own drain
  task. A slow Braintrust upload therefore cannot starve OTel, and each listener
  sees events in commit order. OTel needs that order: its span state expects
  `turn.started` before `tool.*`.
- On overflow the event is dropped and counted. `Engine::observer_stats()` reports
  drops per listener, and a rate-limited `tracing::warn` fires. This is the same
  behavior as `EventSinkError::Full`.
- Each `on_event` runs inside `catch_unwind` (or a spawned task), as
  `CompositeEventListener` already does. That code can be reused.
- The `output.message.delta` accumulated prefix stays stripped for app listeners
  (TM-DOS-037).

### 4. Lifecycle: explicit flush

- Add `Engine::shutdown(&self, deadline) -> ObserverReport`. It stops intake,
  drains the queues, then calls `flush()` on every listener.
- Add `async fn flush(&self) {}` as a defaulted method on
  `everruns_core::EventListener`. Braintrust implements it by signalling its
  batch task and awaiting the final POST. The OTel wrapper implements it with
  `force_flush` on its provider.
- `Drop` for the last Engine clone does a best-effort non-blocking drain. The
  docs point short-lived programs at `shutdown`.

### 5. OTel globals belong to the application

The framework never installs a global tracer or `tracing` subscriber on its own.
- `OpenTelemetry::global()` uses the provider the application installed.
  `OpenTelemetry::with_tracer_provider(p)` takes an explicit provider.
  `OpenTelemetry::from_env()` reads the content-capture and convention env vars,
  as `OtelEventListener::new` does today.
- `everruns::observability::install_otlp_from_env() -> TelemetryGuard` is an
  opt-in convenience over `init_telemetry` (`crates/host/src/observability/telemetry.rs`). Its docs say it
  installs globals.
- Message content stays off by default:
  `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` or `.record_content(true)`
  turns it on. Braintrust keeps its payload mode.

### 6. Semantics

- Listeners see exactly what `Session::events()` sees: post-commit durable events
  plus live ephemeral ones. A missing or failing listener never creates, rolls
  back, or reorders history (the contract in `crates/everruns/src/events.rs`).
- Nothing is replayed on resume. The first event after `session.resumed` opens a
  new turn span.
- Sessions spawned through the Engine (subagents, tasks) inherit the dispatcher.
  OTel links them through the existing `parent_span_id` handling.

### 7. Features and stability

- New `everruns` features: `otel` forwards to `everruns-host/observability`
  (OTel parts). `braintrust` needs the same host feature plus `direct-egress`.
  A later split of the host feature into `otel` and `braintrust` would keep each
  dependency tree small. Default features stay offline.
- The new surface is marked `Stability: Alpha` (see `crates/everruns/src/stability.rs`).
- Update `knowledge/framework/application-api.md`, which lists promoted concerns,
  and `docs/observability/*` with a framework section.

## Tests

- An app listener sees turn, tool, and output events for a session run on the
  offline simulator provider.
- A resumed session keeps flowing to the same listener, with no replay.
- A listener that sleeps does not change turn latency. Overflow increments the
  drop count and the turn still succeeds.
- A panicking listener does not affect the other listeners or the turn.
- `shutdown` delivers everything queued and calls `flush`. A Braintrust test
  against a mock HTTP server gets the final batch before `shutdown` returns.
- OTel with an in-memory exporter yields `invoke_agent`, `chat`, and
  `execute_tool` spans from a framework run.
- `--no-default-features` still builds, and `otel` and `braintrust` each build alone.

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

---
title: Observability
description: Export Framework sessions to OpenTelemetry and Braintrust, register your own event listeners, and flush them on shutdown.
---

An Engine delivers every event from every session it owns to the listeners
registered on it. The `everruns` crate ships two listeners, OpenTelemetry and
Braintrust, each behind its own Cargo feature. Without either feature nothing
leaves the process.

```toml
[dependencies]
everruns = { version = "0.34", features = ["openai", "otel", "braintrust"] }
```

The features are independent: enable only the one you export to.

## OpenTelemetry

The `otel` feature adds `everruns::observability::OpenTelemetry`. Register it on
the Engine builder:

```rust
use std::time::Duration;
use everruns::observability::{OpenTelemetry, install_otlp_from_env};
use everruns::{Agent, Engine, Model};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
let _telemetry = install_otlp_from_env();
let engine = Engine::builder()
    .observe(OpenTelemetry::from_env())
    .build();

let agent = Agent::builder()
    .instructions("Answer concisely.")
    .model(Model::simulated("Hello."))
    .build()?;
engine.create(agent).run("Say hello.").await?;

let report = engine.shutdown(Duration::from_secs(10)).await;
assert!(!report.timed_out);
# Ok(())
# }
```

Pick the constructor by who owns the tracer provider:

| Constructor | Tracer provider | Content capture |
|---|---|---|
| `OpenTelemetry::global()` | The global provider your application installed | Off |
| `OpenTelemetry::with_tracer_provider(provider)` | An `SdkTracerProvider` you pass in. `Engine::shutdown` force-flushes it | Off |
| `OpenTelemetry::from_env()` | The global provider | From the environment |

None of them installs global state. `install_otlp_from_env()` is the one helper
that does: it installs an OTLP exporter as the global tracer provider and a
console `tracing` subscriber, and returns a `TelemetryGuard`. Keep the guard
alive until after `Engine::shutdown`. Skip it when your application already
sets up OpenTelemetry.

`install_otlp_from_env()` reads:

| Variable | Default | Effect |
|---|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | none | OTLP endpoint. Without it no spans are exported, and only the console subscriber is installed |
| `OTEL_SERVICE_NAME` | `everruns` | Service name on the spans |
| `OTEL_SERVICE_VERSION` | none | Service version |
| `OTEL_ENVIRONMENT` | none | Deployment environment label |
| `OTEL_SDK_DISABLED` | `false` | `true` turns exporting off without unsetting the endpoint |
| `RUST_LOG`, then `LOG_LEVEL` | `info` | Console log filter |

`OpenTelemetry::from_env()` reads
`OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT` (or its older alias
`OTEL_RECORD_CONTENT`) and `EVERRUNS_TRACE_CONVENTIONS`. Content capture puts
instructions, messages, reasoning, and tool arguments and results on spans; it
is off by default. Turn it on in code with `.record_content(true)`.

The span layout and attribute vocabularies are the same as on the platform:
see [OpenTelemetry](/observability/opentelemetry/).

## Braintrust

The `braintrust` feature adds `everruns::observability::Braintrust`.
`Braintrust::from_env()` returns `None` when Braintrust is disabled or no API
key is set, so registration is conditional:

```rust
use std::time::Duration;
use everruns::Engine;
use everruns::observability::Braintrust;

# #[tokio::main]
# async fn main() {
let mut builder = Engine::builder();
if let Some(braintrust) = Braintrust::from_env() {
    builder = builder.observe(braintrust);
}
let engine = builder.build();
// ... run sessions ...
let report = engine.shutdown(Duration::from_secs(10)).await;
# let _ = report;
# }
```

It reads the `BRAINTRUST_*` variables listed on
[Braintrust](/observability/braintrust/). Its final batch is sent before
`Engine::shutdown` returns.

## Your own listeners

Implement `EventListener` to receive the same events in application code, for
example to write an audit log or update metrics:

```rust
use everruns::approval::async_trait;
use everruns::{Engine, EventFilter, EventListener, SessionEvent};

struct ToolAudit;

#[async_trait]
impl EventListener for ToolAudit {
    async fn on_event(&self, event: &SessionEvent) {
        println!("{} {}", event.session_id, event.event_type());
    }

    fn filter(&self) -> EventFilter {
        EventFilter::event_types(["tool.started", "tool.completed"])
    }

    fn name(&self) -> &'static str {
        "tool-audit"
    }
}

let engine = Engine::builder().listener(ToolAudit).build();
# let _ = engine;
```

| Method | Default | Purpose |
|---|---|---|
| `on_event` | required | Handle one event, after it is committed |
| `filter` | `EventFilter::all()` | Restrict delivery to the listed event types |
| `flush` | no-op | Write out buffered state during `Engine::shutdown` |
| `name` | `"EventListener"` | Label in observer statistics |

For a quick hook, `Engine::builder().on_event(|event| async move { ... })`
registers an async closure that receives each `SessionEvent` by value.

The event types and payloads are listed in [Events](/features/events/).

## Delivery and shutdown

Each listener has its own bounded queue, holding up to
`OBSERVER_QUEUE_CAPACITY` (8192) events and 8 MiB of serialized event data. A
slow listener never delays a turn or another listener. When its queue is full,
new events for that listener are dropped and counted. A listener that panics is
isolated, and the panic is counted.

`engine.observer_stats()` returns those counters per listener (`delivered`,
`dropped`, `panics`, `flushed`), with `dropped()` and `panics()` totals.

`engine.shutdown(deadline)` stops accepting events, drains the queues, and
calls `flush` on each listener. Listeners still running at the deadline are
cancelled, and the returned `ObserverReport` has `timed_out` set. Call it
before the process exits, or the last events may never reach the exporter.

```rust
use std::time::Duration;
use everruns::Engine;

# #[tokio::main]
# async fn main() {
let engine = Engine::builder().build();
let report = engine.shutdown(Duration::from_secs(10)).await;
if report.timed_out || report.stats.dropped() > 0 {
    eprintln!("observers lost events: {:?}", report.stats);
}
# }
```

## Runnable example

The [Framework observability example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/framework_observability.rs) registers both
integrations and runs a turn on a simulated model, so it needs no provider key:

```bash
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318 \
  cargo run -p everruns --features otel,braintrust \
  --example framework_observability
```

## See also

- [Events and Cancellation](/framework/events-and-cancellation/), per-session event streams
- [Deploying a Framework app](/framework/deployment/)
- [OpenTelemetry](/observability/opentelemetry/) and [Braintrust](/observability/braintrust/), exporter details shared with the platform

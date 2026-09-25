---
title: Observability
description: Send Everruns session traces, token usage, and tool-call timings to your observability platform of choice.
sidebar:
  order: 0
---

Everruns emits structured events for every agent turn, model calls, tool invocations, retries, token usage, latency. The integrations in this section forward those signals to observability platforms so you can monitor agents in production, evaluate prompt changes, and debug failures with full trace context.

## Available Integrations

- [OpenTelemetry](/observability/opentelemetry/), export traces over OTLP to any tracing backend. Spans follow the Gen-AI semantic conventions and the OpenInference conventions at once, so Grafana Tempo, Jaeger, Datadog, Langfuse, and Arize Phoenix all read them.
- [Braintrust](/observability/braintrust/), LLM observability, evaluation, and trace visualization. Turn traces are grouped by session, with token usage, time-to-first-token, and tool execution times.

## Framework Applications

The `everruns` crate keeps observability offline unless an application enables
the independent `otel` or `braintrust` feature. Register an integration once on
the Engine:

```rust
use std::time::Duration;
use everruns::{Engine, observability::OpenTelemetry};

let engine = Engine::builder()
    .observe(OpenTelemetry::from_env())
    .build();

let report = engine.shutdown(Duration::from_secs(10)).await;
assert!(!report.timed_out);
```

`OpenTelemetry::global()` uses a provider that the application already
installed. `OpenTelemetry::with_tracer_provider(provider)` keeps an explicit
provider and force-flushes it during `Engine::shutdown`. Neither constructor
installs global state. Use `install_otlp_from_env()` only when the application
explicitly wants Everruns to install the OTLP provider and tracing subscriber.

`Braintrust::from_env()` returns `None` when it is not configured. Its final
HTTP batch completes before `Engine::shutdown` returns. Message content is off
by default; set `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=true` or
call `.record_content(true)` to include it in OpenTelemetry spans.

Run the complete provider-free Framework example while sending spans to a
local OTLP collector:

```bash
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318 \
  cargo run -p everruns --features otel,braintrust \
  --example framework_observability
```

## Related

- [Events](/features/events/), the streaming event protocol that backs every observability export.
- [Environment Variables](/sre/environment-variables/), configure exporters, sampling, and OTLP endpoints.
- [Runnable Framework examples](/framework/examples/), complete programs built on the `everruns` crate.

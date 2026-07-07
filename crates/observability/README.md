# everruns-observability

> Observability exporters (Braintrust, OpenTelemetry) for Everruns agents.

Event-listener backends that translate the agentic loop's events into external
observability systems. Split out of `everruns-core` (EVE-651) so the core crate
carries no exporter-specific HTTP/telemetry dependencies and a misconfigured
exporter can never panic core's initialization path.

## Backends

- **`braintrust`** — Braintrust tracing and logging over HTTP (`reqwest`).
- **`otel`** — OpenTelemetry spans following the gen-AI semantic conventions.
  Emits plain `tracing` spans; the OTLP exporter wiring lives in
  `everruns_core::telemetry`.

Both implement `everruns_core::EventListener`.

See `specs/observability.md` for the full specification.

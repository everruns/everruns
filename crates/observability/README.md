# everruns-observability

> Observability exporters (Braintrust, OpenTelemetry) for Everruns agents.

Runtime and exporter implementations behind the neutral observability
contracts in `everruns-core`. Split out of `everruns-core` (EVE-651, EVE-876)
so the core crate carries no exporter, OpenTelemetry SDK, or
tracing-subscriber dependencies and a misconfigured exporter can never panic
core's initialization path.

Part of the [Everruns](https://everruns.com) ecosystem.

## What It Provides

- **`telemetry`**: OpenTelemetry initialization: OTLP exporter wiring,
  tracing-subscriber layers, `TelemetryConfig` / `TelemetryGuard` /
  `init_telemetry`.
- **`composite`**: `CompositeEventListener` fan-out with panic isolation.
- **`braintrust`**: Braintrust tracing and logging over HTTP (`reqwest`).
- **`otel`**: OpenTelemetry spans following the gen-AI semantic conventions.
  Emits plain `tracing` spans; the OTLP exporter wiring lives in `telemetry`.

Listener backends implement `everruns_core::EventListener`. Core keeps only
the neutral contracts: the `EventListener` trait, event types, and the gen-AI
span conventions in `everruns_core::telemetry`.

```rust
use everruns_observability::CompositeEventListener;

let _type_name = std::any::type_name::<CompositeEventListener>();
```

## Documentation

- [Observability guide](https://docs.everruns.com/observability/)
- [API reference](https://docs.rs/everruns-observability)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).

---
title: Framework engines get event listeners and OpenTelemetry export
description: Register a listener once per Engine to receive events from every session it owns, and export Framework sessions to OpenTelemetry or Braintrust.
date: 2026-09-25
kind: feature
significance: notable
products: [framework]
surfaces: [code]
release: "0.32.0"
links:
  docs: [/framework/observability/]
  prs: [everruns/everruns#3804, everruns/everruns#3806]
---

![Everruns and OpenTelemetry logos side by side, announcing OpenTelemetry export](./integration@2x.png)

An Engine now delivers every event from every session it owns, including
resumed and spawned ones, to the listeners registered on it. You no longer poll
`Session::events()` per session. Each listener drains a bounded ordered queue,
and shutdown flushes them within one deadline.

The `everruns` crate ships two listeners behind independent features, `otel`
and `braintrust`:

```rust
let engine = Engine::builder()
    .observe(OpenTelemetry::from_env())
    .build();
```

Message content is not exported by default.

See [Observability](/framework/observability/). PRs [#3804](https://github.com/everruns/everruns/pull/3804),
[#3806](https://github.com/everruns/everruns/pull/3806).

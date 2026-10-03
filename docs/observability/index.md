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

Framework apps export the same traces from the `everruns` crate, behind the
`otel` and `braintrust` Cargo features. See
[Framework observability](/framework/observability/) for registering exporters
and your own listeners on the Engine, and for flushing them on shutdown.

## Related

- [Events](/features/events/), the streaming event protocol that backs every observability export.
- [Environment Variables](/sre/environment-variables/), configure exporters, sampling, and OTLP endpoints.
- [Runnable Framework examples](/framework/examples/), complete programs built on the `everruns` crate.

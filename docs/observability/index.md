---
title: Observability
description: Send Everruns session traces, token usage, and tool-call timings to your observability platform of choice.
sidebar:
  order: 0
---

Everruns emits structured events for every agent turn, model calls, tool invocations, retries, token usage, latency. The integrations in this section forward those signals to observability platforms so you can monitor agents in production, evaluate prompt changes, and debug failures with full trace context.

## Investigate failed turns

In the platform UI, each agent's activity chart covers the last 24 hours, with
one bar per hour. Hover or keyboard-focus the chart to confirm the time window.
Click **1 failed** (or the corresponding failure count) to open the agent's
**Sessions** tab filtered to failures in that chart's window.

The filter uses the time a turn failed, so it includes sessions created earlier,
archived sessions, and sessions that later recovered. Counts distinguish failed
turns from affected sessions. Each card shows its current outcome and the latest
matching failure. **View failure** opens that turn in **Trace**, selecting the
error step when one was recorded. A failure can exist without a model or tool
step; the Trace banner still identifies the failed turn.

Use **With failed turns** in an agent's Sessions tab to investigate failures
over the last 24 hours, 7 days, or 30 days. Choose **All sessions** to clear the
failure filter.

For scripted investigation, run `everruns agents failures list <agent-id>`.
Use `--after` and `--before` to choose a failure-time window, and `--limit` and
`--offset` to page through matching sessions.

## Available Integrations

For live execution inside the platform, use [Monitoring](/observability/monitoring/).
It provides organization-scoped and platform-wide views of execution phases,
worker ownership, and actor leases.

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

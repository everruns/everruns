---
title: OpenTelemetry fixes export and cached-token totals
description: OpenTelemetry fixes background export, cached-token totals, repeated tool-call IDs, error privacy and Framework shutdown failure reporting.
date: 2026-10-14
kind: fix
significance: notable
products: [platform, framework]
surfaces: [operations, code]
links:
  docs: [/observability/opentelemetry/, /framework/observability/]
---

OpenTelemetry exports now use an HTTP client compatible with background batching. Repeated provider tool-call IDs stay isolated across sessions and turns, while prompt totals include cache reads and writes. Errors before a provider stream opens produce model spans.

With content capture off, error status and exception messages carry safe classifications. Detailed error text follows the same opt-in as messages and tool payloads; see [OpenTelemetry](/observability/opentelemetry/).

[Framework observers](/framework/observability/) report exporter flush failures and keep shutdown deadlines responsive. Pass an explicit tracer provider to have Engine shutdown flush it, then use the telemetry guard's async shutdown before exiting.

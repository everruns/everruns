---
title: OpenTelemetry traces follow Gen-AI and OpenInference conventions
description: Agent turn traces now carry the OpenTelemetry Gen-AI agent attributes and OpenInference attributes on the same spans, selectable per deployment.
date: 2026-09-03
kind: improvement
significance: notable
products: [platform, framework]
surfaces: [operations]
release: "0.24.0"
links:
  docs: [/observability/opentelemetry/, /framework/observability/]
  prs: [everruns/everruns#3353]
---

OpenTelemetry traces of agent turns now follow the current Gen-AI agent and
inference conventions and carry OpenInference attributes on the same spans. The
turn root carries the agent identity, chat spans have real durations, and thinking
is nested under its model call.

Choose the vocabularies with `EVERRUNS_TRACE_CONVENTIONS` (default
`gen_ai,openinference`):

```bash
# OpenInference only, for example for Arize Phoenix
export EVERRUNS_TRACE_CONVENTIONS=openinference
```

See [OpenTelemetry](/observability/opentelemetry/) and
[Framework observability](/framework/observability/). [#3353](https://github.com/everruns/everruns/pull/3353)

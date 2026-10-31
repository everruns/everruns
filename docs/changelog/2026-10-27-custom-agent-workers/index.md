---
title: Connect custom agent workers to your control plane
description: Self-hosted installations can connect agent-scoped custom workers with private tools while retaining session APIs, canonical events and tracing.
date: 2026-10-27
kind: feature
significance: notable
products: [platform]
release: "0.50.0"
surfaces: [code, operations]
links:
  docs: [/how-to/custom-agent-workers/]
---

Self-hosted installations can [run custom agent workers](/how-to/custom-agent-workers/)
that own their model calls, tools and database access. Users keep the existing
session APIs, chat interface and event streams. An operator provisions an
agent-scoped credential and enables a separate restricted listener:

```bash
CUSTOM_WORKER_GRPC_BIND_ADDR=127.0.0.1:9002
```

The Framework data-agent example queries a private SQLite database and forwards
canonical execution events through the control plane. Lease checks restrict
reads and writes to the worker's assigned sessions; revoked deployments retain
their routing binding so built-in workers cannot silently take over.

This is experimental and operator-managed. Custom code owns its execution policy
and side-effect recovery. Customer image hosting in Everruns Cloud is not enabled.

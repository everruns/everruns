---
title: Upgrade a Kubernetes install without downtime
description: The Helm chart runs several API servers rolled one at a time, scales workers on chat load, and adds network policies, S3 file storage, metrics and the egress proxy.
date: 2026-11-08
kind: improvement
significance: notable
products: [platform]
surfaces: [operations]
links:
  docs: [/getting-started/kubernetes/]
---

The [Helm chart](/getting-started/kubernetes/) now runs the same shape as the
managed demo. With two or more API servers, an upgrade or a node drain rolls
them one at a time and the API stays up; before, the single server was stopped
before its replacement started.

```bash
helm upgrade everruns oci://ghcr.io/everruns/charts/everruns --reuse-values \
  --namespace everruns \
  --set server.replicas=2 --set valkey.enabled=true \
  --set worker.autoscaling.enabled=true --set networkPolicy.enabled=true
```

Worker autoscaling now counts chat turns, running and waiting, not only
background tasks, so a busy chat adds workers. A stopping worker gets two
minutes to hand off its running turns. The bundled NATS and Valkey get
generated passwords.

New, off by default: NetworkPolicies between the parts, session files in S3
with cloud identity, a Prometheus metrics port and ServiceMonitor, and the
tenant egress proxy that gives agent traffic its own address and per-org caps.

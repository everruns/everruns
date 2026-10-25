---
title: Helm chart for Kubernetes
description: An official Helm chart installs the self-hosted Platform on Kubernetes, published with every release as an OCI chart.
date: 2026-10-25
kind: feature
significance: notable
products: [platform]
surfaces: [operations]
links:
  docs: [/getting-started/kubernetes/]
  prs: [everruns/everruns#4910]
---

The `everruns` Helm chart installs the self-hosted Platform on Kubernetes: the
API server, workers, the UI, a proxy with the same routes as Docker Compose,
and NATS. Bring your own PostgreSQL, or turn on a single bundled one to try it
out. Workers can scale on queue depth with KEDA.

```bash
helm install everruns oci://ghcr.io/everruns/charts/everruns \
  --namespace everruns --create-namespace \
  --set database.url="$DATABASE_URL" --set auth.adminEmail=you@example.com
```

The chart generates the encryption key and other secrets on the first install
and keeps them across upgrades and uninstalls. See
[Kubernetes](/getting-started/kubernetes/).

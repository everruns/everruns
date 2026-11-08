---
title: Install on Kubernetes with everruns k8s up
description: One CLI command checks the cluster, installs the Helm chart, waits for the pods and signs the CLI in; later runs repair it and upgrade backs up first.
date: 2026-11-08
kind: feature
significance: notable
products: [platform]
surfaces: [cli, operations]
links:
  docs: [/getting-started/kubernetes/, /reference/cli/k8s/]
---

`everruns k8s up` does for a Kubernetes cluster what
[`everruns docker up`](/getting-started/docker-compose/) does for a laptop. It
shows which cluster it is about to use and asks first, checks the cluster can
take the install, asks a few questions ([all options](/reference/cli/k8s/)), installs the
[Helm chart](/getting-started/kubernetes/), waits for the pods, and signs the
[CLI](/features/cli/) in.

```bash
everruns k8s up --database-url "$DATABASE_URL" \
  --public-url https://everruns.example.com --ingress-class nginx
everruns agents list
```

Running it again repairs the install and keeps every value set before,
`--set server.replicas=2` included. `everruns k8s upgrade` moves to the CLI's
chart version and dumps a bundled database first. Secrets stay in the cluster,
never on your machine. Without a public URL, `everruns k8s port-forward` keeps
it reachable on `localhost:9300`.

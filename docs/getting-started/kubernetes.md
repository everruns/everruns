---
title: Kubernetes
description: Install the self-hosted Everruns Platform on Kubernetes with the official Helm chart, using your own PostgreSQL and secrets kept across upgrades.
sidebar:
  label: "Self-hosted: Kubernetes"
appliesTo: [platform]
---

The `everruns` Helm chart installs the self-hosted Platform: the API server,
workers, the UI, a proxy in front of them, and NATS. Each release publishes the
chart as an OCI chart that runs that release's images.

## Prerequisites

- Kubernetes 1.27+ and Helm 3.8+
- PostgreSQL 15+ that the cluster can reach, such as a managed database or one
  run by an operator like CloudNativePG
- About 2 GB of memory for the Everruns pods

## Install

```bash
helm install everruns oci://ghcr.io/everruns/charts/everruns \
  --namespace everruns --create-namespace \
  --set database.url='postgres://everruns:PASSWORD@db.example.com:5432/everruns' \
  --set auth.adminEmail=you@example.com \
  --set publicUrl=https://everruns.example.com
```

The first start runs the database migrations. Then sign in as the admin with
the generated password:

```bash
kubectl -n everruns get secret everruns-generated \
  -o jsonpath='{.data.AUTH_ADMIN_PASSWORD}' | base64 -d
kubectl -n everruns port-forward svc/everruns 9300:9300   # then open http://localhost:9300
```

To try the chart without a database of your own, add
`--set postgresql.enabled=true`. It starts a single PostgreSQL pod with no
backups or failover, so use it only for experiments.

## What it runs

| Part | Notes |
|---|---|
| Server | Always one replica, replaced with `Recreate`: it keeps per-process session state and applies migrations when it starts. |
| Workers | `worker.replicas` (default 1), or scaled on queue depth by [KEDA](https://keda.sh) with `worker.autoscaling.enabled=true`. Scale workers, not the server. |
| UI | One replica. |
| Proxy | Caddy with the same routes as the [Docker Compose](/getting-started/docker-compose/) setup: `/api`, `/mcp`, OAuth and agent-readable documents go to the server, the rest to the UI. Service `everruns`, port 9300. |
| NATS | Bundled with JetStream for live event delivery, or `nats.url` for your own. |
| Valkey | Optional (`valkey.enabled` or `valkey.url`). The single server keeps rate limits in memory without it. |

## Expose it

Point an Ingress at the proxy service:

```yaml
publicUrl: https://everruns.example.com
ingress:
  enabled: true
  className: nginx
  host: everruns.example.com
  tls:
    - secretName: everruns-tls
      hosts: [everruns.example.com]
```

`publicUrl` is the address people use. Sign-in callbacks and links are built
from it.

## Who signs in

`auth.mode` works like the server's `AUTH_MODE`:

- `admin` (default): one admin account, `auth.adminEmail`, with a password
  generated on install or set with `auth.adminPassword`.
- `full`: people sign up and sign in. Each organization adds its own provider
  keys.
- `none`: no sign-in. Only with `deploymentGrade: dev`, and only on a network
  you trust.

## Secrets

On the first install the chart generates the encryption key, the session
signing key, the worker token and the admin password, and stores them in the
Secret `<release>-generated`. Upgrades read them back instead of making new
ones, and uninstalling the chart leaves that Secret in place.

**Back up that Secret with the database.** The encryption key protects the
provider keys and secrets stored in the database; without it they cannot be
read. To manage these values yourself, create a Secret with
`SECRETS_ENCRYPTION_KEY`, `AUTH_JWT_SECRET`, `WORKER_GRPC_AUTH_TOKEN` and
`AUTH_ADMIN_PASSWORD` and set `secrets.existingSecret`.

Provider keys for the whole server go in `providerKeys` or in a Secret named by
`providerKeys.existingSecret`. With `seedDefaultProviderKeys: true` they become
the default organization's keys (not with `auth.mode: full`).

## Upgrade

```bash
helm upgrade everruns oci://ghcr.io/everruns/charts/everruns --version 0.49.0 \
  --namespace everruns --reuse-values
```

The new server applies the database migrations when it starts. Migrations only
go forward, so back up the database first.

## No operator

Everruns ships a chart, not an operator. PostgreSQL is the only stateful part,
and managed databases or a PostgreSQL operator handle its lifecycle. The server
migrates itself, and KEDA scales workers. Agents and sessions live in the
database behind the API, so they are managed with the
[CLI](/features/cli/), the UI or MCP, not as Kubernetes objects.

All chart values, with comments:
[`charts/everruns/values.yaml`](https://github.com/everruns/everruns/blob/main/charts/everruns/values.yaml).

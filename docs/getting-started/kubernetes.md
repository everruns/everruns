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
| Server | `server.replicas` (default 1). It applies migrations when it starts. See [Run several servers](#run-several-servers). |
| Workers | `worker.replicas` (default 1), or scaled by [KEDA](https://keda.sh) with `worker.autoscaling.enabled=true` on waiting and running work: background tasks and chat turns. |
| UI | `ui.replicas` (default 1). |
| Proxy | Caddy with the same routes as the [Docker Compose](/getting-started/docker-compose/) setup: `/api`, `/mcp`, OAuth and agent-readable documents go to the server, the rest to the UI. Service `everruns`, port 9300. `proxy.replicas` (default 1). |
| NATS | Bundled with JetStream and a generated password for live event delivery, or `nats.url` for your own. |
| Valkey | Optional with one server (`valkey.enabled` or `valkey.url`), which then keeps rate limits in memory. Required with more. |

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

## Run several servers

With one server, an upgrade stops it before the new one starts, so requests
wait for a short while. With two or more, servers roll one at a time and the
API stays up through upgrades and node drains. This is how the managed demo
runs:

```yaml
server:
  replicas: 2
valkey:
  enabled: true      # shared rate limits; NATS is on by default
ui:
  replicas: 2
proxy:
  replicas: 2
worker:
  autoscaling:
    enabled: true    # needs KEDA in the cluster
    maxReplicas: 10
```

More than one server needs NATS, so each server's event streams see events the
others write, and Valkey, so rate limits are shared. The chart refuses to
render without them. Workers find every server through a headless Service and
spread their calls across them.

## Lock down traffic between pods

`networkPolicy.enabled=true` adds NetworkPolicies so each part accepts traffic
only from the parts that call it: the server from the proxy, the UI and
workers; NATS from the server; Valkey from the server and workers; the UI from
the proxy. Only the proxy accepts traffic from outside. It needs a network
plugin that enforces NetworkPolicy, such as Calico or Cilium. With bundled
PostgreSQL and worker autoscaling, KEDA's namespace (`networkPolicy.kedaNamespace`,
default `keda`) may read the database.

## Keep files in S3

Session files and other uploads are stored in PostgreSQL by default. To keep
them in S3 or an S3-compatible store:

```yaml
storage:
  s3:
    enabled: true
    bucket: everruns-files
    region: us-east-1
    # endpoint: https://minio.example.com   # S3-compatible stores
    # existingSecret: s3-keys   # STORAGE_S3_ACCESS_KEY_ID / STORAGE_S3_SECRET_ACCESS_KEY
serviceAccount:
  create: true
  annotations:
    eks.amazonaws.com/role-arn: arn:aws:iam::123456789012:role/everruns-files
```

Without `existingSecret`, the server uses its pod's cloud identity, such as
EKS IRSA or GKE Workload Identity, through `serviceAccount`.

## Metrics

`metrics.enabled=true` serves the server's Prometheus metrics on their own port
(9090) and Service. With the Prometheus Operator, add
`metrics.serviceMonitor.enabled=true`.

## Send agent traffic through an egress proxy

`egressProxy.enabled=true` runs the tenant egress proxy. Agent tool calls,
integrations and MCP traffic then leave through it, while the platform's own
calls, such as LLM providers, go direct. Schedule it on nodes with their own
NAT gateway (`egressProxy.nodeSelector`, `egressProxy.tolerations`) and agent
traffic gets its own public address, which you can drain or replace without
touching the platform's. `egressProxy.orgLimits` caps requests, concurrency and
bytes per organization. With network policies on, the proxy also cannot reach
private address ranges, the cluster or instance metadata.

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
Secret `<release>-generated`, and the bundled NATS and Valkey passwords in
`<release>-internal`. Upgrades read them back instead of making new
ones, and uninstalling the chart leaves those Secrets in place.

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

An upgrade does not interrupt running agents. The old server drains for up to
45 seconds (`server.terminationGracePeriodSeconds`): it lets in-flight requests
finish and ends event streams with a `disconnecting` event so clients reconnect.
Workers roll one at a time with the replacement started first, and a stopping
worker hands each running turn to another worker at its next step. A
PodDisruptionBudget (`worker.podDisruptionBudget`) keeps node drains to one
worker at a time. With one server, it is stopped before its replacement
starts, so requests in that gap wait for it to come back; with
[several servers](#run-several-servers) there is no gap.

## No operator

Everruns ships a chart, not an operator. PostgreSQL is the only stateful part,
and managed databases or a PostgreSQL operator handle its lifecycle. The server
migrates itself, and KEDA scales workers. Agents and sessions live in the
database behind the API, so they are managed with the
[CLI](/features/cli/), the UI or MCP, not as Kubernetes objects.

All chart values, with comments:
[`charts/everruns/values.yaml`](https://github.com/everruns/everruns/blob/main/charts/everruns/values.yaml).

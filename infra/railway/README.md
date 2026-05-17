# Railway Deployment And Template

This directory documents the Everruns Railway template shape and keeps a
maintainable Caddy ingress config for repo-backed deployments. Railway does not
run Docker Compose directly, so the live template maps
`examples/docker-compose-full.yaml` into individual Railway services.

Current validated Everruns image tag:

```text
v0.8.31
```

Pin the live validation deployment to `v0.8.31`. The public template can use
`latest` after the validation deployment is healthy, because the release
workflow only moves `latest` on versioned releases.

Template image URL:

```text
https://raw.githubusercontent.com/everruns/everruns/main/infra/railway/template-icon.svg
```

## Service Shape

| Service | Source | Public | Notes |
| --- | --- | --- | --- |
| `postgres` | Railway PostgreSQL | No | Use Railway-managed Postgres. |
| `redis` | Railway Redis | No | Used as `VALKEY_URL` for distributed rate limiting. |
| `nats` | Docker image `nats:2-alpine` | No | Start with JetStream and a `/data` volume. |
| `server` | Docker image `ghcr.io/everruns/everruns-server:v0.8.31` | No | HTTP API on `9000`, worker gRPC on `9001`. |
| `worker` | Docker image `ghcr.io/everruns/everruns-worker:v0.8.31` | No | Scale replicas as needed. |
| `ui` | Docker image `ghcr.io/everruns/everruns-ui:v0.8.31` | No | Next.js server on `9100`. |
| `caddy` | GitHub repo path `infra/railway/caddy` | Yes | Single public ingress. |

## Template Inputs

Required user inputs:

```env
AUTH_ADMIN_EMAIL=<template input>
AUTH_ADMIN_PASSWORD=<template input>
```

Provider API keys are intentionally not template inputs. Configure provider
keys in the Everruns UI after deployment, or add `DEFAULT_*` variables to the
server service manually when a deployment should start with seeded defaults.

## Caddy Service

Use the repo-backed service at `infra/railway/caddy`. Keep public HTTP
networking enabled only on this service.

Variables:

```env
SERVER_HOST=${{ server.RAILWAY_PRIVATE_DOMAIN }}
UI_HOST=${{ ui.RAILWAY_PRIVATE_DOMAIN }}
```

Healthcheck path:

```text
/health
```

## NATS Service

Use Docker image `nats:2-alpine`.

Start command:

```bash
nats-server --jetstream --store_dir /data -m 8222
```

Attach a volume at:

```text
/data
```

## Server Variables

```env
DATABASE_URL=${{ postgres.DATABASE_URL }}
DATABASE_UNPOOLED_URL=${{ postgres.DATABASE_URL }}
VALKEY_URL=${{ redis.REDIS_URL }}
NATS_URL=nats://${{ nats.RAILWAY_PRIVATE_DOMAIN }}:4222

DEPLOYMENT_GRADE=dev

AUTH_MODE=admin
AUTH_ADMIN_EMAIL=<template input>
AUTH_ADMIN_PASSWORD=<template input>
AUTH_JWT_SECRET=${{ secret(64) }}

WORKER_GRPC_AUTH_TOKEN=${{ secret(32) }}
SECRETS_ENCRYPTION_KEY=kek-v1:${{ secret(43, "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+/") }}=

PUBLIC_APP_URL=https://${{ caddy.RAILWAY_PUBLIC_DOMAIN }}
```

Optional LLM defaults. Do not include these in the public template form; add
them manually to the server service only when a deployment should start with
seeded provider defaults:

```env
DEFAULT_OPENAI_API_KEY=<manual server variable>
DEFAULT_ANTHROPIC_API_KEY=<manual server variable>
DEFAULT_GEMINI_API_KEY=<manual server variable>
```

## Worker Variables

```env
SERVER_GRPC_ADDRESS=${{ server.RAILWAY_PRIVATE_DOMAIN }}:9001
WORKER_GRPC_AUTH_TOKEN=${{ server.WORKER_GRPC_AUTH_TOKEN }}
```

## UI Variables

Set these in the template. They are fixed service configuration, not user
inputs.

```env
PORT=9100
HOSTNAME=0.0.0.0
```

## Cleanup Checklist

Remove these legacy or now-defaulted variables from the live Railway project
unless the deployment intentionally overrides them:

Server:

```env
ADDR
WORKER_GRPC_ADDR
API_PREFIX
RUST_LOG
FRONTEND_URL
AUTH_BASE_URL
```

Worker:

```env
WORKER_GRPC_ADDRESS
RUST_LOG
```

## Template Notes

- Generate the template from a validated Railway project rather than from the
  Compose file.
- Keep only `caddy` public. The server, worker, UI, Postgres, Redis, and NATS
  should stay on Railway private networking.
- Use template variables for `AUTH_JWT_SECRET` and `WORKER_GRPC_AUTH_TOKEN`.
- Generate `SECRETS_ENCRYPTION_KEY` as `kek-v1:` plus 32 random bytes encoded
  as standard base64.

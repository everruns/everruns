# Railway Template

This directory contains the repo-backed pieces for the Everruns Railway
template. Railway does not run Docker Compose directly, so the template maps
`examples/docker-compose-full.yaml` into individual Railway services.

## Service Shape

| Service | Source | Public | Notes |
| --- | --- | --- | --- |
| `postgres` | Railway PostgreSQL | No | Use Railway-managed Postgres. |
| `redis` | Railway Redis | No | Used as `VALKEY_URL` for distributed rate limiting. |
| `nats` | Docker image `nats:2-alpine` | No | Start with JetStream and a `/data` volume. |
| `server` | Docker image `ghcr.io/everruns/everruns-server:latest` | No | HTTP API on `9000`, worker gRPC on `9001`. |
| `worker` | Docker image `ghcr.io/everruns/everruns-worker:latest` | No | Scale replicas as needed. |
| `ui` | Docker image `ghcr.io/everruns/everruns-ui:latest` | No | Next.js server on `9100`. |
| `caddy` | GitHub repo root `railway/caddy` | Yes | Single public ingress. |

## Caddy Service

Create the Caddy service from this repository with root directory
`/railway/caddy`. Enable public HTTP networking only on this service.

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

ADDR=0.0.0.0:9000
WORKER_GRPC_ADDR=0.0.0.0:9001
API_PREFIX=/api
DEPLOYMENT_GRADE=dev
RUST_LOG=info

AUTH_MODE=admin
AUTH_ADMIN_EMAIL=<template input>
AUTH_ADMIN_PASSWORD=<template input>
AUTH_JWT_SECRET=${{ secret(64) }}

WORKER_GRPC_AUTH_TOKEN=${{ secret(32) }}
SECRETS_ENCRYPTION_KEY=kek-v1:${{ secret(43, "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+/") }}=

PUBLIC_APP_URL=https://${{ caddy.RAILWAY_PUBLIC_DOMAIN }}
FRONTEND_URL=https://${{ caddy.RAILWAY_PUBLIC_DOMAIN }}
AUTH_BASE_URL=https://${{ caddy.RAILWAY_PUBLIC_DOMAIN }}/api
```

Optional LLM defaults:

```env
DEFAULT_OPENAI_API_KEY=<template input>
DEFAULT_ANTHROPIC_API_KEY=<template input>
DEFAULT_GEMINI_API_KEY=<template input>
```

## Worker Variables

```env
WORKER_GRPC_ADDRESS=${{ server.RAILWAY_PRIVATE_DOMAIN }}:9001
WORKER_GRPC_AUTH_TOKEN=${{ server.WORKER_GRPC_AUTH_TOKEN }}
RUST_LOG=info
```

## UI Variables

```env
PORT=9100
HOSTNAME=0.0.0.0
```

## Template Notes

- Generate the template from a validated Railway project rather than from the
  Compose file.
- Keep only `caddy` public. The server, worker, UI, Postgres, Redis, and NATS
  should stay on Railway private networking.
- Use template variables for `AUTH_JWT_SECRET` and `WORKER_GRPC_AUTH_TOKEN`.
- Generate `SECRETS_ENCRYPTION_KEY` as `kek-v1:` plus 32 random bytes encoded
  as standard base64.

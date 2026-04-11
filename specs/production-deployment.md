# Production Deployment Specification

## Abstract

This specification is the aggregation point for Everruns production deployment. It does not restate subsystem specs. It defines the minimum production shape, the required reverse proxy contract, and the canonical references operators should follow for the details.

## Design Goals

Production deployment guidance should:

1. Give operators one place to orient quickly.
2. Keep deployment-critical contracts explicit.
3. Point to the real source-of-truth docs instead of duplicating them.
4. Make routing, security, and operational boundaries hard to misread.

## Ownership Boundary

- This spec owns the high-level production deployment contract.
- Subsystem specs own their own detailed behavior.
- SRE docs and runbooks own step-by-step operational procedures.

## Baseline Production Topology

Production deployment consists of these roles:

- Reverse proxy / ingress
- Server (control plane)
- Worker pool
- PostgreSQL
- Optional Valkey
- Optional NATS
- Optional UI

The server is the only public backend HTTP surface. Workers communicate with the control plane over gRPC only.

See:
- [`specs/architecture.md`](./architecture.md)
- [`docs/getting-started/architecture.md`](../docs/getting-started/architecture.md)
- [`docs/getting-started/docker-compose.md`](../docs/getting-started/docker-compose.md)

## Required Deployment Decisions

Every production deployment must make explicit decisions for:

- ingress / reverse proxy
- TLS termination
- database connectivity and TLS mode
- auth mode
- worker authentication
- secrets encryption keys
- observability and metrics exposure
- whether Valkey and NATS are enabled

See:
- [`docs/sre/environment-variables.md`](../docs/sre/environment-variables.md)
- [`specs/authentication.md`](./authentication.md)
- [`specs/encryption.md`](./encryption.md)
- [`specs/prometheus-metrics.md`](./prometheus-metrics.md)
- [`specs/otel-observability.md`](./otel-observability.md)
- [`specs/threat-model.md`](./threat-model.md)

## Production Reverse Proxy Setup

The reverse proxy is mandatory in production unless an equivalent platform ingress enforces the same routing contract.

### Route Contract

The proxy must preserve these public route groups:

- `/api/*` -> backend unchanged
- `/mcp` -> backend unchanged
- `/.well-known/*` -> backend unchanged
- `/health` -> backend
- `/api-doc/openapi.json` -> backend when exposed
- all other browser routes -> UI

Do not rewrite `/mcp` under `/api`. Do not rewrite `/.well-known/*` under `/api`.

### Transport Requirements

- TLS/HTTPS required for public traffic
- Disable proxy buffering for SSE responses under `/api/*`
- Preserve Host and standard forwarding headers
- Keep gRPC worker traffic off the public ingress path

### Operational Notes

- REST API base URL should stay under `/api`
- MCP OAuth endpoints stay at root: `/oauth/*`
- MCP endpoint stays at root: `/mcp`
- OAuth discovery metadata stays at root: `/.well-known/oauth-authorization-server`
- If the UI is deployed separately, it still must target the same public `/api` base and root-level `/oauth/*`, `/mcp`

Canonical references:
- [`specs/apis.md`](./apis.md)
- [`specs/mcp.md`](./mcp.md)
- [`docs/sre/environment-variables.md`](../docs/sre/environment-variables.md)

Concrete config examples:
- [`local/Caddyfile`](../local/Caddyfile)
- [`examples/docker-compose-full.yaml`](../examples/docker-compose-full.yaml)

## Security Minimums

Production deployments must satisfy these minimums:

- HTTPS enabled for public traffic
- `AUTH_JWT_SECRET` configured
- `WORKER_GRPC_AUTH_TOKEN` configured
- secrets encryption key configured
- explicit CORS policy when UI and API origins differ
- private network or equivalent isolation for PostgreSQL, worker gRPC, Valkey, and NATS

See:
- [`specs/threat-model.md`](./threat-model.md)
- [`specs/authentication.md`](./authentication.md)
- [`specs/encryption.md`](./encryption.md)

## Operations And Upgrades

For production operations, use the runbooks rather than this spec:

- authentication setup: [`docs/sre/runbooks/authentication.md`](../docs/sre/runbooks/authentication.md)
- durable mode setup: [`docs/sre/runbooks/durable-mode-setup.md`](../docs/sre/runbooks/durable-mode-setup.md)
- encryption key rotation: [`docs/sre/runbooks/encryption-key-rotation.md`](../docs/sre/runbooks/encryption-key-rotation.md)
- admin tasks and migrations: [`docs/sre/admin-container.md`](../docs/sre/admin-container.md)

Release and rollout references:
- [`specs/release-process.md`](./release-process.md)
- [`specs/shipping.md`](./shipping.md)
- [`specs/maintenance.md`](./maintenance.md)

## Non-Goals

This spec does not define:

- exact Kubernetes manifests
- exact Docker Compose commands
- cloud-vendor-specific load balancer configuration
- step-by-step migration or incident procedures

Those belong in operator docs, runbooks, or deployment-specific repositories.

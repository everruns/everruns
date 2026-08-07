---
type: Specification
title: "Production Deployment Specification"
description: "Production deployment aggregation and reverse proxy contract."
tags:
  - everruns
  - operations
---
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
- [`knowledge/foundations/architecture.md`](../foundations/architecture.md)
- [`docs/getting-started/architecture.md`](../../docs/getting-started/architecture.md)
- [`docs/getting-started/docker-compose.md`](../../docs/getting-started/docker-compose.md)

## Required Deployment Decisions

Every production deployment must make explicit decisions for:

- ingress / reverse proxy
- TLS termination
- database connectivity and TLS mode
- listener connectivity for PostgreSQL `LISTEN/NOTIFY` paths
- auth mode
- worker authentication
- secrets encryption keys
- system email provider and verified sender domain, if email is enabled
- observability and metrics exposure
- whether Valkey and NATS are enabled

See:
- [`docs/sre/environment-variables.md`](../../docs/sre/environment-variables.md)
- [`knowledge/security/authentication.md`](../security/authentication.md)
- [`knowledge/security/encryption.md`](../security/encryption.md)
- [`knowledge/operations/email.md`](email.md)
- [`knowledge/operations/prometheus-metrics.md`](prometheus-metrics.md)
- [`knowledge/operations/observability.md`](observability.md)
- [`knowledge/security/threat-model.md`](../security/threat-model.md)

### PostgreSQL Listener Connectivity

Production deployments may use a pooled `DATABASE_URL` for ordinary query traffic, but any server path that relies on PostgreSQL `LISTEN/NOTIFY` must use a direct session-scoped connection.

Canonical operator contract:

- `DATABASE_URL` may point at a pooler/proxy for normal queries
- `DATABASE_UNPOOLED_URL` is the canonical override for PostgreSQL listener traffic
- if the selected listener URL still points at an obvious pooler/proxy endpoint, startup must fail fast instead of silently accepting a broken deployment

Reasoning:

- PostgreSQL `LISTEN/NOTIFY` is session-scoped
- pooled/proxied endpoints can interleave notification frames with ordinary query traffic
- that failure mode is intermittent and looks like storage corruption or driver protocol errors even when the underlying writes are correct

NATS changes the requirement only partially:

- when NATS event delivery is active, the legacy PostgreSQL event wakeup listener is skipped
- PostgreSQL-backed notification SSE and PostgreSQL task-notification fallback still require a direct listener connection when those paths are active

See:
- [`knowledge/foundations/architecture.md`](../foundations/architecture.md)
- [`knowledge/operations/durable-execution-engine.md`](durable-execution-engine.md)
- [`knowledge/operations/notifications.md`](notifications.md)
- [`docs/sre/environment-variables.md`](../../docs/sre/environment-variables.md)

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
- Compress eligible non-streaming API responses with content negotiation and a
  minimum-size threshold; do not recompress already encoded payloads
- Disable proxy buffering for SSE responses under `/api/*`
- Exclude `text/event-stream` from response compression so events remain
  observable before stream completion
- Preserve Host and standard forwarding headers, including `X-Request-ID` (pass through unchanged)
- Keep gRPC worker traffic off the public ingress path

See [`knowledge/operations/correlation-ids.md`](correlation-ids.md) for the `X-Request-ID` contract: the server generates a UUID if the header is absent and always echoes it in the response. Stripping or rewriting it breaks client-injected request correlation.

### Operational Notes

- REST API base URL should stay under `/api`
- MCP OAuth endpoints stay at root: `/oauth/*`
- MCP endpoint stays at root: `/mcp`
- OAuth discovery metadata stays at root: `/.well-known/oauth-authorization-server`
- If the UI is deployed separately, it still must target the same public `/api` base and root-level `/oauth/*`, `/mcp`

### MCP Endpoint Scaling

`/mcp` is stateless request/response per JSON-RPC call (see [`knowledge/integrations/mcp.md`](../integrations/mcp.md)). It satisfies the MCP `2026-07-28` stateless model out of the box:

- **No sticky sessions, no shared session store.** There is no `Mcp-Session-Id` and no server-side per-connection state, so any MCP request can route to any backend instance. Plain round-robin behind any load balancer is sufficient; horizontal scaling is "add instances."
- **All state is in PostgreSQL** (sessions, OAuth clients/tokens, MCP server configs). Instances share one source of truth, so there are no read-after-write affinity requirements at the proxy.
- **Routing headers are honored, not required.** When clients send the `2026-07-28` `Mcp-Method` / `Mcp-Name` headers, gateways/load-balancers/rate-limiters may route and throttle on them without parsing the JSON-RPC body. The body stays authoritative; the server rejects duplicate routing-header values and any header that disagrees with the body. Proxies should reject or canonicalize duplicate `Mcp-Method` / `Mcp-Name` values before routing on them; forwarding singular headers unchanged enables body-free routing but is not mandatory.
- **SSE buffering.** `tools/call` responses may carry SSE-framed results from remote MCP servers; keep proxy buffering disabled on the `/mcp` route as for `/api/*`.

Canonical references:
- [`knowledge/execution/apis.md`](../execution/apis.md)
- [`knowledge/integrations/mcp.md`](../integrations/mcp.md)
- [`docs/sre/environment-variables.md`](../../docs/sre/environment-variables.md)

Concrete config examples:
- [`local/Caddyfile`](../../local/Caddyfile)
- [`examples/docker-compose-full.yaml`](../../examples/docker-compose-full.yaml)

## Security Minimums

Production deployments must satisfy these minimums:

- HTTPS enabled for public traffic
- `AUTH_JWT_SECRET` configured
- `WORKER_GRPC_AUTH_TOKEN` configured
- secrets encryption key configured
- `EMAIL_PROVIDER`, provider API key, and verified sender domain configured when deployment features send email
- explicit CORS policy when UI and API origins differ
- private network or equivalent isolation for PostgreSQL, worker gRPC, Valkey, and NATS

See:
- [`knowledge/security/threat-model.md`](../security/threat-model.md)
- [`knowledge/security/authentication.md`](../security/authentication.md)
- [`knowledge/security/encryption.md`](../security/encryption.md)

## Operations And Upgrades

For production operations, use the runbooks rather than this spec:

- authentication setup: [`docs/sre/runbooks/authentication.md`](../../docs/sre/runbooks/authentication.md)
- durable mode setup: [`docs/sre/runbooks/durable-mode-setup.md`](../../docs/sre/runbooks/durable-mode-setup.md)
- encryption key rotation: [`docs/sre/runbooks/encryption-key-rotation.md`](../../docs/sre/runbooks/encryption-key-rotation.md)
- admin tasks and migrations: [`docs/sre/admin-container.md`](../../docs/sre/admin-container.md)

Release and rollout references:
- [`knowledge/project/release-process.md`](../project/release-process.md)
- [`knowledge/project/shipping.md`](../project/shipping.md)
- [`knowledge/project/maintenance.md`](../project/maintenance.md)

## Non-Goals

This spec does not define:

- exact Kubernetes manifests
- exact Docker Compose commands
- cloud-vendor-specific load balancer configuration
- step-by-step migration or incident procedures

Those belong in operator docs, runbooks, or deployment-specific repositories.

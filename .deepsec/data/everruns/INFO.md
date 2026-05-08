# everruns

## What this codebase does

Everruns is a self-hostable durable agent runtime and management app. The Rust
control plane (`crates/server`, `crates/core`, `crates/worker`, integrations)
exposes Axum REST/SSE/MCP/app-channel endpoints backed by PostgreSQL or in-memory
dev storage. The UI (`apps/ui`) is a Next.js app that talks to the backend under
`/api`, manages auth/org state with React Query providers, and renders agent,
session, tool-call, file, model, MCP, app, eval, budget, and settings workflows.

## Auth shape

- `AuthUser`, `PlatformUser`, `ResolvedOrg`, `OrgContext`, `OrgAdmin`, and
  `OrgOwner` are Axum extractors in `crates/server/src/auth/middleware.rs`.
- `AuthState` wires `BuiltinAuthBackend`; credentials are JWT cookies, bearer
  JWTs, or `evr_` API keys. `AUTH_MODE=none` intentionally returns an anonymous
  owner/admin user for local/dev OSS mode.
- Tenant scoping usually flows through `ResolvedOrg` and then `Caller`; API key
  org selection is `X-Org-Id`, `everruns_org`, or single-org fallback.
- UI auth is cookie-based: `AuthProvider` fetches `/v1/auth/config` and
  `/v1/auth/me`; `OrgProvider` syncs `everruns_org` via
  `/v1/users/me/switch-org`; `api/client.ts` refreshes on 401.
- Route-level guards are mostly extractors inside handlers, not one global auth
  middleware. Public/anonymous routes must document their alternate auth gate.

## Threat model

Highest-impact failures are cross-organization data access, unauthorized app or
agent invocation, exposure of LLM/provider/API secrets, and writes into another
tenant's sessions, files, memory, volumes, schedules, budgets, or MCP servers.
Unauthenticated ingress exists by design for published apps, Slack, AG-UI, A2A,
webhooks, OAuth/MCP metadata, health, OpenAPI, and presigned worker image URLs;
those paths rely on per-channel tokens, Slack signing secrets, HMAC signatures,
publication state, method gates, and rate limits. Agent/tool outputs are
untrusted UI content and can contain markdown, generated UI, images, and file
metadata.

## Project-specific patterns to flag

- Handler uses an unscoped storage/domain lookup (`*_unscoped`,
  `get_by_public_id_unscoped`) without an explicit public-channel or internal
  signature gate.
- Handler accepts `SessionId`, `AgentId`, `AppId`, `ImageId`, `VolumeId`, or
  other prefixed IDs and does not bind the resource back to the current
  `ResolvedOrg`/`Caller` or authenticated app channel.
- Anonymous app ingress (`app_webhooks`, `app_a2a`, `ag_ui`, `slack_events`)
  reaches session/message creation before checking publication status, channel
  enabled state, per-channel token/key/signature, and method/rate gates.
- UI code renders agent/tool/markdown/OpenUI/A2UI/MCP-card content with HTML,
  iframe, or postMessage behavior outside the established renderers and
  sandbox/origin checks.
- Secret-bearing connection/provider/MCP/agent-identity code returns raw
  encrypted/plain API keys, headers, OAuth tokens, or webhook tokens instead of
  redacted/set-only response fields.

## Known false-positives

- `AUTH_MODE=none`, anonymous owner/admin, seed data, and `apps/ui/src/app/dev`
  fixtures are dev-mode behavior, not production auth bypasses.
- `GET /health`, `/api-doc/openapi.json`, OAuth/MCP metadata, and homepage
  discovery links are intended public endpoints.
- `/v1/apps/{app_id}/ag-ui`, `/ag-ui/images`, `/webhooks/{channel_id}`,
  `/a2a/{channel_id}`, and Slack event/manifest endpoints are public only for
  published apps/channels and must be judged by their app-channel gates.
- `/internal/images/{image_id}` deliberately has no user/org auth; access is
  via short-lived HMAC-signed URLs using `WORKER_GRPC_AUTH_TOKEN`.
- UI API clients in `apps/ui/src/lib/api/*` usually omit explicit auth headers
  because same-origin cookies carry `access_token` and `everruns_org`.

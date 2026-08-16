---
type: Specification
title: "App API Keys (execution-scoped)"
description: "App-scoped, execution-only API keys over the native session API."
tags:
  - everruns
  - integrations
---
# App API Keys (execution-scoped)

## Abstract

An **App API key** is an app-scoped, execution-only credential that lets an
external integrator drive an App's agent over native session routes mounted
under the App, without holding any management access. It is the programmatic
counterpart to a Personal Access Token (PAT): where a PAT is the **management
plane** (user-scoped, full account access, see
[`authentication.md`](../security/authentication.md)), an App API key is the **execution
plane** (one App, run sessions only).

This is the "new, separate concept" that `authentication.md` anticipates: a
narrow, non-human credential. It is deliberately **not** a PAT variant, PATs
stay user-scoped and full-access, and their dormant `scopes` column is not the
mechanism here.

The App is the boundary. The key inherits exactly what the App can do (its
Harness + Agent) and nothing else. Execution-only is **structural**: the key
authenticates only the app-mounted routes below and has no path to any
management API, there is no permission to widen, because there is no route to
widen onto.

## Goals

1. Hand an integrator a credential that can **run sessions for one App** and
   read their results, with no path to management APIs.
2. Reuse the proven app-mounted ingress pattern (A2A / AG-UI) rather than
   threading a new execution identity through the core auth and permission
   layers.
3. Extend the existing shared App-endpoint `api_key` auth scheme
   ([`app-endpoint-auth.md`](app-endpoint-auth.md)) rather than inventing new
   auth machinery.
4. Confine exposure: reads surface only the agent's completed (final) messages,
   never raw internal tool names, arguments, or results.
5. Make keys first-class: listable via the channel, rotatable, revocable,
   hashed at rest.

## Non-Goals

1. Org-scoped "service account that can run any agent." That contradicts the
   apps-as-boundary model; if ever needed it is a separate concept again.
2. Replacing A2A. A2A ([`a2a-channel.md`](a2a-channel.md)) exposes the App over
   the A2A JSON-RPC protocol for agent-to-agent callers. App API keys expose
   **native REST** session routes for first-party integrators. Both are
   app-scoped, execution-only ingress; they differ only in protocol shape and
   may coexist on the same App.
3. Projecting the global `/v1/sessions/*` management surface onto the key.
   Considered and rejected as the launch shape, see "Dismissed: project through
   /v1/sessions".
4. A raw event / SSE stream. Reads return completed assistant messages only.
   Streaming is a follow-up (see "Follow-ups").

## Concept

App API keys are modeled as an **App channel** of a new
`ChannelType::ApiEndpoint` (`"api_endpoint"`). Modeling it as a channel keeps it
uniform with `a2a` / `ag_ui` (own enabled flag, own config, own lifecycle,
published-app gate) and lets one App expose several keys with independent
settings.

Channel config (`ApiEndpointChannelConfig`, see `crates/platform/src/app.rs`)
carries:

- The generated key material, `api_key_hash` (SHA-256 hex) and non-secret
  `api_key_prefix` for display. Plaintext is returned **once** at create / rotate
  and never persisted; the hash is redacted on read. Mirrors A2A key handling.
- `session_mode: InvocationSessionMode`, `shared_session` vs
  `session_per_invocation`, reusing app-channel routing semantics.
- Optional `rate_limit_per_minute`, reusing the shared `ChannelRateLimiter`
  primitive (namespace `apikey`, disjoint from `a2a` / `agui` / `fcp`).
- Optional inline `auth`, when omitted, the generated per-channel API-key
  bearer scheme applies; otherwise any shared App-endpoint auth mode
  (OIDC / OAuth2 / HTTP Basic / mTLS).

Key format: `evr_app_<64 hex chars>` (32 random bytes, 256-bit entropy),
prefix-scoped so secret scanners target it distinctly from `evr_pat_` and
`evra2a_`.

## Endpoints (app-mounted, execution-only)

All routes are mounted under the App and authenticated by the `evr_app_` key,
the same shape as A2A's `/v1/apps/{app_id}/a2a/{channel_id}`. See
`crates/server/src/api/app_api.rs`.

| Method | Path | Behavior |
|--------|------|----------|
| `POST` | `/v1/apps/{app_id}/api/{channel_id}/sessions` | Create (or, for `shared_session`, resolve) the app-owned session and dispatch the body `message`, starting a turn. → `{ session_id, status, created_session }`. |
| `POST` | `/v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}/messages` | Dispatch a follow-up `message` into an existing **app-owned** session. → `202 { session_id, status }`. |
| `GET` | `/v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}` | Derived status (`submitted` / `working` / `completed` / `failed` / `canceled`) plus the agent's **completed** messages. |
| `POST` | `/v1/apps/{app_id}/api/{channel_id}/sessions/{session_id}/cancel` | Cancel the in-flight turn; emits `turn.cancelled`. → `{ session_id, status: "canceled" }`. |

Auth runs through the shared App-endpoint verifier
([`app-endpoint-auth.md`](app-endpoint-auth.md)) after published-app /
enabled-channel resolution and before any session work. Only **published** Apps
with an **enabled** `api_endpoint` channel accept traffic; unpublish / disable
stops new work, existing sessions remain. Failures are generic 401 / 403 / 404
so callers cannot distinguish misconfiguration from probing.

The session created on `POST .../sessions` is bound to the App's Harness + Agent
(the caller cannot pick an arbitrary agent or pass management-only fields). It
carries the standard app-channel routing tags, including a distinct
`app_channel_type:api_endpoint` tag so per-channel-type metrics and rate-limit
buckets stay disjoint from `a2a` / `ag_ui`.

## Confinement

Every read / message / cancel against a `{session_id}` checks that the session
carries this channel's `app:{app_id}` + `app_channel:{channel_id}` routing tags;
otherwise it returns `404` (not `403`, to avoid cross-app existence probing). So
one App's key cannot read or drive another App's sessions even if a session id
leaks. This mirrors A2A's `session_belongs_to_a2a_channel` guard
(`THREAT[TM-APIKEY-002]`). Confinement is an imperative ownership check in the
execution path, not a `Policy` `Rule`, because policy evaluation sees only the
`Caller`, not the target resource, and these routes self-authenticate via the
key rather than going through the `Caller`/permission layer.

## Exposure projection

Reads surface only **completed, non-Commentary assistant messages**
(`output.message.completed` events) and a derived turn status. Raw tool names,
arguments, results, and internal event bodies are never returned to the key
(`THREAT[TM-APIKEY-004]`). This is the same safe projection AG-UI applies to
public streams, achieved here by returning final messages rather than a raw
event feed.

## Management surfaces

Key lifecycle lives under the App, gated by the App-management policy
(`APP_MANAGE`, i.e. a human / PAT), a key cannot mint, rotate, or read sibling
keys:

- `POST /v1/apps/{app_id}/api-endpoint-channels`, create channel, return
  plaintext key once.
- `POST /v1/apps/{app_id}/api-endpoint-channels/{channel_id}/regenerate-key`,
  rotate; invalidates the previous key.
- `PATCH /v1/apps/{app_id}/channels/{channel_id}`, update non-secret fields
  (session mode, rate limit, `auth`); the key hash is preserved across edits.
- `DELETE /v1/apps/{app_id}/channels/{channel_id}`, remove.

## Audit logging

Reuses the shared app-channel invocation audit path
([`a2a-channel.md`](a2a-channel.md) "Audit Logging"): on dispatch emit
`agent` / `agent.app_invocation.started` with `source = "app_api_endpoint"`,
`app_id`, `app_channel_id`, `app_channel_type = "api_endpoint"`, `session_id`,
`created_session`, and the App owner principal id. Actor is none (external
key-holder, not an Everruns user).

## Threat model

See `knowledge/security/threat-model.md` (TM-APIKEY-* category):

- `TM-APIKEY-001`, key storage / verification (hashed at rest, constant-time
  compare, plaintext once, rotation invalidates prior keys).
- `TM-APIKEY-002`, cross-App isolation (tag-based confinement; 404 on foreign
  session).
- `TM-APIKEY-003`, runaway-traffic DoS (per-channel, per-IP rate limit).
- `TM-APIKEY-004`, exposure leakage (reads return only completed assistant
  messages).

## Dismissed: project through /v1/sessions

An earlier draft proposed making the existing `/v1/sessions/*` management
endpoints accept the `evr_app_` key by threading an app-execution identity
through the core auth middleware, the `Caller` / permission resolver, and every
session command policy (a new `OrgSessionsExecute` permission + a confinement
`Rule`). Rejected for the launch: high blast radius across the shared auth path
for the same user-visible capability that the app-mounted routes provide with
near-zero blast radius, reusing the proven A2A / AG-UI pattern. The native REST
shape under the App is also self-confining and self-redacting. Revisit only if a
concrete need arises to expose the full management session surface to execution
keys.

## Follow-ups

- Streaming reads (SSE) projected through the same completed-message exposure
  policy, mirroring AG-UI's public stream.
- Optional UI for managing `api_endpoint` channels (create / rotate / disable).

## References

- [`apps.md`](apps.md), App entity, channels, lifecycle
- [`app-endpoint-auth.md`](app-endpoint-auth.md), shared inbound auth verifier
- [`a2a-channel.md`](a2a-channel.md), sibling execution-only channel (A2A protocol)
- [`authentication.md`](../security/authentication.md), PATs and the management plane
- [`apis.md`](../execution/apis.md), native session API surface

---
type: Specification
title: "A2A Channel"
description: "A2A inbound channel."
tags:
  - everruns
  - integrations
---
# A2A Channel

## Abstract

The A2A channel exposes an Everruns App as an **Agent2Agent (A2A) protocol**
endpoint so other agents can invoke the app over JSON-RPC. It is a sibling of
the `webhook` channel: app-scoped ingress that injects a rendered user message
into an app-owned session, with a published-app + enabled-channel gate.

The first cut implemented the **API key** authentication scheme from the A2A
security model. A2A channels now also adopt the shared App endpoint auth model
documented in [`knowledge/integrations/apps.md`](apps.md): each channel may keep the generated
bearer API key or attach inline `channel_config.auth` for HTTP Basic,
Google/OIDC JWT bearer, OAuth2 introspection, or mTLS.

A2A is a separate `ChannelType` so an app can advertise itself as an agent to
other agents without conflating it with bare HTTP webhook ingress.

References:

- A2A protocol: <https://a2aproject.github.io/A2A>
- `knowledge/integrations/app-invocation-channels.md`, sibling invocation channels
- `knowledge/integrations/apps.md`, app entity, harness, agent identity binding
- `crates/server/specs/slack-integration.md`, sibling messaging channel

## Goals

1. Let an Everruns App act as a discoverable A2A agent for other agents
2. Reuse the existing App lifecycle, ownership, harness, agent, and identity
3. Authenticate inbound calls with a hashed API key (no plaintext at rest)
4. Reuse `InvocationSessionMode` for session routing (shared / per-invocation)
5. Publish a minimal **Agent Card** for protocol discovery

## Non-Goals

1. The full A2A method surface, this iteration canonically supports
   `message/send`, `message/stream`, `tasks/get`, and `tasks/cancel`.
   `tasks/resubscribe`, push notifications, and authenticated extensions
   remain out of scope.
2. Persistent per-task identity beyond the session lifecycle. Tasks are
   identified by the underlying session id (`task_id == contextId`); a
   shared session reuses the same task id across follow-up messages.

This spec covers **inbound** A2A only, exposing an Everruns App as an A2A
server for other agents to call. The complementary **outbound** direction,
letting an Everruns agent call an external A2A agent, lives as a separate
capability spec, [`knowledge/integrations/a2a-capability.md`](a2a-capability.md), and a
separate threat-model surface (TM-AGENT-005 covers the high-risk capability
gate; SSRF protection comes from `validate_safe_url`, response size is
bounded by `MAX_RESULT_CHARS`, and external agent IDs are configured rather
than model-supplied).

## Model

A new `ChannelType::A2a` (`"a2a"`) variant. Configuration:

```rust
pub struct A2aChannelConfig {
    /// SHA-256 hex digest of the API key. Plaintext is never stored.
    pub api_key_hash: String,
    /// Public, non-secret display prefix (e.g. `evra2a_abc1...`) for the UI.
    pub api_key_prefix: String,
    /// Whether invocations reuse a stable session or create a new one.
    pub session_mode: InvocationSessionMode,
    /// Message template rendered into the session on each invocation.
    pub message: String,
    /// Optional human-readable agent name surfaced in the Agent Card.
    pub agent_card_name: Option<String>,
    /// Optional human-readable description surfaced in the Agent Card.
    pub agent_card_description: Option<String>,
    /// Optional per-IP rate limit applied to this app's A2A endpoint, in
    /// requests per minute. `None` or `Some(0)` disables the per-channel
    /// limit; the global API limit still applies. Mirrors
    /// `AgUiChannelConfig::rate_limit_per_minute`.
    pub rate_limit_per_minute: Option<u32>,
    /// Optional inline endpoint auth config. When absent, the generated
    /// API-key bearer scheme remains the effective auth policy.
    pub auth: Option<AppEndpointAuthConfig>,
}
```

Storage uses the existing `app_channels` row schema. The migration extends the
`channel_type` CHECK constraint to allow `'a2a'`.

API key generation:

- Format: `evra2a_<64 hex chars>`, 32 random bytes (256-bit entropy),
  prefix-scoped so secret scanners can target A2A keys distinctly from
  platform `evr_` API keys.
- Hash: `SHA-256` of the full key, hex-encoded. Matches `auth/api_key.rs`.
- Display prefix: first 8 hex chars after `evra2a_`, suffixed with `...`.
- Plaintext returned **only once**: in the `AddA2aChannel` / regenerate
  command response. Subsequent reads expose only `api_key_prefix`.

## Endpoints

### Inbound JSON-RPC

`POST /v1/apps/{app_id}/a2a/{channel_id}`

- Content-Type: `application/json`
- Auth: `Authorization: Bearer <api_key>` (the `apiKey` scheme in the Agent
  Card uses `bearer` for unification with the standard HTTP header).
- If `auth` is configured, the endpoint uses that shared App endpoint auth
  policy instead. `auth.mode = api_key` keeps generated-key behavior;
  `google_oidc`, `oidc`, and `oauth2_introspection` use bearer tokens;
  `http_basic` uses HTTP Basic; `mtls` uses the configured trusted reverse
  proxy identity header.
- Body: A2A JSON-RPC 2.0 envelope. Canonical methods are `message/send`,
  `message/stream`, `tasks/get`, and `tasks/cancel`. For compatibility with
  linked clients that still emit legacy method names, the endpoint accepts
  `SendMessage`, `SendStreamingMessage`, `GetTask`, and `CancelTask` as aliases
  for the canonical methods.

```json
{
  "jsonrpc": "2.0",
  "id": "req-1",
  "method": "message/send",
  "params": {
    "message": {
      "role": "user",
      "parts": [{ "kind": "text", "text": "Hello" }],
      "messageId": "msg-1"
    }
  }
}
```

Response: A2A JSON-RPC 2.0 success with a non-terminal `Task` result:

```json
{
  "jsonrpc": "2.0",
  "id": "req-1",
  "result": {
    "id": "<session_id>",
    "contextId": "<session_id>",
    "status": { "state": "submitted" },
    "kind": "task"
  }
}
```

The task `id` is the underlying Everruns `SessionId`, it is intentionally
the same value as `contextId`. The durable workflow is asynchronous, so the
initial response is always non-terminal (`submitted`). Clients observe
state transitions via `tasks/get` (see "Task Lifecycle" below) or
`message/stream` (see "Streaming"). Shared sessions reuse the same task id
across follow-up messages.

Error mapping. Transport-level failures use plain HTTP errors; protocol-level
failures use JSON-RPC error envelopes returned with HTTP 200 so A2A clients
that key off the JSON-RPC `id` and `error.code` see a structured response:

| HTTP | JSON-RPC code | Reason                                |
|------|---------------|---------------------------------------|
| 401  |, | Missing or invalid API key            |
| 403  |, | App not published or channel disabled |
| 404  |, | App or channel not found              |
| 400  |, | Invalid path-level input (e.g. malformed channel ID) |
| 400  | `-32600`      | Invalid Request (malformed envelope, returned with HTTP 400) |
| 200  | `-32601`      | Method not found (only canonical `message/send`, `message/stream`, `tasks/get`, `tasks/cancel` and their legacy linked-client aliases are supported) |
| 200  | `-32602`      | Invalid params (e.g. no non-empty text parts, malformed task id) |
| 200  | `-32001`      | Task not found (`tasks/get` / `tasks/cancel` against an unknown task id) |

### Streaming (`message/stream`)

The same endpoint accepts `method = "message/stream"`. Authentication, channel
gating, method allowlist, and `params.message.parts` validation are identical
to `message/send`. On success the response is `Content-Type:
text/event-stream` instead of `application/json`, and the body is a sequence
of SSE events. Each event's `data:` payload is a JSON-RPC 2.0 envelope whose
`id` echoes the request `id` and whose `result` carries one A2A streaming
frame.

Frame kinds emitted:

- `status-update` with `status.state = "working"` and `final = false`, sent
  immediately after the session is resolved so clients see liveness even
  before the durable runtime emits its first event.
- `message` with `role = "agent"` and `parts: [{ kind: "text", text: ... }]`
, emitted from `output.message.completed` events for the same session.
  Tool calls and intermediate deltas are not surfaced in this iteration.
- A terminal `status-update` with `final = true` and one of
  `state = "completed" | "failed" | "canceled"`, emitted from the
  corresponding `turn.completed` / `turn.failed` / `turn.cancelled` event.
  The stream closes after this frame.

If the session subscription drops without a terminal turn event, the channel
emits a synthetic `status-update` with `state = "failed"` and `final = true`
so clients do not hang. Reconnection / replay (`tasks/resubscribe`,
`Last-Event-ID`) is not supported in this iteration; clients that lose the
stream should retry the original `message/stream` call.

Authentication, gating, and validation errors before the stream opens follow
the same JSON-RPC error envelope as `message/send` (returned as a normal
`application/json` response with HTTP 200 and `error.code` set, except for
401/403/404/400 transport-level failures which are returned as plain HTTP
errors).

### Task Lifecycle (`tasks/get`, `tasks/cancel`)

The same endpoint accepts `method = "tasks/get"` and `method = "tasks/cancel"`
with `params = { "id": "<task_id>" }`. Authentication, channel gating, and
method allowlisting are identical to `message/send`.

Task identity. The task id returned by `message/send` and `message/stream`
is the underlying Everruns `SessionId`. The state of a task is **derived**
from the most recent turn lifecycle event on that session; there is no
separate task table in this iteration:

| Latest turn event       | A2A state    |
|-------------------------|--------------|
| `turn.completed`        | `completed`  |
| `turn.failed`           | `failed`     |
| `turn.cancelled`        | `canceled`   |
| `turn.started`          | `working`    |
| (no turn events yet)    | `submitted`  |

`tasks/get` returns the current task with `id` and `contextId` echoing the
session id. An unknown but well-formed task id surfaces `-32001 Task not
found`. A malformed task id surfaces `-32602 Invalid params`.

**Structured result artifact.** When the underlying session reported a
deterministic, schema-bound result (`result.json`, produced by a task declared
with a `result_schema`, see [`knowledge/runtime-resources/subagents.md`](../runtime-resources/subagents.md) and
[`knowledge/runtime-resources/session-tasks.md`](../runtime-resources/session-tasks.md)), `tasks/get` surfaces that JSON as
an A2A `Artifact` on the task rather than leaving the caller to parse
last-message text:

```json
"artifacts": [
  {
    "artifactId": "result",
    "name": "result",
    "parts": [{ "kind": "data", "data": { "...": "the result.json object" } }]
  }
]
```

The artifact is present only when a result was reported; a plain agent turn
returns the task with no `artifacts`. When a session reported more than one
structured result the most recently updated one wins. Retrieval is org-scoped
and further fenced by the same channel-binding check as the rest of `tasks/get`
(TM-A2A-012): the artifact for a session created by one channel is never
returned to an API key for a different channel, even within the same org, the
cross-channel lookup collapses to `-32001 Task not found` with no artifact leak.

`tasks/cancel` cancels the in-flight durable workflow for the underlying
session and emits a `turn.cancelled` event so subsequent `tasks/get` calls
observe the canceled state. Cancelling an already-terminal task is
idempotent: the same terminal task object is returned without any state
transition.

Reusing the session id as the task id is acceptable for the in-channel
contract because:

- Each shared-session invocation runs at most one durable turn at a time,
  so there is exactly one in-flight task per session and "the most recent
  turn" is unambiguous.
- The contract advertised in the Agent Card does not promise per-message
  task identity; clients that need that should use `message/stream`.
- Persistent multi-task state across the lifetime of a session is a
  separate concern that belongs to a future iteration if required.

### Agent Card

`GET /v1/apps/{app_id}/a2a/{channel_id}/.well-known/agent-card.json`

Unauthenticated. Returns the published Agent Card so other agents can
discover the endpoint. Only returned for **published** apps with **enabled**
A2A channels; otherwise `404`. Card shape:

```json
{
  "name": "<agent_card_name | app.name>",
  "description": "<agent_card_description | app.description>",
  "version": "0.1",
  "supportedInterfaces": [
    {
      "url": "<absolute endpoint URL>",
      "protocolBinding": "JSONRPC",
      "protocolVersion": "1.0"
    }
  ],
  "capabilities": {
    "streaming": true,
    "pushNotifications": false,
    "stateTransitionHistory": false
  },
  "defaultInputModes":  ["text/plain"],
  "defaultOutputModes": ["text/plain"],
  "skills": [
    {
      "id": "default",
      "name": "<app.name>",
      "description": "<app.description>",
      "tags": ["everruns", "a2a"]
    }
  ],
  "securitySchemes": { "...": "derived from channel_config.auth" },
  "securityRequirements": [{ "...": [] }]
}
```

The Agent Card derives `securitySchemes` from the effective channel auth policy:
legacy/default API key emits HTTP bearer; HTTP Basic emits `http/basic`; Google
and generic OIDC emit `openIdConnect`; OAuth2 introspection emits generic HTTP
bearer because the A2A schema requires concrete token flows for OAuth2 schemes;
mTLS emits `mutualTLS`. The card never includes credentials, hashes, trusted
header values, or the channel internal id.

## Session Routing

`InvocationSessionMode` determines session reuse. Routing tags reuse the
shared app-channel set:

- `app:{app_id}`
- `app_channel:{channel_id}`
- `app_channel_type:a2a`
- `__internal:app_invocation`
- per-invocation mode adds `app_invocation:{uuid}`

Template context:

- `app.id`, `app.name`
- `channel.id`, `channel.type` (`"a2a"`)
- `invocation.source` (`"a2a"`), `invocation.triggered_at`
- `payload`, the A2A `params` object verbatim
- `a2a.text`, concatenated text parts of `params.message.parts` (joined with
  newlines), for cheap default templates like `{{a2a.text}}`
- `a2a.message_id`, `a2a.task_id`, `a2a.context_id`, protocol identifiers
- `a2a.role`, `params.message.role`

If no `text` part is present the channel rejects the request with
`-32602 Invalid params`.

## Audit Logging

A2A uses the shared app-channel invocation path with webhook and schedule
channels. After a request successfully resolves a session and dispatches the
rendered user message, the server emits one audit log entry:

- domain/action: `agent` / `agent.app_invocation.started`
- target: `app_channel:{channel_id}`
- actor: none, because the caller is an external API-key holder rather than an
  Everruns user
- metadata: `source = "app_a2a"`, `app_id`, `app_channel_id`,
  `app_channel_type = "a2a"`, `session_id`, `created_session`, and the app
  owner principal id; `agent_identity_id` is also present when the invocation
  runs through an agent identity

This mirrors webhook/schedule coverage because the event is emitted by the
common app-channel invocation helper, not by the A2A HTTP adapter.

## Lifecycle

- Publish/unpublish controls whether the endpoint accepts traffic.
- Disabling the channel rejects further requests with `403`.
- Deleting the channel removes the row; sessions previously created remain.
- Updating the API key replaces `api_key_hash` and `api_key_prefix` and
  invalidates previously issued keys.

## Surfaces

A2A channels are reachable through the same surfaces as other channels:

- HTTP app APIs:
  - `POST /v1/apps/{id}/a2a-channels`, create channel + return plaintext key once
  - `POST /v1/apps/{id}/a2a-channels/{channel_id}/regenerate-key`, rotate key
  - `PATCH /v1/apps/{id}/channels/{channel_id}`, update non-secret fields
  - `DELETE /v1/apps/{id}/channels/{channel_id}`, delete channel
- MCP/bash command catalog: `add_a2a_app_channel` flat command, plus
  `update_app_channel` / `delete_app_channel`
- `platform_management` capability (`manage_app_channels`)
- Apps UI (`/apps/{id}` detail page → channels list → A2A entry)

## Testing

Coverage required:

1. Channel CRUD: create generates an API key, only the hash and prefix are
   stored, rotation replaces both.
2. Inbound `message/send` returns a terminal completed task and creates a
   user message in the routed session.
3. Shared-session vs per-invocation routing.
4. Auth: missing / wrong / disabled / unpublished all return the documented
   JSON-RPC error codes.
5. Agent Card returns 404 when the app is unpublished or the channel is
   disabled, and returns the documented shape when live.
6. Method gating: `message/stream`, `tasks/get`, etc. return `-32601 Method
   not found`.
7. Validation: empty text parts rejected; non-`message/send` methods
   rejected; malformed envelopes rejected.
8. Structured result: a session that reported a schema-bound `result.json`
   surfaces it as the `tasks/get` artifact `DataPart`; a session with no
   reported result has no `artifacts`; the artifact is not leaked to a
   different channel's API key (cross-channel lookup stays `-32001`).

## Rate Limiting

Per-channel rate limiting bounds unattended agent-to-agent traffic that the
global API limit alone does not, a runaway counterparty agent should not be
able to drain an app's quota or LLM budget while it is unattended.

- Field: `A2aChannelConfig::rate_limit_per_minute: Option<u32>`. `None` or
  `Some(0)` disables the per-channel cap; positive values cap requests
  per minute, per source IP, per app.
- Server validates the field at write time: cap at `1_000_000` to prevent a
  typo from silently disabling the limiter.
- The check runs **after** API key verification so an unauthenticated
  caller cannot grow the limiter cache or learn whether a channel exists
  from rate-limit signals.
- Implementation: shared `ChannelRateLimiter` primitive
  (`crates/server/src/api/channel_rate_limit.rs`) with two backends,
  in-memory (governor) for single-instance/dev, Valkey for distributed.
  The same primitive backs the AG-UI channel; namespace strings (`agui`
  vs `a2a`) keep Valkey keys disjoint and separate `ChannelRateLimiter`
  instances keep in-memory buckets disjoint.
- Scope: A2A passes `app_id:channel_id` (not just `app_id`) so an app that
  exposes multiple A2A channels with different `rate_limit_per_minute`
  settings keeps independent buckets. Sharing an `app_id`-only key would
  let an attacker alternate between channels with different limits to
  flush the cached limiter (replace-on-limit-change) and bypass the
  stricter cap. AG-UI keeps the `app_id` scope because there is at most
  one AG-UI channel per app.
- 429 response: HTTP 429 with `ErrorResponse` body (`{"error": "A2A rate
  limit exceeded for this app channel"}`); Agent Card discovery is
  intentionally not rate-limited.

Threat coverage: TM-A2A-013 (DoS via runaway A2A client) is mitigated by
this control. See `knowledge/security/threat-model.md` for the full entry.

## Replay Protection (opt-in)

`A2aChannelConfig::signing_secret` opts a channel into Slack-style HMAC
request signing. When set, every request must additionally carry a
timestamp + signature header pair; otherwise the channel keeps the
existing authentication-only behavior. This closes TM-A2A-010
(captured-request replay until rotation) without breaking deployments
that have not opted in. Signing is **orthogonal** to the inline endpoint
auth (`channel_config.auth`), it layers replay protection on top of
whichever auth mode the channel uses (default API key, HTTP Basic, OIDC,
OAuth2, or mTLS).

Headers (sent by the client):

- `X-Everruns-A2A-Timestamp`, unix-second timestamp of the request.
- `X-Everruns-A2A-Signature`, `v0={hex}`, where `{hex}` is
  `HMAC-SHA256(signing_secret, "v0:{timestamp}:{channel_scope}:{raw_body}")`.
  `channel_scope` is the literal string `{app_id}:{channel_id}` (the same
  values that appear in the request path). Including the scope inside the
  signed basestring binds the signature to its target endpoint and
  prevents cross-channel replay when operators share the same
  `signing_secret` across multiple A2A channels.

Verification is performed in `crates/server/src/api/a2a_signing.rs` and
called from `app_a2a::authenticate_request` **after** primary
authentication so unauthenticated callers cannot probe channel existence
from signing-related signals or grow the in-memory replay store. The
check covers:

- **Timestamp window**: 300 seconds (5 minutes), symmetric (rejects both
  too-old and too-future stamps so a fast client clock cannot bypass
  replay).
- **Signature**: constant-time HMAC-SHA256 comparison.
- **Replay dedup**: the verified signature itself is recorded as a
  single-use nonce (scoped per `app_id:channel_id`) with a 5-minute TTL.
  Because the signature is deterministic over
  `(timestamp, channel_scope, body, secret)`, two requests with
  byte-identical bodies sent in the same unix second to the same channel
  produce the same HMAC and the second is rejected as a replay. This
  matches the Slack precedent and is benign in practice because JSON-RPC
  clients vary `id` per request, which makes the body distinct. Clients
  that may legitimately repeat identical payloads (e.g. unkeyed
  notifications) must either include a per-request token in the body or
  bump the timestamp by at least one second between retries. Two
  backends mirror the rate limiter, in-memory `HashMap` for
  single-instance/dev (with threshold-triggered TTL pruning) and Valkey
  `SET ... NX EX` for distributed deployments. The per-channel rate
  limiter runs **before** the nonce-record path, so rate-limited
  traffic does not grow the replay store.
- **Failure modes** (missing-header, stale, mismatch, replay) all
  collapse to a single 401 response so a remote attacker cannot
  distinguish them.

The plaintext secret is **write-only**: stored encrypted at rest via
the existing `channel_config` envelope encryption, redacted on read with
a `signing_secret_configured: bool` flag, and preserved across PATCH so
editing an unrelated field cannot accidentally turn off replay
protection.

The Agent Card advertises a vendor extension `everrunsHmacSignature` in
`securitySchemes` when signing is enabled, so other agents that
recognise it know to compute the signing headers in addition to the
primary authentication scheme.

## Threat Model

See `knowledge/security/threat-model.md` (TM-A2A-* category) for inbound-auth, key
storage, replay, method-gate, and runaway-traffic threats.

# A2A Channel

## Abstract

The A2A channel exposes an Everruns App as an **Agent2Agent (A2A) protocol**
endpoint so other agents can invoke the app over JSON-RPC. It is a sibling of
the `webhook` channel: app-scoped ingress that injects a rendered user message
into an app-owned session, with a published-app + enabled-channel gate.

This first cut implements the **API key** authentication scheme from the A2A
security model. Other schemes (HTTP Basic, OAuth2, OIDC, mTLS) are out of
scope for this iteration.

A2A is a separate `ChannelType` so an app can advertise itself as an agent to
other agents without conflating it with bare HTTP webhook ingress.

References:

- A2A protocol: <https://a2aproject.github.io/A2A>
- `specs/app-invocation-channels.md` — sibling invocation channels
- `specs/apps.md` — app entity, harness, agent identity binding
- `crates/server/specs/slack-integration.md` — sibling messaging channel

## Goals

1. Let an Everruns App act as a discoverable A2A agent for other agents
2. Reuse the existing App lifecycle, ownership, harness, agent, and identity
3. Authenticate inbound calls with a hashed API key (no plaintext at rest)
4. Reuse `InvocationSessionMode` for session routing (shared / per-invocation)
5. Publish a minimal **Agent Card** for protocol discovery

## Non-Goals

1. The full A2A method surface — this iteration supports `message/send` and
   `message/stream`. `tasks/get`, `tasks/cancel`, push notifications, etc.
   remain out of scope.
2. Authentication schemes beyond API key (HTTP, OAuth2, OIDC, mTLS).
3. Multi-turn task state machine — every `message/send` and `message/stream`
   call returns a terminal task (`completed`, `failed`, or `canceled`) with
   no follow-up `tasks/*` handle yet.

This spec covers **inbound** A2A only — exposing an Everruns App as an A2A
server for other agents to call. The complementary **outbound** direction —
letting an Everruns agent call an external A2A agent — lives as a separate
capability spec, [`specs/a2a-capability.md`](a2a-capability.md), and a
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
}
```

Storage uses the existing `app_channels` row schema. The migration extends the
`channel_type` CHECK constraint to allow `'a2a'`.

API key generation:

- Format: `evra2a_<64 hex chars>` — 32 random bytes (256-bit entropy),
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
- Body: A2A JSON-RPC 2.0 envelope. Only `message/send` is honored:

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

Response: A2A JSON-RPC 2.0 success with a terminal `Task` result:

```json
{
  "jsonrpc": "2.0",
  "id": "req-1",
  "result": {
    "id": "<task_id>",
    "contextId": "<session_id>",
    "status": { "state": "completed" },
    "kind": "task"
  }
}
```

The task `id` is a fresh UUID per invocation; `contextId` is the Everruns
`SessionId` (so subsequent A2A calls referencing the same `contextId` can be
correlated for debugging — Everruns does not require it).

Error mapping. Transport-level failures use plain HTTP errors; protocol-level
failures use JSON-RPC error envelopes returned with HTTP 200 so A2A clients
that key off the JSON-RPC `id` and `error.code` see a structured response:

| HTTP | JSON-RPC code | Reason                                |
|------|---------------|---------------------------------------|
| 401  | —             | Missing or invalid API key            |
| 403  | —             | App not published or channel disabled |
| 404  | —             | App or channel not found              |
| 400  | —             | Invalid path-level input (e.g. malformed channel ID) |
| 400  | `-32600`      | Invalid Request (malformed envelope, returned with HTTP 400) |
| 200  | `-32601`      | Method not found (only `message/send` and `message/stream` are supported) |
| 200  | `-32602`      | Invalid params (e.g. no non-empty text parts) |

### Streaming (`message/stream`)

The same endpoint accepts `method = "message/stream"`. Authentication, channel
gating, method allowlist, and `params.message.parts` validation are identical
to `message/send`. On success the response is `Content-Type:
text/event-stream` instead of `application/json`, and the body is a sequence
of SSE events. Each event's `data:` payload is a JSON-RPC 2.0 envelope whose
`id` echoes the request `id` and whose `result` carries one A2A streaming
frame.

Frame kinds emitted:

- `status-update` with `status.state = "working"` and `final = false` — sent
  immediately after the session is resolved so clients see liveness even
  before the durable runtime emits its first event.
- `message` with `role = "agent"` and `parts: [{ kind: "text", text: ... }]`
  — emitted from `output.message.completed` events for the same session.
  Tool calls and intermediate deltas are not surfaced in this iteration.
- A terminal `status-update` with `final = true` and one of
  `state = "completed" | "failed" | "canceled"` — emitted from the
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

### Agent Card

`GET /v1/apps/{app_id}/a2a/{channel_id}/.well-known/agent-card.json`

Unauthenticated. Returns the published Agent Card so other agents can
discover the endpoint. Only returned for **published** apps with **enabled**
A2A channels; otherwise `404`. Card shape:

```json
{
  "name": "<agent_card_name | app.name>",
  "description": "<agent_card_description | app.description>",
  "url": "<absolute endpoint URL>",
  "protocolVersion": "0.3.0",
  "version": "0.1",
  "preferredTransport": "JSONRPC",
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
  "securitySchemes": {
    "apiKey": { "type": "http", "scheme": "bearer" }
  },
  "security": [{ "apiKey": [] }]
}
```

The Agent Card never includes the API key, the key hash, or the channel
internal id.

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
- `payload` — the A2A `params` object verbatim
- `a2a.text` — concatenated text parts of `params.message.parts` (joined with
  newlines), for cheap default templates like `{{a2a.text}}`
- `a2a.message_id`, `a2a.task_id`, `a2a.context_id` — protocol identifiers
- `a2a.role` — `params.message.role`

If no `text` part is present the channel rejects the request with
`-32602 Invalid params`.

## Lifecycle

- Publish/unpublish controls whether the endpoint accepts traffic.
- Disabling the channel rejects further requests with `403`.
- Deleting the channel removes the row; sessions previously created remain.
- Updating the API key replaces `api_key_hash` and `api_key_prefix` and
  invalidates previously issued keys.

## Surfaces

A2A channels are reachable through the same surfaces as other channels:

- HTTP app APIs:
  - `POST /v1/apps/{id}/a2a-channels` — create channel + return plaintext key once
  - `POST /v1/apps/{id}/a2a-channels/{channel_id}/regenerate-key` — rotate key
  - `PATCH /v1/apps/{id}/channels/{channel_id}` — update non-secret fields
  - `DELETE /v1/apps/{id}/channels/{channel_id}` — delete channel
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

## Threat Model

See `specs/threat-model.md` (TM-A2A-* category) for inbound-auth, key
storage, replay, and method-gate threats.

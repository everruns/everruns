---
title: Agent API
description: Call one agent from your own code with an agent key. Reference for its base URL, keys, routes, auth, channel settings, streaming, retries, errors and SDKs.
appliesTo: [platform, cloud]
---

The **Agent API** exposes one Agent to your own code. An Agent API channel (`channel_type: "api"`) gives the Agent its own **base URL**, and callers use an **agent key** (`evr_ak_…`) to start sessions, send messages, follow events, answer questions and approve tool calls on that Agent. A key reaches only this one channel's routes. It cannot read or change agents, harnesses, other sessions or anything else in the management API, so you can hand it to an application without handing over your account.

For a step-by-step walkthrough, see [Call your agent from code](/how-to/call-your-agent-from-code/). This page is the reference.

## Base URL

Every route on this page is relative to the agent base URL. The channel's page in the console shows it as the **agent URL**.

| Host | Agent base URL |
|---|---|
| Everruns Cloud | `https://app.everruns.com/api/v1/channels/{channel_id}` |
| Self-hosted Platform | `https://your-everruns-host/api/v1/channels/{channel_id}` |
| A [serve](/framework/serve/) app | `http://localhost:3000/v1/channels/{agent}` |

`{channel_id}` is the channel's own ID (`appchan_…`), not the Agent's. In a serve app it is the agent's name. The shapes are the same on both hosts, so one client works against either; see [The same API on serve](#the-same-api-on-serve).

## Set up a channel

**In the console:**

1. Open the Agent and select the **Integrations** tab.
2. Select **Add channel**, choose **Agent API**, adjust its settings and select **Create channel**.
3. Select **Publish**. A channel serves traffic only while it is live, its Agent is active and the Agent's exposures are not suspended. See [Channel lifecycle](/features/channels/#channel-lifecycle).
4. On the channel page, select **Create key** in the **Agent keys** card and copy the secret. It is shown once.

**With the management API**, authenticated as a member who can manage the Agent:

```bash
# Create the channel. Every setting in channel_config is optional.
curl -X POST "$EVERRUNS_API/v1/agents/$AGENT_ID/channels" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY" -H "Content-Type: application/json" \
  -d '{"channel_type": "api", "channel_config": {"visibility": "activity"}}'

# Publish it (needs the dangerous Agent permission, Owner by default).
curl -X POST "$EVERRUNS_API/v1/agents/$AGENT_ID/channels/$CHANNEL_ID/publish" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY"
```

**With the CLI:**

```bash
everruns agents channels create --agent-id "$AGENT_ID" --channel-type api --channel-config '{}'
everruns agents channels publish --agent-id "$AGENT_ID" --channel-id "$CHANNEL_ID"
everruns agents channels keys create --agent-id "$AGENT_ID" --channel-id "$CHANNEL_ID" --name "Support backend"
```

Change settings later with `PATCH /v1/agents/{agent_id}/channels/{channel_id}` and a new `channel_config`. Changing a live channel's configuration needs the same dangerous Agent permission as publishing.

## Agent keys

An agent key is the credential of one application. Keys belong to the organization, not to the member who created them, so a key keeps working when its creator leaves. Everruns stores only a hash and a display prefix; the secret appears once, in the response that creates or rotates it.

| Method | Path (management API) | What it does |
|---|---|---|
| GET | `/v1/agents/{agent_id}/channels/{channel_id}/keys` | List the channel's keys, newest first, without secrets |
| POST | `/v1/agents/{agent_id}/channels/{channel_id}/keys` | Create a key |
| POST | `/v1/agents/{agent_id}/channels/{channel_id}/keys/{key_id}/rotate` | Replace the secret, keeping the key's ID |
| POST | `/v1/agents/{agent_id}/channels/{channel_id}/keys/{key_id}/revoke` | Stop the key for good |

### Create

```bash
curl -X POST "$EVERRUNS_API/v1/agents/$AGENT_ID/channels/$CHANNEL_ID/keys" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY" -H "Content-Type: application/json" \
  -d '{"name": "Support backend", "permissions": ["end_user"]}'
```

| Field | Required | Meaning |
|---|---|---|
| `name` | Yes | Display name, 1 to 128 characters. Name the application that holds the key, so you know which one to revoke. |
| `expires_at` | No | RFC 3339 time when the key stops working. Must be in the future. Omit for no expiry. |
| `permissions` | No | Every key has `sessions`, the Agent API routes. Add `end_user` to let the key act for your application's users with the `End-User` header. |

The response is `201` with the key and its `secret`:

```json
{
  "id": "agentkey_01933b5a000070008000000000000001",
  "channel_id": "appchan_01933b5a000070008000000000000001",
  "name": "Support backend",
  "prefix": "evr_ak_1a2b3c4d...",
  "permissions": ["sessions", "end_user"],
  "created_at": "2026-10-13T09:00:00Z",
  "secret": "evr_ak_1a2b3c4d…"
}
```

Later reads return the same fields without `secret`, plus `expires_at`, `previous_valid_until`, `last_used_at` (updated at most once a minute) and `revoked_at` when they are set. A channel holds at most 50 live keys; revoke one before creating more.

### Rotate

`POST …/keys/{key_id}/rotate` with an optional `{"overlap_hours": 24}` returns a new `secret` for the same key ID. The replaced secret keeps working for the overlap, from 0 to 168 hours, 24 by default, and `previous_valid_until` says until when. Deploy the new secret within that window. Sessions the key started stay reachable, because they belong to the key's ID, not its secret.

### Revoke

`POST …/keys/{key_id}/revoke` stops the key immediately, including a previous secret still inside its rotation overlap. A revoked key never works again.

## Authentication

Every request to the Agent API sends `Authorization: Bearer <credential>`. The credential decides who the **caller** is, and a caller sees only its own sessions.

| Credential | Caller | How to get it |
|---|---|---|
| Agent key `evr_ak_…` | The application holding the key | [Agent keys](#agent-keys) |
| Agent key with `end_user`, plus `End-User: <id>` | One of your application's users | [End users](#end-users) |
| Runtime token | The end user it was minted for | [Runtime tokens for browsers](#runtime-tokens-for-browsers) |
| A token from your identity provider | The token's subject, as an end user | [Your identity provider](#your-identity-provider) |
| Personal access token `evr_pat_…` | A member of your organization, as an end user | [Organization members](#organization-members) |

Keys of a retired `api_endpoint` channel (`evr_app_…`) were imported when the channel converted and keep working the same way as agent keys.

### End users

A key acting as itself is your application: every request it sends shares one set of sessions. To keep your users apart, create the key with `"permissions": ["end_user"]` and send `End-User` with each request:

```bash
curl -X POST "$EVERRUNS_AGENT_URL/sessions" \
  -H "Authorization: Bearer $EVERRUNS_AGENT_KEY" \
  -H "End-User: customer-42"
```

The ID is yours: up to 256 characters, no control characters and no leading or trailing spaces. Everruns creates an end user for it on first sight, scoped to the key, so two applications with different keys never collide on the same ID. Sessions an end user starts belong to that end user: the key acting as itself, or for another user, gets `404` for them. The Agent's per-user connections (tools that act as the user) use that end user's own connections, never the key holder's.

`End-User` from a key without `end_user`, a malformed ID, or `End-User` sent with any credential other than an agent key answers `403`.

### Runtime tokens for browsers

Never ship an agent key in a browser or mobile app. Your backend exchanges it for a short-lived token for one user and hands the app that token:

```bash
curl -X POST "$EVERRUNS_AGENT_URL/runtime-auth" \
  -H "Authorization: Bearer $EVERRUNS_AGENT_KEY" \
  -H "End-User: customer-42"
```

```json
{
  "access_token": "eyJhbGciOi…",
  "token_type": "Bearer",
  "expires_in": 900,
  "virtual_user_id": "identity_01933b5a000070008000000000000001"
}
```

The token lasts 15 minutes, works only on this channel, and cannot be exchanged for another, so a leaked token cannot renew itself. `/runtime-auth` also accepts a token from one of the channel's `auth_methods`. A key without `End-User` gets `400`: a key acting as itself is an application, not a user.

The page calls the channel with `Authorization: Bearer <access_token>`. List the page's origin in [`cors_origins`](#channel-settings) so the browser allows the cross-origin calls. Callers with a runtime token always get the public error codes.

### Your identity provider

`auth_methods` lists up to 5 identity providers whose access tokens the channel accepts besides agent keys: `oidc` (JWTs checked against the issuer's discovery keys), `google_oidc` (Google Sign-In ID tokens) and `oauth2_introspection` (opaque tokens checked at an RFC 7662 endpoint). Each token's subject becomes its own end user. Everruns only validates these tokens; it never issues them.

```json
{
  "auth_methods": [
    {
      "mode": "oidc",
      "provider": {
        "type": "oidc",
        "issuer": "https://login.example.com"
      },
      "requirements": {
        "audiences": ["api://support-agent"],
        "scopes": ["agent:invoke"]
      }
    }
  ]
}
```

`requirements` can also restrict `subjects`, `groups`, email `domains` and exact `claims` values. A token that matches no method answers `401`.

### Organization members

Set `org_members` to `true` to let members of your organization call the Agent with their own personal access token. Each member runs as their own end user with their own connections, as in the console. Only a full-access token counts: a read-only token answers `403` with `insufficient_scope`, because running an Agent is a write. Console session tokens are not accepted.

## Channel settings

Set these in the channel's `channel_config`, or in the console form. Unknown fields are rejected.

| Setting | Default | Values and effect |
|---|---|---|
| `visibility` | `activity` | What event reads and the stream show. `messages`: user input, final assistant text, turn boundaries, and questions or approvals waiting on the caller. `activity`: adds tool start and finish with `tool_activity_text`, never tool names or arguments. `full`: the raw events, including tool names, arguments, results, reasoning and usage, for callers who own both ends. |
| `tool_activity_text` | `Working...` | The text a tool start shows under `activity`. |
| `errors` | `public` | `public`: a failed turn reports only one of the [public error codes](#errors-and-status-codes). `detailed`: the internal error and code, for key-authenticated developers. |
| `session_binding` | `per_user` | `per_user` or `session_per_invocation`. Either way each caller has its own sessions; sessions are never shared between callers. |
| `tool_approvals` | `operator` | Who answers tool calls the Agent's approval gate holds back. `operator`: someone with access to the Agent in Everruns decides, and the caller sees only that a call waits. `caller`: the caller sees the tool and its arguments and decides through [`tool-approvals`](#questions-and-tool-approvals). |
| `rate_limit_per_minute` | none | Requests per minute per caller and client IP. Unset or `0` turns the channel's own limit off; the global API limit still applies. |
| `daily_spend_limit_usd` | none | The most one caller (a key acting as itself, or one end user) may spend on this Agent per UTC day, up to 1,000,000. Past it, starting a session or sending a message answers `429` until the next UTC day; reads still work and a running turn finishes. |
| `cors_origins` | none | Up to 20 browser origins, such as `https://app.example.com`, whose pages may call this channel's routes. Write each exactly as a browser sends it: `https` (or `http` for `localhost`, `127.0.0.1` and `[::1]`), no path, no trailing slash, no default port. These origins reach only this channel's routes, never the management API. |
| `auth_methods` | none | Up to 5 identity providers. See [Your identity provider](#your-identity-provider). |
| `org_members` | `false` | Accept members' personal access tokens. See [Organization members](#organization-members). |

## Routes

All routes take `Authorization: Bearer <credential>` and answer JSON, except the stream.

| Method | Path | What it does |
|---|---|---|
| GET | `/` | The agent card |
| POST | `/sessions` | Start a session |
| GET | `/sessions` | The caller's sessions, most recently active first |
| GET | `/sessions/{session_id}` | One session, with what it waits on and its latest [work report](/features/work-reports/#in-the-agent-execution-api) |
| POST | `/sessions/{session_id}/messages` | Send a user message |
| GET | `/sessions/{session_id}/events` | Events, oldest first |
| GET | `/sessions/{session_id}/sse` | Follow events live over SSE |
| POST | `/sessions/{session_id}/question-answers` | Answer the Agent's questions |
| POST | `/sessions/{session_id}/tool-approvals` | Allow or reject held-back tool calls |
| POST | `/sessions/{session_id}/cancel` | Cancel the running turn |
| POST | `/runtime-auth` | Mint a runtime token for an end user |

A session that belongs to another caller or another channel answers `404`, the same as one that does not exist.

### Get the agent card

`GET /` describes the Agent and which credentials the base URL accepts:

```json
{
  "name": "Support agent",
  "description": "Answers questions about orders.",
  "streaming": true,
  "input": {
    "text": true,
    "images": false,
    "files": false
  },
  "auth": [
    {
      "type": "agent_key"
    },
    {
      "type": "runtime_token"
    },
    {
      "type": "oidc",
      "issuer": "https://login.example.com"
    }
  ],
  "links": {
    "sessions": "/api/v1/channels/appchan_01933b5a000070008000000000000001/sessions"
  }
}
```

`auth` always lists `agent_key` and `runtime_token`, then `personal_access_token` when `org_members` is on, then one `oidc` entry per OpenID Connect issuer and `oauth2` for introspection. `input` says the Agent takes text, images and files ([Send files](/how-to/call-your-agent-from-code/#send-files)).

### Start a session

`POST /sessions` with an optional body:

```json
{
  "title": "Order 42"
}
```

The title is at most 200 characters. An empty body is fine. `metadata` is not supported on Everruns and answers `400`. The session starts idle; send a message to start a turn. The response is `201` with a `Location` header and the session:

```json
{
  "id": "session_01933b5a000070008000000000000001",
  "title": "Order 42",
  "status": "idle",
  "created_at": "2026-10-13T09:00:00Z",
  "updated_at": "2026-10-13T09:00:00Z"
}
```

`status` is `started`, `active`, `idle`, `waitingforinput` or `failed`. After a turn ends, `last_turn_status` is `completed`, `failed` or `cancelled`.

### List sessions

`GET /sessions?limit=50&page_token=…` returns `{"data": [...], "next_page_token": "…"}`. `limit` is 1 to 200, 50 by default. Pass `next_page_token` back as `page_token` for the next page; it is absent on the last page. A list leaves out `pending_questions` and `pending_approvals`.

### Get a session

`GET /sessions/{session_id}` returns the session plus `pending_questions` and `pending_approvals`: what the session waits on from the caller. See [Questions and tool approvals](#questions-and-tool-approvals).

### Send a message

`POST /sessions/{session_id}/messages` sends a user message. It starts a turn when the session is idle, or steers the running turn.

```json
{
  "message": {
    "role": "user",
    "content": [
      {
        "type": "text",
        "text": "Where is order 42?"
      }
    ]
  }
}
```

The role must be `user` and the content one or more `text` parts. The response is `201` with the stored message, including its `id`, `session_id`, `sequence`, `role` and `content`.

### Read events

`GET /sessions/{session_id}/events?after_sequence=0&limit=100` returns `{"data": [...]}`: the session's events after `after_sequence` (exclusive), oldest first. `limit` is 1 to 500, 100 by default. To page, pass the last event's `sequence` as the next `after_sequence`. Each event uses the [event envelope](/event-reference/), filtered by the channel's [visibility](#what-callers-see).

### Cancel a turn

`POST /sessions/{session_id}/cancel` cancels the running turn and answers `{"status": "cancelled", "message": "Turn cancelled"}`. The stream then shows `turn.cancelled`.

## Streaming

`GET /sessions/{session_id}/sse` follows a session's events live as Server-Sent Events. It uses the same framing as the session API's stream (event data shortened here):

```text
event: connected
data: {"status":"connected"}

event: output.message.completed
id: event_01933b5a000070008000000000000009
data: {"id":"event_01933b5a000070008000000000000009","type":"output.message.completed","sequence":9,"data":{"message":{"role":"assistant","content":[{"type":"text","text":"Order 42 shipped yesterday."}]}}}

: heartbeat

event: disconnecting
data: {"reason":"connection_cycle","retry_ms":100}
```

- The SSE `event:` name is the event type, and `data:` is the event envelope after visibility.
- Durable events carry an `id:`. To resume after a drop, reconnect with `since_id` set to the last `id` you received.
- `after_sequence` replays events after that sequence first, then continues live. A client that holds no events yet passes `after_sequence=0` to replay the whole session; without either parameter the stream starts live and can miss a turn that ends quickly.
- A heartbeat comment arrives while nothing happens.
- The server cycles each connection every 5 minutes. It sends `disconnecting` first; reconnect right away with `since_id`.

Open streams are capped per organization and per session; past the cap the stream answers `429`. The SDKs' `stream_events` handles reconnection for you; see [Stream events](/how-to/stream-events/).

## What callers see

`visibility` filters both `GET …/events` and the stream, before anything is written.

| Event type | `messages` | `activity` | `full` |
|---|---|---|---|
| `input.message`, `output.message.completed` | Text parts only | Text parts only | Raw |
| `turn.started`, `turn.completed`, `turn.cancelled` | `turn_id` only | `turn_id` only | Raw |
| `turn.failed` | Per `errors` | Per `errors` | Raw, with public errors when `errors` is `public` |
| `tool.call_requested` | Only questions and approvals waiting on the caller | Same | Raw |
| `tool.started`, `tool.completed` | Hidden | `tool_call_id` with `activity` text, or `success` | Raw |
| Everything else (reasoning, usage, deltas) | Hidden | Hidden | Raw |

Under `messages` and `activity`, intermediate assistant commentary is dropped, an event keeps only `id`, `type`, `ts`, `session_id`, `sequence`, `context` and its filtered `data`, and event metadata and tags are removed.

## Questions and tool approvals

When the Agent asks a question with [`ask_user`](/capabilities/ask-user/), or its [tool approval](/capabilities/tool-approval/) gate holds a call back, the session waits and `GET /sessions/{session_id}` lists what it waits on. The stream carries the same as a `tool.call_requested` event at every visibility.

```json
{
  "id": "session_01933b5a000070008000000000000001",
  "status": "waitingforinput",
  "created_at": "2026-10-13T09:00:00Z",
  "updated_at": "2026-10-13T09:01:00Z",
  "pending_questions": [
    {
      "tool_call_id": "toolu_01933b5a00007000800000000000001",
      "questions": [
        {
          "kind": "choice",
          "id": "target",
          "header": "Environment",
          "question": "Which environment should I check?",
          "multi_select": false,
          "allow_other": true,
          "options": [
            {
              "label": "Staging",
              "description": "The staging cluster",
              "default": false
            },
            {
              "label": "Production",
              "description": "The live cluster",
              "default": false
            }
          ]
        }
      ],
      "expires_at": "2026-10-13T09:31:00Z"
    }
  ],
  "pending_approvals": [
    {
      "tool_call_id": "tool_approval_toolu_01933b5a00007000800000000000002",
      "answerable": true,
      "tool_name": "refund_order",
      "arguments": {
        "order_id": "42"
      },
      "risk": "destructive",
      "expires_at": "2026-10-13T09:31:00Z"
    }
  ]
}
```

**Answer questions** with `POST /sessions/{session_id}/question-answers`:

```json
{
  "tool_call_id": "toolu_01933b5a00007000800000000000001",
  "status": "answered",
  "answers": [
    {
      "id": "target",
      "selected": ["Staging"]
    }
  ]
}
```

`tool_call_id` is optional when only one question set waits. `status` is `answered` (the default) or `declined`. Each answer names the question `id`, the chosen option labels in `selected`, and free text in `other_text` for a `text` question or a choice that allows another answer. The response is `{"status", "answered_by", "session_status"}` and the turn resumes. A question that asks for a credential (`kind: "secret"`) cannot be answered over this API, only declined; a person completes it in Everruns. Answers that do not match what was asked answer `400`; a session that is not waiting, or a set already answered, answers `409`.

**Approve tool calls** with `POST /sessions/{session_id}/tool-approvals`, on a channel whose `tool_approvals` is `caller`:

```json
{
  "decisions": [
    {
      "tool_call_id": "tool_approval_toolu_01933b5a00007000800000000000002",
      "decision": "allow"
    }
  ]
}
```

`decision` is `allow`, `allow_always` (this call and later calls of the same tool in this session), `reject` or `reject_always`. A pending request of the same batch that is left out is settled as not approved; silence never approves. The response lists how each request was settled in `resolved` (`tool_call_id`, `tool`, `outcome`) with the session `status`. When `tool_approvals` is `operator`, approvals show `answerable: false` without the tool name, arguments or risk, and this route answers `403`. A request that expired or was already answered answers `409`.

## Safe retries

`POST /sessions` and `POST /sessions/{session_id}/messages` accept an `Idempotency-Key` header of 1 to 255 printable ASCII characters. A retry with the same key and the same request within 24 hours returns the first response with `Idempotent-Replayed: true` instead of starting a second session or sending the message twice.

- Reusing a completed key for a different request answers `422` with `idempotency_key_reused`.
- A retry while the first request is still running answers `409` with `idempotency_key_in_progress`.
- Keys belong to the caller, so two applications or two end users never collide on one key.
- Only a success is remembered. A failed request frees its key, so you can fix the request and retry with the same key.

A keyed admission and its replay response commit together. If the server stops
before that commit, neither is accepted; retry the same key after reconnecting.
If the commit succeeded but its response was lost, the retry returns that response.
Pending claims written by older servers retain their original lease during an
upgrade, so they can continue returning `409` until that lease expires.

## Errors and status codes

Errors are [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457) problem details (`application/problem+json`) with `title`, `status` and `detail`, and a `code` where one applies.

| Status | When |
|---|---|
| `400` | Invalid body or query (bad `limit` or `page_token`, non-text content, `metadata`), a credential in a question answer, `/runtime-auth` with a key and no `End-User`, or a retired `api_endpoint` request shape (`api_endpoint_retired`) |
| `401` | Missing or invalid credential, including a token none of the channel's `auth_methods` accepts, or a personal access token when `org_members` is off |
| `403` | The channel is not live, its Agent is inactive or its exposures are suspended (`This agent is not available`); `End-User` not allowed; a read-only personal access token (`insufficient_scope`); or `tool-approvals` on an `operator` channel |
| `404` | No such `api` channel, or a session that is not the caller's |
| `409` | Nothing to answer, an answer already given, or an `Idempotency-Key` still in progress |
| `422` | An `Idempotency-Key` reused for a different request |
| `429` | The channel's rate limit, the caller's daily spending limit, or too many open streams |

The checks run in a fixed order: the channel resolves and is live, then the credential, then the rate limit, then the session. A bad key and a missing channel give the same generic answer, so a caller cannot probe which channels exist.

A turn that fails shows up as a `turn.failed` event rather than an HTTP error. With `errors: "public"`, which callers with a runtime token always get, its `data` carries only `turn_id`, a public `error` message and one of four `error_code` values: `rate_limited`, `service_unavailable`, `request_too_large` or `internal_error`. With `detailed`, it carries the internal `error` and `error_code`.

## SDKs

The Python, TypeScript and Rust SDKs have an `AgentClient` for the Agent API. Use `everruns-sdk` 0.3.1 or later. It reads `EVERRUNS_AGENT_URL` and `EVERRUNS_AGENT_KEY` from the environment, or takes them as arguments, and sends the credential as a Bearer token.

```python
from everruns_sdk import AgentClient

async with AgentClient() as agent:
    # One call: start a session, send, follow the stream, return the reply.
    print(await agent.run("Where is order 42?"))

    # Or step by step, with a safe retry key.
    session = await agent.create_session("Order 42", idempotency_key="order-42-start")
    await agent.send_message(session["id"], "Where is order 42?", idempotency_key="order-42-msg-1")
    async for event in agent.stream_events(session["id"], after_sequence=0):
        if event.type == "turn.completed":
            break

    # Act for one of your users, and mint a token for their browser.
    alice = agent.for_end_user("customer-42")
    await alice.run("Where is my order?")
    token = await alice.runtime_token()
```

```typescript
import { AgentClient } from "@everruns/sdk";

const agent = new AgentClient(); // EVERRUNS_AGENT_URL + EVERRUNS_AGENT_KEY
console.log(await agent.run("Where is order 42?"));

const session = await agent.createSession({ title: "Order 42", idempotencyKey: "order-42-start" });
await agent.sendMessage(session.id, "Where is order 42?", { idempotencyKey: "order-42-msg-1" });
for await (const event of agent.streamEvents(session.id, { afterSequence: 0 })) {
  if (event.type === "turn.completed") break;
}

const alice = agent.forEndUser("customer-42");
const token = await alice.runtimeToken();
```

```rust
use everruns_sdk::AgentClient;

let agent = AgentClient::from_env()?; // EVERRUNS_AGENT_URL + EVERRUNS_AGENT_KEY
println!("{}", agent.run("Where is order 42?", None).await?);

let session = agent.create_session(Some("Order 42"), Some("order-42-start")).await?;
agent.send_message(&session.id, "Where is order 42?", Some("order-42-msg-1")).await?;

let alice = agent.for_end_user("customer-42");
let token = alice.runtime_token().await?;
```

The client also has `card`, `list_sessions`, `get_session`, `list_events`, `cancel`, `answer_questions` and `submit_tool_approvals` (camelCase in TypeScript). `run` raises an error when the turn fails. Calling an Agent through the management client's `sessions.create` and `messages.create` still works but is deprecated. See [SDKs](/features/sdk/).

## The same API on serve

A [serve](/framework/serve/) app serves the same routes and shapes for each top-level agent at `/v1/channels/{agent}`, so a client written for one host works against the other: point `EVERRUNS_AGENT_URL` at either. A serve app is one operator's process rather than a multi-tenant platform, so some parts differ:

- **Credentials.** With no method configured the API is open. Static keys come from `SERVE_API_KEYS` (comma separated) or `[auth]` in `serve.toml`, and `oidc`, `google_oidc` and `oauth2_introspection` methods use the same verifier and requirements as a channel's `auth_methods`. Keys are not managed through an API, and there is no `End-User`, `/runtime-auth` or personal access token.
- **Callers are not separated.** Every authenticated caller reaches every session of the agent, as an operator would. A session is still reachable only under its own agent's base URL.
- **Public card.** `GET /v1/channels/{agent}` needs no credential; its `auth` list names the accepted methods, and its `links` add the agent's AG-UI and A2A routes.
- **No channel settings.** Events are not filtered by visibility, the caller answers tool approvals, and there are no spending or rate limits, browser origins or `Idempotency-Key` handling.
- **Messages.** `content` takes text parts only; send images and other files in `files`, which serve keeps in the session's `/attachments` folder as the server does.
- **Sessions.** `POST …/sessions` accepts `metadata`, and `GET …/sessions` returns up to `limit` sessions with no `page_token`.

See [Serve](/framework/serve/#one-agents-api) and the [serve tutorial](/framework/serve-tutorial/), which calls an agent with `AgentClient`.

## Migrating from `api_endpoint`

The former `api_endpoint` channel type converted to Agent API channels, keeping keys and sessions. Requests in the old shape answer `400` with `api_endpoint_retired`: starting a session no longer takes a message, and a message is an object, not a string. `/v1/apps/{app_id}/api/{channel_id}/…` routes answer `410`. Create the session, then send `{"message": {"role": "user", "content": [...]}}` to `…/messages`.

## See also

- [Call your agent from code](/how-to/call-your-agent-from-code/): the step-by-step guide.
- [Channels](/features/channels/): every channel type and the shared lifecycle.
- [Stream events](/how-to/stream-events/): reconnection patterns.
- [Handle errors and cancel turns](/how-to/handle-errors-and-cancellation/).
- [Event Reference](/event-reference/): every event type.
- [API reference](/api/): the generated reference for every route.

---
title: A2A (Agent2Agent)
description: Let other agents call an Everruns agent over the A2A 1.0 protocol, and let an Everruns agent delegate work to external A2A agents.
sidebar:
  label: A2A
---

[A2A](https://a2a-protocol.org) is an open protocol for agents to call each
other over HTTP. Everruns supports it in both directions:

- **Inbound**: an A2A channel makes an Everruns agent callable by any A2A
  client, such as another vendor's agent, the official `a2a` CLI, or the
  A2A Inspector.
- **Outbound**: the `a2a_agent_delegation` capability lets an Everruns agent
  hand work to external A2A agents through `spawn_agent`.

Both speak A2A 1.0. The inbound channel also answers A2A 0.3 clients on the
same URL.

Framework apps can serve their agents over A2A too. See
[A2A in the Framework](#a2a-in-the-framework).

## Expose an agent over A2A

An A2A channel belongs to an agent. Create it through the API with a key you
generate. Everruns stores only the key's SHA-256 hash, so keep the plaintext
somewhere safe.

```bash
EVERRUNS=https://your-everruns-host/api
KEY="evra2a_$(openssl rand -hex 32)"
HASH=$(printf %s "$KEY" | sha256sum | cut -d' ' -f1)

curl -sS -X POST "$EVERRUNS/v1/agents/$AGENT_ID/channels" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY" \
  -H 'Content-Type: application/json' \
  -d "{
    \"channel_type\": \"a2a\",
    \"channel_config\": {
      \"session_mode\": \"session_per_invocation\",
      \"message\": \"{{a2a.text}}\",
      \"agent_card_name\": \"Research agent\",
      \"agent_card_description\": \"Researches a topic and returns notes.\",
      \"api_key_hash\": \"$HASH\",
      \"api_key_prefix\": \"${KEY:0:15}...\"
    }
  }"
```

The response carries the channel `id`. Publish it so it accepts traffic:

```bash
curl -sS -X POST "$EVERRUNS/v1/agents/$AGENT_ID/channels/$CHANNEL_ID/publish" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY"
```

The channel then serves:

| Path | What it is |
| --- | --- |
| `POST /v1/channels/{channel_id}/a2a` | The A2A JSON-RPC endpoint. Needs `Authorization: Bearer <key>`. |
| `/v1/channels/{channel_id}/a2a/message:send`, `/tasks`, ... | The same operations over A2A's plain HTTP binding (HTTP+JSON). Same key. |
| `GET /v1/channels/{channel_id}/a2a/.well-known/agent-card.json` | The public Agent Card, served only while the channel is live. |

When the agent has an avatar, the Agent Card carries it as `iconUrl`: the 256 px square preset, on the same origin as the card.

### Configuration

| Field | Description |
| --- | --- |
| `session_mode` | `session_per_invocation` starts a fresh session for each new task. `shared_session` sends every call into one long-lived session. |
| `message` | Template for the user message the agent receives. `{{a2a.text}}` is the caller's text parts joined by newlines; `a2a.task_id`, `a2a.context_id`, and `payload` (the raw request params) are also available. |
| `agent_card_name`, `agent_card_description` | Shown in the Agent Card. They default to the agent's name and description. |
| `rate_limit_per_minute` | Optional per-caller-IP limit for this channel. Over the limit, calls get HTTP 429. |
| `signing_secret` | Optional. When set, every request must also carry an HMAC signature of its body. |

Instead of a generated key, a channel can use the shared channel auth
modes: HTTP Basic, OIDC or Google JWT bearer tokens, OAuth2 token
introspection, or mTLS. The Agent Card advertises whichever scheme is in use
and never includes a credential.

### Call it

Any A2A 1.0 client works. With the official
[`a2a` CLI](https://github.com/a2aproject/a2a-go):

```bash
CARD="$EVERRUNS/v1/channels/$CHANNEL_ID/a2a/.well-known/agent-card.json"
a2a card get -a "$CARD"
a2a send -a "$CARD" --auth "Bearer $KEY" "Summarize the A2A spec in three bullets."
a2a send -a "$CARD" --auth "Bearer $KEY" --stream "Explain A2A tasks in one sentence."
```

Or with plain JSON-RPC:

```bash
curl -sS -X POST "$EVERRUNS/v1/channels/$CHANNEL_ID/a2a" \
  -H "Authorization: Bearer $KEY" \
  -H 'Content-Type: application/json' \
  -H 'A2A-Version: 1.0' \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "SendMessage",
    "params": {
      "message": {
        "role": "ROLE_USER",
        "messageId": "msg-1",
        "parts": [{ "text": "Summarize the A2A spec in three bullets." }]
      }
    }
  }'
```

`SendMessage` waits for the turn to finish and returns the task with the
agent's reply as a `response` artifact. Set
`"configuration": { "returnImmediately": true }` to get the task back at once
and follow it with `GetTask`, `SubscribeToTask`, or a push notification.

The `A2A-Version` header picks the wire format: `1.0`, or `0.3` for older
clients. Without it, 1.0 method names such as `SendMessage` are answered in
1.0 and 0.3 names such as `message/send` in 0.3.

Clients that speak A2A's plain HTTP binding (HTTP+JSON) use paths below the
same URL instead of a JSON-RPC envelope. The Agent Card lists both bindings.
This binding is A2A 1.0 only:

```bash
curl -sS -X POST "$EVERRUNS/v1/channels/$CHANNEL_ID/a2a/message:send" \
  -H "Authorization: Bearer $KEY" \
  -H 'Content-Type: application/a2a+json' \
  -d '{
    "message": {
      "role": "ROLE_USER",
      "messageId": "msg-1",
      "parts": [{ "text": "Summarize the A2A spec in three bullets." }]
    }
  }'
```

| HTTP+JSON | Operation |
| --- | --- |
| `POST message:send`, `POST message:stream` | `SendMessage`, `SendStreamingMessage` |
| `GET tasks/{id}`, `GET tasks?…`, `POST tasks/{id}:cancel`, `POST tasks/{id}:subscribe` | `GetTask`, `ListTasks`, `CancelTask`, `SubscribeToTask` |
| `POST`/`GET tasks/{id}/pushNotificationConfigs`, `GET`/`DELETE tasks/{id}/pushNotificationConfigs/{configId}` | The push notification config operations |

Errors come back with the matching HTTP status and a `google.rpc.Status` body
whose `details[0].reason` names the A2A error, for example `TASK_NOT_FOUND`.

### Supported methods

| A2A 1.0 | A2A 0.3 | Notes |
| --- | --- | --- |
| `SendMessage` | `message/send` | A `taskId` or `contextId` from an earlier reply continues that conversation. |
| `SendStreamingMessage` | `message/stream` | Server-sent events: the task, then the reply as an artifact, then the final status. Needs `session_mode: session_per_invocation`; a `shared_session` channel answers `-32004`. |
| `GetTask` | `tasks/get` | Current state and the latest turn's reply. |
| `ListTasks` | `tasks/list` | This channel's tasks, newest first, filtered by `contextId`, `status`, or `statusTimestampAfter`, with `pageSize` and `pageToken` paging. |
| `SubscribeToTask` | `tasks/resubscribe` | Reattach a stream to a running task. A finished task answers `-32004`. |
| `CancelTask` | `tasks/cancel` | Cancels the running turn. A finished task answers `-32002`. |
| `CreateTaskPushNotificationConfig`, `Get…`, `List…`, `Delete…` | `tasks/pushNotificationConfig/set`, `get`, `list`, `delete` | Webhooks for a task. See below. |

The extended Agent Card is not offered (`-32007`).

A task's id is the id of the Everruns session that runs it, and its
`contextId` is the same value. A caller can only see and continue tasks
started through its own channel.

### Push notifications

Register a webhook on a task, or pass one in `SendMessage` as
`configuration.taskPushNotificationConfig`:

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "CreateTaskPushNotificationConfig",
  "params": {
    "taskId": "<task id>",
    "id": "my-hook",
    "url": "https://hooks.example.com/a2a",
    "token": "opaque-value-you-check",
    "authentication": { "scheme": "Bearer", "credentials": "hook-secret" }
  }
}
```

When the task completes, fails, is canceled, or stops to ask a question,
Everruns POSTs the full task (`{ "task": … }`, `Content-Type:
application/a2a+json`) to each registered URL. The `token` arrives as
`X-A2A-Notification-Token` and `authentication` as
`Authorization: {scheme} {credentials}`. Both are stored encrypted and are
never returned by `Get` or `List`.

A task holds at most ten configs. URLs must be public `https`; private,
loopback, and cloud metadata addresses are rejected. A failed delivery is
retried twice and dropped on a 4xx other than 429, so treat `GetTask` as the
source of truth.

### Questions and approvals

When the agent asks a question with `ask_user`, the task goes to
`input-required`. The status message carries the question as text and as an
`everruns/ask_user` data part with a JSON Schema for the answer. Answer it
with `SendMessage` on the same `taskId`. A question that asks for a secret
reports `auth-required` instead and links to the session, so a person
provides the value in Everruns rather than through another agent.

### Personal agents (PACT)

[PACT](https://github.com/openpactprotocol/openpactprotocol) (Personal Agent
Consent and Trust) is how a person's own agent, such as an assistant on their
phone, talks to a company's agent on their behalf. Everruns implements the
PACT Identity profile. Add `pact` to an A2A channel's config to serve it:

```json
{
  "pact": {
    "audience": "everruns-acme",
    "personal_agents": [
      { "issuer": "https://pa.example", "jwks_uri": "https://pa.example/.well-known/jwks.json" }
    ]
  }
}
```

| Field | Description |
| --- | --- |
| `audience` | The value personal agents put in the token's `aud`. Use the same value on every channel. |
| `personal_agents[].issuer` | The personal agent's `iss`, matched exactly. Any other issuer is refused. |
| `personal_agents[].jwks_uri` or `jwks` | Where its public keys are: an HTTPS URL, or the JWKS document inline. |
| `personal_agents[].enabled` | `false` refuses that personal agent. Defaults to `true`. |

The channel is then also served at `/v1/a2a/{channel_id}`, with its PACT Agent
Card at `/v1/a2a/{channel_id}/.well-known/agent-card.json`. Each request
carries a short-lived ES256 or RS256 JWT that the personal agent signs for one
of its users (`sub`). `message:send` answers with the agent's reply as a
Message. Its `contextId` continues the conversation for that same user only,
and a retried `messageId` returns the stored reply instead of running the
agent again. There are no tasks, streaming or push notifications on this URL.
The channel's ordinary A2A URL keeps working with its own key.

## Delegate to external A2A agents

The `a2a_agent_delegation` capability gives an agent a `spawn_agent` target of
type `external_a2a`. The agents it may call are fixed in the capability's
configuration, so the model can never reach a URL you did not list:

```json
{
  "agents": [
    {
      "id": "research",
      "name": "Research agent",
      "description": "Researches a topic and returns notes.",
      "base_url": "https://agents.example.com/research"
    }
  ]
}
```

`base_url` is where the Agent Card is resolved
(`{base_url}/.well-known/agent-card.json`). The model then calls:

```json
{
  "instructions": "Research tide pools and return five bullet notes.",
  "target": { "type": "external_a2a", "id": "research" },
  "mode": "foreground"
}
```

`foreground` waits for the remote task and returns its reply. `background`
returns a `task_id` at once and wakes the session when the remote task
finishes; the generic `wait_task`, `message_task`, and `cancel_task` tools
work on it. Pass `result_schema` to require a structured result.

URLs are checked before every call: localhost, private ranges, and metadata
addresses are refused, resolved addresses are pinned, and redirects are not
followed. `allow_local_urls: true` lifts the address check only when
`DEPLOYMENT_GRADE=dev`.

> **Status:** On the hosted platform, outbound delegation is experimental and
> available in Dev environments.

## A2A in the Framework

In a [Framework](/framework/) app (full guide: [Framework A2A](/framework/a2a/)):

- To **serve** agents over A2A, turn on the `a2a` feature of `everruns-serve`.
  Every top-level agent then answers at `POST /v1/channels/{agent}/a2a`, the same
  path shape as an Everruns channel. See [Serve](/framework/serve/#a2a).
- To **delegate** to A2A agents, turn on the `a2a` feature of `everruns` and
  add `CapabilityRef::new("a2a_agent_delegation")` with the config above.

The [A2A example](https://github.com/everruns/everruns/tree/main/examples/serve/a2a)
example does both. A `researcher` agent is served over A2A, and a `writer`
agent delegates research to it and drafts from its notes. It runs offline by
default and on OpenAI when `OPENAI_API_KEY` is set:

```bash
cargo run -p serve-example-a2a --bin researcher   # serves on :3000
cargo run -p serve-example-a2a --bin writer -- "tide pools"
```

## See also

- [Sub-agents](/capabilities/sub-agents/), the shared `spawn_agent` tool.
- [ARD Discovery](/integrations/ard/), for finding A2A agents at runtime.
- [Channels](/features/channels/), for channel lifecycle and legacy routes.

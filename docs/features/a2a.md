---
title: A2A (Agent2Agent)
description: Let other agents call an Everruns agent over the A2A 1.0 protocol, and let an Everruns agent delegate work to external A2A agents.
sidebar:
  label: A2A
---

[A2A](https://a2a-protocol.org) is an open protocol for agents to call each
other over HTTP. Everruns supports it in both directions:

- **Inbound**: an A2A endpoint makes an Everruns agent callable by any A2A
  client, such as another vendor's agent, the official `a2a` CLI, or the
  A2A Inspector.
- **Outbound**: the `a2a_agent_delegation` capability lets an Everruns agent
  hand work to external A2A agents through `spawn_agent`.

Both speak A2A 1.0. The inbound endpoint also answers A2A 0.3 clients on the
same URL.

Framework apps can serve their agents over A2A too. See
[A2A in the Framework](#a2a-in-the-framework).

## Expose an agent over A2A

An A2A endpoint belongs to an agent. Create it through the API with a key you
generate. Everruns stores only the key's SHA-256 hash, so keep the plaintext
somewhere safe.

```bash
EVERRUNS=https://your-everruns-host/api
KEY="evra2a_$(openssl rand -hex 32)"
HASH=$(printf %s "$KEY" | sha256sum | cut -d' ' -f1)

curl -sS -X POST "$EVERRUNS/v1/agents/$AGENT_ID/endpoints" \
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

The response carries the endpoint `id`. Publish it so it accepts traffic:

```bash
curl -sS -X POST "$EVERRUNS/v1/agents/$AGENT_ID/endpoints/$ENDPOINT_ID/publish" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY"
```

The endpoint then serves:

| Path | What it is |
| --- | --- |
| `POST /v1/e/{endpoint_id}/a2a` | The A2A JSON-RPC endpoint. Needs `Authorization: Bearer <key>`. |
| `GET /v1/e/{endpoint_id}/a2a/.well-known/agent-card.json` | The public Agent Card, served only while the endpoint is live. |

### Configuration

| Field | Description |
| --- | --- |
| `session_mode` | `session_per_invocation` starts a fresh session for each new task. `shared_session` sends every call into one long-lived session. |
| `message` | Template for the user message the agent receives. `{{a2a.text}}` is the caller's text parts joined by newlines; `a2a.task_id`, `a2a.context_id`, and `payload` (the raw request params) are also available. |
| `agent_card_name`, `agent_card_description` | Shown in the Agent Card. They default to the agent's name and description. |
| `rate_limit_per_minute` | Optional per-caller-IP limit for this endpoint. Over the limit, calls get HTTP 429. |
| `signing_secret` | Optional. When set, every request must also carry an HMAC signature of its body. |

Instead of a generated key, an endpoint can use the shared endpoint auth
modes: HTTP Basic, OIDC or Google JWT bearer tokens, OAuth2 token
introspection, or mTLS. The Agent Card advertises whichever scheme is in use
and never includes a credential.

### Call it

Any A2A 1.0 client works. With the official
[`a2a` CLI](https://github.com/a2aproject/a2a-go):

```bash
CARD="$EVERRUNS/v1/e/$ENDPOINT_ID/a2a/.well-known/agent-card.json"
a2a card get -a "$CARD"
a2a send -a "$CARD" --auth "Bearer $KEY" "Summarize the A2A spec in three bullets."
a2a send -a "$CARD" --auth "Bearer $KEY" --stream "Explain A2A tasks in one sentence."
```

Or with plain JSON-RPC:

```bash
curl -sS -X POST "$EVERRUNS/v1/e/$ENDPOINT_ID/a2a" \
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

### Supported methods

| A2A 1.0 | A2A 0.3 | Notes |
| --- | --- | --- |
| `SendMessage` | `message/send` | A `taskId` or `contextId` from an earlier reply continues that conversation. |
| `SendStreamingMessage` | `message/stream` | Server-sent events: the task, then the reply as an artifact, then the final status. Needs `session_mode: session_per_invocation`; a `shared_session` endpoint answers `-32004`. |
| `GetTask` | `tasks/get` | Current state and the latest turn's reply. |
| `ListTasks` | `tasks/list` | This endpoint's tasks, newest first, filtered by `contextId`, `status`, or `statusTimestampAfter`, with `pageSize` and `pageToken` paging. |
| `SubscribeToTask` | `tasks/resubscribe` | Reattach a stream to a running task. A finished task answers `-32004`. |
| `CancelTask` | `tasks/cancel` | Cancels the running turn. A finished task answers `-32002`. |
| `CreateTaskPushNotificationConfig`, `Get…`, `List…`, `Delete…` | `tasks/pushNotificationConfig/set`, `get`, `list`, `delete` | Webhooks for a task. See below. |

The extended Agent Card is not offered (`-32007`).

A task's id is the id of the Everruns session that runs it, and its
`contextId` is the same value. A caller can only see and continue tasks
started through its own endpoint.

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
addresses are refused. `allow_local_urls: true` lifts that for local
development only.

> **Status:** On the hosted platform, outbound delegation is experimental and
> available in Dev environments.

## A2A in the Framework

In a [Framework](/framework/) app (full guide: [Framework A2A](/framework/a2a/)):

- To **serve** agents over A2A, turn on the `a2a` feature of `everruns-serve`.
  Every top-level agent then answers at `POST /v1/e/{agent}/a2a`, the same
  path shape as an Everruns endpoint. See [Serve](/framework/serve/#a2a).
- To **delegate** to A2A agents, turn on the `a2a` feature of `everruns` and
  add `CapabilityRef::new("a2a_agent_delegation")` with the config above.

The [`examples/serve/a2a`](https://github.com/everruns/everruns/tree/main/examples/serve/a2a)
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
- [Apps Compatibility](/features/apps/), for endpoint lifecycle and legacy routes.

---
title: agentd API
description: Every route agentd serves when it runs on its own, with what it takes and what it answers.
---

The API of an install started with `agentd serve` (or a gateway with
`--install`). [Run agents with agentd](/getting-started/agentd/) walks
through it. Every route except `/health` needs a credential: an API key or
token as `Authorization: Bearer <...>`, or a client certificate. Errors
answer with a status and a plain-text reason.

Names (agent ids, secret names, group values, label names) are letters,
digits, `-`, `_` and `.`, at most 128 characters.

## Agents

| Route | Does |
| --- | --- |
| `PUT /v1/agents/{agent}` | Push an [agent definition](/reference/agent-package/). Answers `{"id", "revision", "changed"}`: the same definition again keeps its revision (`changed: false`). `400` when agentd cannot build it. |
| `GET /v1/agents` | `{"data": [{"id", "revision", ...}]}` |
| `GET /v1/agents/{agent}` | The agent at its current revision, with its definition. |
| `DELETE /v1/agents/{agent}` | Delete it. Running sessions keep their revision. |

## Secrets

A secret body has one source and, for a model key, a binding:

```json
{
  "value": "sk-...",
  "bind": { "model": "openai", "provider_type": "openai", "base_url": "https://...", "api": "chat_completions" }
}
```

- Source: `value` (stored, sealed), `env` (a variable the executor reads;
  needs `--env-secrets`) or `resolve: {"ref", "url"}` (your resolver;
  `url` defaults to `AGENTD_RESOLVER_URL`).
- `bind.model` is the provider name agents use in `model.provider`;
  `provider_type` its driver when the name is not one (`openai`,
  `anthropic`, `openrouter`, `gemini`, `mistral`, ...); `base_url` and `api`
  for OpenAI-compatible endpoints.

| Route | Scope |
| --- | --- |
| `PUT`, `DELETE /v1/secrets/{name}` | The install |
| `PUT`, `DELETE /v1/groups/{group}/secrets/{name}` | One group |
| `PUT`, `DELETE /v1/agents/{agent}/secrets/{name}` | One agent's sessions |
| `PUT`, `DELETE /v1/sessions/{session}/secrets/{name}` | One session |
| `GET /v1/secrets` | Names and revisions of the install's secrets; values never leave. |

`PUT` answers `{"name", "scope", "revision"}`. A turn uses the narrowest key
bound to its provider: session, agent, group, install.

## Groups

The install's group label (`--group-label`) names each session's group.

| Route | Does |
| --- | --- |
| `PUT /v1/groups/{group}` | Set `{"limit": <turns at once>, "suspended": <bool>, "spend_usd": <dollars>}`. Leave a field out to clear it. |
| `GET /v1/groups/{group}` | `{"group", "limit", "suspended", "spend_usd", "spent_usd", "reserved_usd"}` |

## Sessions

| Route | Does |
| --- | --- |
| `POST /v1/sessions` | Create: `{"agent_id": "...", "labels": {...}}`, or `{"agent": <definition>, "labels": {...}}` for a one-off agent. The group label is required when the install has one. Answers the session, with its `id`. |
| `GET /v1/sessions?label=k=v&limit=n` | Sessions whose labels hold every `label` given, newest first (`limit` up to 1000, default 100). |
| `GET /v1/sessions/{session}` | The session. |
| `POST /v1/sessions/{session}/messages` | Send `{"text": "..."}`. `?wait=true` answers when the turn ends, with its status and output. |
| `GET /v1/sessions/{session}/messages` | Its messages. |
| `GET /v1/sessions/{session}/events` | Its events (`?since_id=` to read on). |
| `GET /v1/sessions/{session}/sse` | Its events as server-sent events. |
| `POST /v1/sessions/{session}/cancel` | Cancel the running turn. |
| `POST /v1/sessions/{session}/tool-approvals` | Answer tool calls waiting for approval. |
| `POST /v1/sessions/{session}/tool-results` | Give results of client-side tool calls. |

Labels are set at creation and never change. agentd sets `agentd.agent` (the
pushed agent) itself and refuses other `agentd.` labels.

## Stream

`GET /v1/stream` answers `{"data": [records], "cursor": "..."}`.

| Parameter | Meaning |
| --- | --- |
| `after` | A cursor from an earlier page: records after it, each exactly once. Without it, from the oldest kept. |
| `wait` | Seconds to hold the call open when nothing is new (up to 30). |
| `label` | `name=value`; repeat to require several. |
| `type` | A record kind (`Usage`, `Session`, `Event`, ...) or a session event's type (`turn.completed`); repeat for several. |
| `limit` | Records per page (default 500, at most 5000). |

With `Accept: text/event-stream` the records come as server-sent events, each
with its cursor as `id`; reconnecting with `Last-Event-ID` reads on from it.

Each record has `type`, `shard`, `incarnation`, `index` and `ordinal` (its
position), `group`, `at_unix_ms`, the session's `labels`, and the fields of
its kind. A cursor older than the 30 days the stream keeps answers `410`.

## Health

`GET /health` answers `ok` without credentials.

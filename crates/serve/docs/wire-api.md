# Wire API

> Experimental. See the [README](../README.md) for status.

`dev`, a self-hosted `start` and a hosted deploy all serve the same routes. The
handlers are in [`src/server.rs`](../src/server.rs).

| Route | Body | Answer |
|---|---|---|
| `POST /v1/sessions` | `{input?, metadata?}` | `201`, `Location: /v1/sessions/{id}`, `{id, agent, build_id}` |
| `POST /v1/agents/{name}/sessions` | same | same, on a named agent |
| `GET /v1/sessions/{id}` | | the session record |
| `POST /v1/sessions/{id}/messages` | `{input}` | `202`. Starts a turn when the session is idle, or steers the active one. |
| `GET /v1/sessions/{id}/events` | | SSE, resumable (see below) |
| `POST /v1/sessions/{id}/cancel` | | `{cancelled}` |
| `POST /v1/sessions/{id}/approvals/{approval_id}` | `{decision: "approve"\|"deny", note?}` | `{resolved}`, or `404` when nothing is pending |
| `GET /v1/agent` | | agent card: agents, tools, skills, channels, schedules, version |
| `POST /v1/channels/{name}` | the provider's webhook | whatever the channel answers |
| `GET /health` | | `{status, build_id}` |
| `POST /dev/schedules/{name}` | | runs a schedule now (`dev` only) |

Errors are JSON `{error}`, with `404` for an unknown session, agent, channel or
approval, `400` for a bad request, and `409` for a session pinned to another
build.

## The event stream

Every session has one ordered log. Each event looks like this:

```json
{"seq": 14, "session_id": "session_…", "type": "tool.started", "at": "…",
 "data": {"tool_name": "run_sql", "arguments": {"sql": "…"}, "turn_id": "turn_…"}}
```

On the SSE stream, `seq` is the event `id` and `type` is the event name.

**Resuming.** Reconnect with `Last-Event-ID: <seq>` (or `?after=<seq>`), and
the stream replays every event after that one and then follows the log live.
There is no continuation token. The cursor is a position in the log, so a
client that drops, or a server that restarts, loses nothing. `?follow=false`
replays the log and closes the stream.

Most event types come straight from the everruns runtime (`turn.started`,
`output.message.delta`, `tool.started`, `tool.completed`, `turn.completed`,
…). The host adds these:

| Type | When |
|---|---|
| `session.created`, `session.resumed` | a session was created, or reloaded after a restart |
| `message.accepted` | input was accepted: `disposition` is `started` or `steered` |
| `tool.progress` | a tool called `cx.progress(...)` |
| `approval.requested`, `approval.resolved` | a tool call is waiting for a person, or a person decided |
| `subagent.started`, `subagent.completed` | an `ask_<name>` delegation |
| `turn.result` | the last event of every turn: `{response, success, error, tool_calls}` |
| `delivery.completed`, `delivery.failed` | a reply was, or was not, posted to a channel |
| `session.build_changed` | `dev` resumed a session that started on an older build |
| `stream.lagged` | the host fell behind the runtime's live feed |

Order is guaranteed within the runtime's events and within the host's events.
Between the two, it is only approximate: a `tool.progress` can land just
before the `tool.started` of the same call. `turn.result` is always last.

## Approvals

```text
tool call ──► approval.requested {approval_id, tool, arguments}
                    │
POST /v1/sessions/{id}/approvals/{approval_id} {"decision":"approve"}
                    │
          approval.resolved ──► the tool runs (or the model is told it was declined)
```

A denied call returns to the model as a tool error, with the note attached.
Cancelling the turn drops its pending approvals. In this PoC, approvals do not
survive a restart.

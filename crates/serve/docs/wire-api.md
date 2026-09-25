# Wire API

> Experimental. See the [README](../README.md) for status.

serve's HTTP API is a subset of the everruns server's `/v1` session API, so
the official [everruns Rust SDK](https://docs.rs/everruns-sdk), the everruns
CLI and the UI chat view can drive a serve app. Where a route exists on the
server, the request and response shapes match it. serve adds a few optional
fields and serve-only routes, marked below. `dev`, a self-hosted `start` and a
hosted deploy all serve the same routes; the handlers are in
[`src/server.rs`](../src/server.rs).

```rust
let client = everruns_sdk::Everruns::builder()
    .api_key("unused")                       // serve has no auth yet
    .base_url("http://localhost:3000")
    .build()?;
let session = client
    .sessions()
    .create_with_options(CreateSessionRequest::new().agent_name("analyst"))
    .await?;
client.messages().create(&session.id, "What was revenue last week?").await?;
```

## Routes

| Route | Body | Answer |
|---|---|---|
| `POST /v1/sessions` | `{agent_name?, title?, tags?, hints?, metadata?}` (other fields ignored) | `201` `Session`, `Location: /v1/sessions/{id}`. `404` for an unknown agent. |
| `GET /v1/sessions/{id}` | | `Session` |
| `POST /v1/sessions/{id}/messages` | `{message: {role?: "user", content: [{type: "text", text}]}, metadata?, controls?}` | `201` `Message`. Starts a turn when idle, steers the running one otherwise. Non-text parts are `400`. |
| `POST /v1/sessions/{id}/cancel` | | `{status: "cancelled" \| "no_op", message}` |
| `GET /v1/sessions/{id}/sse` | query below | SSE stream of events |
| `GET /v1/sessions/{id}/events` | query below | `{data: Event[]}` |
| `POST /v1/sessions/{id}/question-answers` | `{tool_call_id?, status?: "answered" \| "declined", answers: [{id, selected?, other_text?}]}` | `{status, answered_by, session_status}` |
| `POST /v1/sessions/{id}/approvals/{tool_call_id}` (serve) | `{decision: "approve" \| "deny", note?}` | `{status, tool_call_id, session_status}`, `404` when nothing is pending |
| `GET /v1/agent` (serve) | | agent card: agents, tools, skills, channels, schedules, version |
| `POST /v1/channels/{name}` (serve) | the provider's webhook | whatever the channel answers |
| `GET /health` (serve) | | `{status, build_id}` |
| `POST /dev/schedules/{name}` (serve, `dev` only) | | runs a schedule now |

Errors are RFC 9457 problem details (`application/problem+json`,
`{title, status, detail}`) like the server's: `404` for an unknown session,
agent, channel, approval or question set, `400` for a bad request, `409` for
an answered question set or a session pinned to another build (with an
`x-serve-build` header naming it).

### Session

The server's `Session` shape: `id` (`session_…`), `organization_id` (a fixed
`org_000…`, since serve has no tenants), `harness_id` (derived from the app
name), `agent_id` (the serve agent name), `status`, `title?`, `tags`,
`hints?`, `created_at`, `updated_at`. `status` uses the server vocabulary:
`active` while a turn runs, `waitingfortoolresults` while an approval or a
question waits for a person, `idle` otherwise.

serve adds `build_id`, `agent_name`, `metadata?`, and the requests waiting on
a person:

```json
"pending_approvals": [{"tool_call_id": "call_…", "tool_name": "run_sql", "arguments": {"sql": "SELECT * FROM orders"}}],
"pending_questions": [{"tool_call_id": "call_…", "questions": [{"id": "question_1", "header": "Target", "question": "Where?", "options": […]}]}]
```

### Message

`{id, session_id, sequence?, role: "user", content, metadata?, created_at}`.
`sequence` is the sequence of the canonical `input.message` event; it is left
out if the runtime has not committed that event yet.

## Events

Events are the everruns engine's durable canonical log, the same envelope the
server sends:

```json
{"id": "event_…", "type": "tool.started", "ts": "…", "session_id": "session_…",
 "context": {"turn_id": "turn_…", …}, "data": {"tool_call": {"name": "run_sql", "arguments": {…}}, …},
 "sequence": 14}
```

serve writes no events of its own. Durable events have a dense per-session
`sequence` starting at 1, which survives restarts. Streaming deltas
(`output.message.delta`, …) are live-only and carry no `sequence`. As on the
server, opaque reasoning replay state is stripped.

### `GET /v1/sessions/{id}/sse`

| Query | Meaning |
|---|---|
| `after_sequence=N` | replay durable events with `sequence > N`, then follow live. `0` replays everything. |
| `since_id=event_…` | replay events whose id sorts after it (ids are time-ordered, so an ephemeral event's id works too), then follow live |
| `types=…`, `exclude=…` | repeatable type filters |

`since_id` with `after_sequence` is `400`. Without either, the stream starts
live, as on the server; a client with no events yet sends `after_sequence=0`.

The stream opens with `event: connected` / `data: {"status":"connected"}`.
Each event is `event: <type>`, `data: <envelope>`, a `retry:` hint, and
`id: <event id>` on durable events only, so a reconnecting client (the SDK
does this itself) resumes with `since_id` from the last id it saw and loses
nothing. A `:heartbeat` comment is sent every 30 s.

### `GET /v1/sessions/{id}/events`

Durable events, oldest first, as `{data: [...]}`. Query: `after_sequence`,
`before_sequence`, `since_id`, `types`, `exclude`, and `limit` (1 to 1000,
the last N; sets `X-Total-Count` to the session's durable event count).

## Approvals

A `#[tool(needs_approval …)]` becomes the runtime's per-tool gate
(`FunctionTool::needs_approval`), answered by serve's approver:

```text
gated call ──► session status "waitingfortoolresults", pending_approvals: [{tool_call_id, …}]
                    │
POST /v1/sessions/{id}/approvals/{tool_call_id} {"decision":"approve"}
                    │
          tool.started … tool.completed   (or, on deny, a tool error: "rejected by user")
```

The runtime tells the model only that the call was rejected; a `note` is not
passed on. Cancelling the turn drops its pending approvals. In this PoC they
do not survive a restart. There is no canonical event for a pending approval:
clients find it on the session (and `dev` prints it with a ready `curl`).

## Questions (`ask_user`)

Every serve agent has the built-in `ask_user` tool. When the model calls it,
the question set appears in `pending_questions`, and
`POST /v1/sessions/{id}/question-answers` answers it exactly as on the
server: every question answered by its `id`, only offered option labels, one
selection for a single-select question. `status: "declined"` is a finished
refusal the model must not re-ask. Without `tool_call_id`, the one pending set
is answered. Nothing pending is `404`; answering twice is `409`. serve stores
no secrets, so `secret` questions cannot be answered.

## Differences from the server

- No auth, organizations, agents CRUD, harnesses, workspaces or files routes.
- `agent_name` names a serve agent from `#[agent]`; `agent_id` is that name,
  not an `agent_…` id. The SDK validates `agent_name` as kebab-case, so an
  agent reachable through the SDK needs a name like `analyst`, not `run_sql`.
- No `tool-results` route (serve has no client-side tools); approvals use the
  serve-only route above.
- `events` supports the listed filters only (no `around`, `q`, `turn_id` …).

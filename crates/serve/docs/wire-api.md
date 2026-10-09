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
| `POST /v1/channels/{agent}/ag-ui` (`ag-ui` feature) | AG-UI 1.0 `RunAgentInput` | SSE stream of AG-UI events, see [AG-UI](#ag-ui) |
| `POST /v1/channels/{agent}/a2a` (`a2a` feature) | A2A 1.0 JSON-RPC (`SendMessage`, `SendStreamingMessage`, `GetTask`, `ListTasks`, `CancelTask`, `SubscribeToTask`) | JSON-RPC result, or SSE for the streaming methods, see [A2A](#a2a) |
| `GET /v1/channels/{agent}/a2a/.well-known/agent-card.json` (`a2a` feature) | | A2A 1.0 Agent Card |
| `GET /v1/channels/{agent}/voice` (`voice` feature) | | HTML test page that places calls from the browser |
| `POST /v1/channels/{agent}/voice/calls` (`voice` feature) | `{sdp, session_id?}` | `{session_id, call_id, model, voice, answer_sdp}`, see [Voice](#voice) |
| `POST /v1/channels/{agent}/voice/calls/{call_id}/end` (`voice` feature) | | `{call_id, utterances, spoken_chunks, interruptions}` |
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
passed on. Cancelling the turn drops its pending approvals. After a restart,
the first request that touches the session (reading it included) finds the
turn the old process left waiting and runs the waiting call again, so the
approval is pending once more under the same `tool_call_id`; pending
questions come back the same way. There is no canonical event for a pending
approval: clients find it on the session (and `dev` prints it with a ready
`curl`).

## Questions (`ask_user`)

Every serve agent has the built-in `ask_user` tool. When the model calls it,
the question set appears in `pending_questions`, and
`POST /v1/sessions/{id}/question-answers` answers it exactly as on the
server: every question answered by its `id`, only offered option labels, one
selection for a single-select question. `status: "declined"` is a finished
refusal the model must not re-ask. Without `tool_call_id`, the one pending set
is answered. Nothing pending is `404`; answering twice is `409`. serve stores
no secrets, so `secret` questions cannot be answered.

## AG-UI

With the `ag-ui` cargo feature, every top-level agent also answers AG-UI 1.0
clients (CopilotKit, `@ag-ui/client`) at `POST /v1/channels/{agent}/ag-ui`, the route
shape of the server's endpoint channel, so a front end moves between the two by
base URL and id alone. The agent card lists the routes under `ag_ui`, and the
manifest under `routes`.

- **Threads.** Each AG-UI `threadId` maps to one session (created on its
  first run, kept in the same thread map channels use), so it survives a
  restart. A run sends only the last user message; the session owns the
  conversation.
- **Stream.** Unnamed `data:` SSE events, `RUN_STARTED` first and exactly one
  `RUN_FINISHED` or `RUN_ERROR` last, with a `keepalive` comment every 15
  seconds. The projection is the server's with the trusted policy: reasoning,
  token usage and runtime errors are visible.
- **Interrupts.** A pending approval (`tool_approval`) or `ask_user` question
  (`everruns.ask_user`) ends the run with the interrupt outcome; the next run's
  `resume` entries answer it and stream the rest of the turn. They are the same
  pending requests the routes above answer, so either API can resolve them.
- **Errors.** A body that is not a `RunAgentInput`, an empty `threadId`, or
  input the run cannot use (no trailing user message, an entry that cannot be
  applied) is `400`; an unknown agent or a subagent is `404`.

## A2A

With the `a2a` cargo feature, every top-level agent also answers A2A 1.0
JSON-RPC at `POST /v1/channels/{agent}/a2a`, with its Agent Card under it, the shape
of the server's A2A endpoint. The protocol is the A2A Rust SDK's
(`a2a-server-lf`) request handler; serve supplies the executor. The agent card
lists the routes under `a2a`, and the manifest under `routes`.

- **Contexts.** Each `contextId` maps to one session (channel key
  `a2a:{agent}` in the thread map), so it survives a restart. A message
  without one gets a new context, and so a new session.
- **Tasks.** One task is one turn: `working`, the final reply as one
  `response` artifact, then `completed`; a failed turn is `failed` with the
  error as the status message. `SendMessage` blocks until then. Tasks live in
  memory: after a restart `GetTask` no longer knows them.
- **Input.** Text parts, and data parts as JSON, joined in order. A message
  with neither is a JSON-RPC `-32602`. File parts are ignored.
- **Version.** A2A 1.0 only: send `A2A-Version: 1.0`. An absent header means
  0.3, which is refused.
- **Interrupts.** A pending approval or `ask_user` question keeps the task
  `working` until the routes above answer it. `CancelTask` cancels the turn
  running on the context's session.
- **Errors.** An unknown agent or a subagent is a `404` problem.


## Voice

With the `voice` cargo feature, every top-level agent also takes browser voice
calls through `everruns::voice`, the voice loop the server's voice channels
run. The agent card lists the settings and each agent's base route under
`voice`, and the manifest lists the routes under `routes`.

- **Settings.** `[voice]` in `serve.toml` is the platform's
  `VoiceChannelConfig` (voice, greeting, turn detection, interruption, filler,
  speaking style), one for the whole app. An invalid section stops the host at
  startup.
- **Provider.** OpenAI Realtime with `OPENAI_API_KEY`. Without a key, `dev` and
  `eval` use the offline simulator; `start` answers `400`. A `sim` model
  forces the simulator.
- **Calls.** A call without `session_id` creates a session tagged `voice`
  (metadata `channel: voice`). With one, the session must belong to the same
  agent (`400` otherwise). Each finished utterance is a user message with
  `metadata.source = "voice"`; the answer is spoken as it streams.
- **Lifetime.** Calls live in the process that placed them, by provider call
  id; `end` on another replica is `404`. Nothing about a call is persisted
  beyond the session's own messages.
- **Errors.** An empty `sdp` or invalid settings are `400`; an unknown agent,
  a subagent, an unknown session or an unknown call is `404`.
## Differences from the server

- No auth, organizations, agents CRUD, harnesses, workspaces or files routes.
- `agent_name` names a serve agent from `#[agent]`; `agent_id` is that name,
  not an `agent_…` id. The SDK validates `agent_name` as kebab-case, so an
  agent reachable through the SDK needs a name like `analyst`, not `run_sql`.
- No `tool-results` route (serve has no client-side tools); approvals use the
  serve-only route above.
- The A2A endpoint speaks 1.0 only (the server also speaks 0.3), has no auth
  or HMAC signing, and keeps tasks in memory.
- `events` supports the listed filters only (no `around`, `q`, `turn_id` …).

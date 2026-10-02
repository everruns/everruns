---
title: Serve AG-UI
description: Stream a Framework session to CopilotKit or any AG-UI 1.0 client from your own HTTP server, with ask_user questions and tool approvals as interrupts.
---

[AG-UI](https://docs.ag-ui.com) is the event protocol between an agent and the
application that renders it. CopilotKit and `@ag-ui/client` post a
`RunAgentInput` and read back a stream of events, usually as server-sent
events. The `ag-ui` feature lets any Rust host answer those requests from a
Framework session, without the Everruns server:

```bash
cargo add everruns --features ag-ui
```

`Session::ag_ui(input)` sends the input's last user message and returns the run
as a stream of AG-UI 1.0 events: `RUN_STARTED`, the projected turn, then one
`RUN_FINISHED` or `RUN_ERROR`. The projection is the one the Everruns server
uses, with the trusted policy: assistant text, reasoning, and token usage are
visible, and a failure carries the runtime's message.

## Mount it on axum

`AgUiThreads` maps each AG-UI `threadId` to a session: the first run of a
thread creates one from your agent, later runs reopen it. Turn each event into
an SSE frame:

```rust
use std::convert::Infallible;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use everruns::ag_ui::{AgUiError, AgUiOptions, AgUiThreads, InterruptGate, RunAgentInput};
use everruns::{Agent, Engine, OpenAI};
use futures::StreamExt;

#[derive(Clone)]
struct App {
    threads: AgUiThreads,
    gate: InterruptGate,
}

async fn ag_ui(State(app): State<App>, Json(input): Json<RunAgentInput>) -> Response {
    let options = AgUiOptions::new().gate(app.gate.clone());
    match app.threads.run(input, options).await {
        Ok(run) => {
            let events = run.map(|event| {
                Ok::<_, Infallible>(SseEvent::default().json_data(&event).unwrap_or_default())
            });
            Sse::new(events).keep_alive(KeepAlive::default()).into_response()
        }
        Err(AgUiError::InvalidInput(why)) => (StatusCode::BAD_REQUEST, why).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gate = InterruptGate::new();
    let agent = Agent::builder()
        .instructions("You are a helpful assistant.")
        .provider(OpenAI::from_env()?)
        .model("gpt-5.6-terra")
        .ask_user(gate.clone())
        .approver(gate.clone())
        .build()?;
    let app = App {
        threads: AgUiThreads::new(Engine::new(), agent),
        gate,
    };
    let router = Router::new().route("/ag-ui", post(ag_ui)).with_state(app);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, router).await?;
    Ok(())
}
```

The crate's `ag_ui_axum` example is the same server on the offline simulated
model:

```bash
cargo run -p everruns --features ag-ui --example ag_ui_axum
```

Point any AG-UI client at the route:

```ts
import { HttpAgent } from "@ag-ui/client";

const agent = new HttpAgent({ url: "http://127.0.0.1:3000/ag-ui" });
```

## Threads across restarts

`AgUiThreads::new` keeps its thread map in memory. To keep threads when the
process restarts, keep both halves on disk: the sessions, with an agent on the
`local` backend, and the map, with `SqliteThreadStore` (both need the `local`
feature):

```rust
use everruns::ag_ui::{AgUiThreads, SqliteThreadStore};
use everruns::{Agent, Engine, LocalConfig, OpenAI};

let config = LocalConfig::new("/var/lib/my-app");
let agent = Agent::builder()
    .instructions("You are a helpful assistant.")
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .local(config.clone())
    .build()?;
let threads = AgUiThreads::with_store(Engine::new(), agent, SqliteThreadStore::local(&config)?);
```

After a restart the next run of a thread reopens its session, history
included. Implement `ThreadStore` to keep the map in your own database
instead.

A thread id is chosen by the client, and anyone who knows one continues that
conversation. When one server serves several users, scope threads to the
authenticated caller with `run_in`:

```rust
let run = app.threads.run_in(&user_id, input, options).await?;
```

Thread ids must be 1 to 128 characters of `[A-Za-z0-9-_.]`; anything else is
`AgUiError::InvalidInput`.

## History the client already has

A client such as CopilotKit keeps the conversation and sends all of it with
every run. When a thread is new to the server, for example after a restart
without a durable backend, `AgUiThreads` records the input's earlier user and
assistant messages as the new session's history before the run's user message,
so the agent picks up where the conversation was. Once the session has
history, the earlier messages are not read again: the session's own
conversation wins.

Only user and assistant text is seeded (an assistant's tool calls as text);
`tool`, `system`, `developer`, activity and reasoning messages are not. The
most recent 256 messages and 512 KiB of text are kept. On a session you
resolve yourself, turn it on with `AgUiOptions::seed_history(true)` for a
thread's first run. The client writes these messages, assistant turns
included, so treat them as no more trusted than the user message.

## Questions and approvals become interrupts

In the Framework, `ask_user` questions and tool approvals are answered by
responders on the agent while the turn waits. `InterruptGate` is a responder
for both that answers through AG-UI instead: register it with `.ask_user(...)`
and `.approver(...)`, and pass it with `AgUiOptions::gate`. A question or
approval then ends the run with the 1.0 interrupt outcome while the turn stays
parked in your process:

| Parked on | `reason` | Answer (`resume[].payload`) |
|---|---|---|
| `ask_user` | `everruns.ask_user` | `{ "answers": [{ "id", "selected", "other_text" }], "status"? }`, described by the interrupt's `responseSchema` |
| `ask_user` with a `secret` question | `everruns.secret_required` | none: only abandoning is accepted |
| a tool that needs approval | `tool_approval` | `{ "decision": "allow" \| "allow_always" \| "reject" \| "reject_always" }` |

The interrupt id is the tool call id. The next run sends `resume` entries and
no new message; the entries answer the parked requests and the run streams the
rest of the same turn. Abandoning an entry (`status: "cancelled"`) declines the
question or rejects the call. An interrupt left without an entry is not
abandoned: nothing is resolved and the run ends with the interrupts again, as
it does when a new message arrives while one is open. An entry that cannot be
applied returns `AgUiError::InvalidInput` before the stream opens, and nothing
is resolved.

Parked requests live in memory. If the process exits, the waiting turn ends
cancelled.

## Policy

`AgUiOptions::policy` takes a `ProjectionPolicy` to hide reasoning or usage,
show fixed text while tools run, or map failures to your own error codes:

```rust
use everruns::ag_ui::{AgUiOptions, ProjectionPolicy};

let options = AgUiOptions::new().policy(ProjectionPolicy {
    reasoning_visible: false,
    usage_visible: false,
    tool_activity_text: Some("Working on it...".into()),
    ..ProjectionPolicy::default()
});
```

## What a run reads from the input

The session owns the conversation, so only the last message is sent, and it
must be a user message (its text parts), unless the run carries frontend tool
results (below). Earlier messages are read only to seed a new thread (above);
`state` and `forwardedProps` are not read. `RUN_STARTED` carries `protocolVersion: "1.0"` only when the request
declared a version, so pre-1.0 clients see the stream they expect.

## Frontend tools

The input's `tools` are tools your page runs, such as a confirmation dialog or
a navigation action. Each run makes them the session's client-side tools, so
send them on every run; the latest set wins. When the model calls one, the
turn parks and the run streams the call as `TOOL_CALL_START`, `TOOL_CALL_ARGS`
and `TOOL_CALL_END` with its name and arguments, then finishes in success with
`outcome.pendingToolCallIds`.

Run the calls in the page, then post the next run with their results as
trailing `tool` messages (`toolCallId` set, `content` the result, `error` for
a failure). The same turn continues and the run streams the rest of it. A
result for a call that is not parked is ignored. Until every parked call has a
result, nothing is recorded and the run reports the calls again, as it does
when a new user message arrives instead.

Definitions are bounded: at most 64 tools, names matching
`^[A-Za-z0-9_-]{1,64}$`, descriptions up to 4096 characters, parameter
schemas up to 16 KiB, results up to 256 KiB. Names starting with `mcp_` are
refused. Parked calls live in memory: if the process exits, they stay
unanswered.

`system` and `developer` messages and `context` entries are ignored by
default, and a system message after the user message is refused. If your
server authenticates whoever posts the input, you can let each run carry
instructions:

```rust
use everruns::ag_ui::AgUiOptions;

let options = AgUiOptions::new().input_instructions(true);
```

Each run's system and developer messages, in order, then its context entries,
are then appended to the agent's instructions for that run. They may appear
anywhere in `messages`, including after the user message, which stays the
run's input. The next run replaces them with its own, and a run with none
clears them; they never enter the conversation history. Leave this off for
callers you do not trust: these messages carry the authority of the system
prompt.

A [serve](/framework/serve/#ag-ui-and-copilotkit) app gets this route built
in with its `ag-ui` feature, at `/v1/e/{agent}/ag-ui`. For a hosted agent with
no server code, Everruns serves the same protocol at
`/v1/e/{endpoint_id}/ag-ui`.

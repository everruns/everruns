# a2a (serve, experimental)

Two agents talking over [A2A](https://a2a-protocol.org) 1.0:

- `researcher` is a [serve](../../../crates/serve) app. The `a2a` feature serves
  it at `POST /v1/e/researcher/a2a`, with its Agent Card at
  `/v1/e/researcher/a2a/.well-known/agent-card.json`, the same route shape as
  an Everruns A2A endpoint.
- `writer` is an `everruns` agent with the `a2a_agent_delegation` capability.
  It asks the researcher for notes with `spawn_agent` (target
  `external_a2a`), then drafts from them.

Both run offline: without `OPENAI_API_KEY`, each agent follows a scripted
simulator, and the message between them still goes over A2A.

```sh
cargo run -p serve-example-a2a --bin researcher                 # dev server on :3000
cargo run -p serve-example-a2a --bin writer -- "tide pools"     # in another shell
cargo run -p serve-example-a2a --bin researcher -- eval         # 1 passed
```

The writer's run, offline:

```text
QUESTION
Write a short explainer on tide pools.

> spawn_agent
  instructions: Research tide pools for a short explainer.
  mode: foreground
  name: research
  target: {"id":"researcher","type":"external_a2a"}
  wait_timeout_secs: 60

< spawn_agent: OK
kind: "external_a2a"
remote_context_id: "01a0fd6e-feee-73a5-bab0-fa8b475d8c91"
remote_task_id: "01a0fd6e-feee-73a5-bab0-fa8ae635a09f"
result: "- Tide pools form where rock holds seawater as the tide goes out.\n- Residents such as ..."
status: "completed"
...

ANSWER

(Offline demo reply: the writer's simulator does not read the notes; ...)
```

With `OPENAI_API_KEY` set for both processes, the researcher answers on
`gpt-5.6-terra` and the writer drafts its paragraph from those notes.
`RESEARCHER_URL` points the writer at another A2A base URL, such as a
researcher started on another port (`-- dev --port 3001`).

## Poke it with the `a2a` CLI

The official [A2A CLI](https://github.com/a2aproject/a2a-cli) takes the card
URL:

```sh
CARD=http://localhost:3000/v1/e/researcher/a2a/.well-known/agent-card.json
a2a card get $CARD
a2a send -a $CARD --context-id demo "Tide pools"
a2a send -a $CARD --context-id demo --stream "Now kelp forests"   # same session
a2a task list -a $CARD
```

```text
[status] working
[artifact] - Tide pools form where rock holds seawater as the tide goes out.
...
[status] completed
```

## Or with curl

Requests need `A2A-Version: 1.0`:

```sh
URL=http://localhost:3000/v1/e/researcher/a2a
curl -s $URL -H 'content-type: application/json' -H 'A2A-Version: 1.0' -d '{
  "jsonrpc": "2.0", "id": 1, "method": "SendMessage",
  "params": { "message": {
    "messageId": "m1", "role": "ROLE_USER", "contextId": "demo",
    "parts": [{ "text": "Tide pools" }] } } }'
```

The result is the completed task, with the reply as its `response` artifact.
`SendStreamingMessage` with the same body streams `statusUpdate` (working),
`artifactUpdate`, then `statusUpdate` (completed) as SSE.

| File | Is |
|---|---|
| `agent/instructions.md` | the researcher's prompt (hot-reloads in `dev`) |
| `src/agent.rs` | `#[agent] fn researcher()` |
| `src/main.rs` | the serve app (`--bin researcher`) |
| `src/bin/writer.rs` | the delegating `everruns` agent (`--bin writer`) |
| `evals/notes.rs` | `#[eval] async fn answers_with_notes(t)` |

Each A2A `contextId` maps to one researcher session, which survives a restart;
tasks are held in memory. The writer needs no extra setup: the `a2a` Cargo
feature plus the `a2a_agent_delegation` capability is the opt-in, and the
runtime records delegated results under `/workspace/.agent-runs` itself, so the
default read-only workspace policy stays in place.

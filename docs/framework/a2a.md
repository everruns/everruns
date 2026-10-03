---
title: A2A
description: Serve Framework agents to other agents over A2A 1.0, and let a Framework agent delegate work to remote A2A agents.
---

[A2A](https://a2a-protocol.org) is an open protocol for agents to call each
other over HTTP: a JSON-RPC endpoint plus a public Agent Card that says what the
agent is and how to reach it. A Framework app can use it in both directions:

- **Serve**: other agents, the official `a2a` CLI, or the A2A Inspector call
  your agents. This comes from the `a2a` feature of
  [`everruns-serve`](/framework/serve/).
- **Call**: your agent hands work to a remote A2A agent with `spawn_agent`. This
  comes from the `a2a` feature of `everruns` and the `a2a_agent_delegation`
  capability.

Both speak A2A 1.0. The routes match an Everruns
[A2A endpoint](/features/a2a/), so a caller can move between a serve app and
Everruns by changing the base URL.

## Serve your agents over A2A

Turn on the feature:

```toml
[dependencies]
everruns-serve = { version = "*", features = ["a2a"] }
```

Every top-level agent in the app is then served at:

| Route | What it is |
| --- | --- |
| `POST /v1/e/{agent}/a2a` | A2A 1.0 JSON-RPC. Requests need the `A2A-Version: 1.0` header. |
| `GET /v1/e/{agent}/a2a/.well-known/agent-card.json` | The Agent Card, built from the agent's name and `description`. |

Nothing else changes in the agent. A `description` is worth setting, because
it is what other agents read on the card:

```rust
use serve::prelude::*;

/// Researches a topic and answers with short factual notes.
#[agent]
fn researcher() -> Agent {
    Agent::builder()
        .model("openai/gpt-5.6-terra")
        .description("Researches a topic and answers with short factual notes.")
        .instructions(md!("instructions.md"))
        .build()
}
```

Try it with the `a2a` CLI against a dev server:

```bash
CARD=http://localhost:3000/v1/e/researcher/a2a/.well-known/agent-card.json
a2a card get -a "$CARD"
a2a send -a "$CARD" "Tide pools"
a2a send -a "$CARD" --stream "Coral reefs"
a2a task list -a "$CARD"
```

Pass the full card URL: the card lives under the agent's route, not at the
server root.

### How A2A maps onto serve

- Each A2A `contextId` is one serve session. A follow-up message with the same
  `contextId` continues the conversation, and the session survives a restart.
- Each A2A task is one turn. A task goes `working`, then delivers the final
  reply as a `response` artifact, then `completed` (or `failed`).
- `SendMessage`, `SendStreamingMessage`, `GetTask`, `ListTasks`, `CancelTask`,
  and `SubscribeToTask` are supported.
- A pending tool approval or `ask_user` question keeps the task `working`
  until it is answered through serve's `/v1` routes.

### Limits

- Only A2A 1.0 is served. Requests without `A2A-Version: 1.0` are rejected.
- Tasks are kept in memory: after a restart, old task ids are unknown, while
  their contexts continue.
- There is no authentication beyond what you put in front of the server, the
  same as the other serve routes. The card advertises no security scheme.
- Push notifications are not offered. Use streaming or `GetTask`.

For a hosted endpoint with API keys, push notifications, and A2A 0.3 support,
use an Everruns [A2A endpoint](/features/a2a/).

## Call remote A2A agents

Turn on the feature and add the capability, listing the agents your agent may
call:

```toml
[dependencies]
everruns = { version = "*", features = ["a2a", "local", "openai"] }
```

```rust
use everruns::{Agent, CapabilityRef, Engine, LocalConfig, OpenAI};
use serde_json::json;

let agent = Agent::builder()
    .name("writer")
    .instructions(
        "Before writing, ask the `researcher` agent for notes with `spawn_agent` \
         (target type `external_a2a`, id `researcher`, mode `foreground`). \
         Then write one paragraph from its notes.",
    )
    .provider(OpenAI::from_env()?)
    .model("gpt-5.6-terra")
    .capability(CapabilityRef::new("a2a_agent_delegation").config(json!({
        "agents": [{
            "id": "researcher",
            "name": "Researcher",
            "description": "Researches a topic and answers with short factual notes.",
            "base_url": "https://agents.example.com/v1/e/researcher/a2a"
        }]
    })))
    // Delegated runs are recorded in session storage.
    .local(LocalConfig::new("./writer-state"))
    .build()?;

let session = Engine::new().create(agent);
```

The model sees one `spawn_agent` tool with an `external_a2a` target. It can
only reach the agents in `agents`; it never supplies a URL.

- `base_url` is where the Agent Card is resolved
  (`{base_url}/.well-known/agent-card.json`).
- `mode: "foreground"` waits for the remote task and returns its reply as the
  tool result. `mode: "background"` returns a `task_id` at once and wakes the
  session when the remote task finishes.
- `result_schema` on `spawn_agent` requires a structured result and validates
  the remote agent's first data part against it.
- URLs are checked before each call: localhost, private ranges, and metadata
  addresses are refused, resolved addresses are pinned, and redirects are not
  followed. Set `"allow_local_urls": true` on an agent entry only while
  developing against a server on your machine under `DEPLOYMENT_GRADE=dev`.

The remote agent can be anything that speaks A2A 1.0: a serve app, an Everruns
A2A endpoint, or another vendor's agent.

## Run the example

[`examples/serve/a2a`](https://github.com/everruns/everruns/tree/main/examples/serve/a2a)
puts both halves together. `researcher` is a serve app served over A2A, and
`writer` is an `everruns` agent that delegates research to it and then drafts.
It runs offline with scripted models, and on OpenAI when `OPENAI_API_KEY` is
set:

```bash
cargo run -p serve-example-a2a --bin researcher              # serves on :3000
cargo run -p serve-example-a2a --bin writer -- "tide pools"  # in another shell
```

The writer's `spawn_agent` call goes over real A2A even offline, so the
example is also a quick way to check an A2A setup end to end.

## See also

- [Serve](/framework/serve/#a2a), the serve routes and configuration.
- [A2A](/features/a2a/), the Everruns A2A endpoint and the hosted delegation
  capability.
- [Serve AG-UI](/framework/ag-ui/), the protocol for agent-to-UI rather than
  agent-to-agent.

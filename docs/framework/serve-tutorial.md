---
title: "Tutorial: Serve an Agent and Call It from Code"
description: Build an agent with serve, protect it with an API key, and call it from Python, TypeScript or Rust with the SDK's AgentClient.
appliesTo: [framework]
---

> **Experimental.** [serve](/framework/serve/) is a proof of concept. Its APIs
> will change and it has no compatibility promise.

By the end of this tutorial you will have an order-status agent running on
your machine as a serve app, with one tool, a real model and an API key in
front of it, and a short program in Python, TypeScript or Rust that talks to
it with the SDK's `AgentClient`.

This is a guided lesson, so it makes the choices for you. The
[serve reference](/framework/serve/) covers everything it leaves out.

## What you need

- Rust 1.94 or later, with `cargo`.
- An OpenAI API key in `OPENAI_API_KEY`. The agent uses `openai/gpt-6.1-sol`.
- For the client: Python 3.10 or later, Node.js 22 or later, or Rust.
- `curl` and `jq` for the checks along the way.

## 1. Create the project

```sh
cargo new order-desk
cd order-desk
mkdir -p agent src/tools
```

Replace the dependencies in `Cargo.toml`:

```toml
[dependencies]
everruns-serve = "0.48"
tokio = { version = "1", features = ["full"] }

[build-dependencies]
everruns-serve-build = "0.48"
```

API keys need `everruns-serve` 0.48 or later.

The package is `everruns-serve` and the library is imported as `serve`. Add a
`build.rs` next to `Cargo.toml`, so the prompt in `agent/` and `serve.toml` are
compiled into the binary:

```rust ignore
// build.rs
fn main() {
    serve_build::embed();
}
```

Name the app in `serve.toml`:

```toml
name = "order-desk"
```

## 2. Write the agent

The prompt goes in `agent/instructions.md`:

```markdown
You answer customer questions about their orders for an online shop.

When someone asks about an order, call `order_status` with its number and
answer in one or two sentences. If the tool finds no such order, say so and
ask for the number again. Never guess a status.
```

The tool goes in `src/tools/order_status.rs`. The doc comments become the
descriptions the model reads, and the function name becomes the tool name:

```rust ignore
use serve::prelude::*;

/// Look up the status of an order by its number.
#[tool]
async fn order_status(
    cx: &Cx,
    /// The order number, for example 42.
    order: u32,
) -> Result<String> {
    cx.progress(format!("looking up order {order}")).await;
    // A real app would query its order database here.
    match order {
        42 => Ok("Shipped October 9 with UPS, tracking 1Z999AA10123456784, arrives October 13.".into()),
        43 => Ok("Paid. Ships within two business days.".into()),
        _ => bail!("there is no order {order}"),
    }
}
```

`src/tools/mod.rs` declares it:

```rust ignore
mod order_status;
```

Finally the agent and `main` in `src/main.rs`:

```rust ignore
mod tools;

use serve::prelude::*;

/// Answers questions about orders.
#[agent]
fn support() -> Agent {
    Agent::builder()
        .model("openai/gpt-6.1-sol")
        .instructions(md!("instructions.md"))
        .build()
}

serve::assets!();

#[tokio::main]
async fn main() -> serve::Result {
    serve::start(serve::App::builder().discover().build()).await
}
```

`#[agent]` and `#[tool]` register the function where it is defined, and
`discover()` collects them, so nothing lists the tool by hand. The agent's
doc comment is its description, and its function name, `support`, is its name
in URLs.

## 3. Run it

```sh
export OPENAI_API_KEY=sk-...
cargo run
```

The first build takes a few minutes. Then the app prints what it found:

```text
  serve · order-desk 0.1.0 · Dev · build b49b68ee33c43b82d
  ⚠ experimental proof of concept; APIs will change

  agent    support (default) · openai/gpt-6.1-sol → openai
  tool     order_status
  sandbox  none

  data     .serve
  listen   http://localhost:3000
```

`→ openai` says the model goes to OpenAI with your key. If it says
`simulator`, `OPENAI_API_KEY` is not set in this shell. A set
`OPENROUTER_API_KEY` or `SERVE_GATEWAY_URL` takes precedence over
`OPENAI_API_KEY`; see [Try it](/framework/serve/#try-it).

`cargo run` with no command is `dev`: sessions are kept in SQLite under
`.serve/`, the terminal shows each turn as it happens, and edits to
`instructions.md` apply without a rebuild.

## 4. Talk to it with curl

The agent's API lives at its own base URL, `/v1/channels/support`. In a
second terminal, read its card:

```sh
curl -s localhost:3000/v1/channels/support
```

```json
{"name":"support","description":"Answers questions about orders.","streaming":true,"input":{"text":true,"images":false,"files":false},"auth":[],"links":{"sessions":"/v1/channels/support/sessions"}}
```

Start a session, send a message, and follow the session's events:

```sh
AGENT=localhost:3000/v1/channels/support
ID=$(curl -s -X POST $AGENT/sessions -H 'content-type: application/json' -d '{}' | jq -r .id)

curl -s -X POST $AGENT/sessions/$ID/messages -H 'content-type: application/json' \
  -d '{"message":{"role":"user","content":[{"type":"text","text":"Where is order 42?"}]}}'

curl -N "$AGENT/sessions/$ID/sse?after_sequence=0"
```

The stream shows the turn as server-sent events: `turn.started`,
`tool.started` and `tool.completed` for the lookup, `output.message.delta` as
the answer streams, and `turn.completed`. The stream stays open for the next
turn; stop it with Ctrl+C. The first terminal shows the same turn:

```text
21:42:34 4cc33a ● new session on support
21:42:34 4cc33a ▸ Where is order 42?
21:42:36 4cc33a   ⚙ order_status {"order":42}
21:42:36 4cc33a   … looking up order 42
21:42:36 4cc33a   ✓ order_status
21:42:38 4cc33a ◂ Order 42 shipped on October 9 via UPS and is expected to arrive October 13. Your tracking number is **1Z999AA10123456784**.
```

## 5. Protect it with an API key

Anyone who can reach port 3000 can use the agent so far. Stop the app with
Ctrl+C, make a key, and start it in production mode with that key:

```sh
export AGENT_KEY=$(openssl rand -hex 32)
SERVE_API_KEYS=$AGENT_KEY cargo run -- start
```

`start` is production mode: a model with no route is an error instead of a
simulator. `SERVE_API_KEYS` takes a comma-separated list, so each application
can get its own key and you can rotate one at a time. Keep keys in the
environment: `serve.toml` is compiled into the binary.

The startup banner's sample requests now carry an `authorization` header.
Without a key, every session route answers `401`:

```sh
curl -si -X POST localhost:3000/v1/channels/support/sessions -H 'content-type: application/json' -d '{}'
```

```text
HTTP/1.1 401 Unauthorized
www-authenticate: Bearer
...
{"detail":"a valid bearer credential is required","status":401,"title":"Unauthorized"}
```

With the key as a bearer token it works again:

```sh
curl -s -X POST localhost:3000/v1/channels/support/sessions \
  -H "authorization: Bearer $AGENT_KEY" -H 'content-type: application/json' -d '{}'
```

The card stays public so clients can find the agent, and its `auth` list now
says which credential it takes: `"auth":[{"type":"agent_key"}]`. API keys are
one of three methods; [Authentication](/framework/serve/#authentication) covers
OpenID Connect and OAuth 2.0 introspection.

## 6. Call it from code

Every Everruns SDK has an `AgentClient` for one agent's API. It reads the
agent's base URL from `EVERRUNS_AGENT_URL` and the credential from
`EVERRUNS_AGENT_KEY`, and sends the credential as a bearer token. In the
terminal where you will run the client:

```sh
export EVERRUNS_AGENT_URL=http://localhost:3000/v1/channels/support
export EVERRUNS_AGENT_KEY=$AGENT_KEY   # the key from step 5
```

Pick one language.

### Python

```sh
python3 -m venv .venv
. .venv/bin/activate
pip install "everruns-sdk>=0.3.1"
```

Save this as `ask.py`:

```python
import asyncio

from everruns_sdk import AgentClient


async def main():
    async with AgentClient() as agent:
        card = await agent.card()
        print(f"Agent: {card['name']}")

        session = await agent.create_session()
        print(await agent.run("Where is order 42?", session["id"]))
        print(await agent.run("And order 43?", session["id"]))


asyncio.run(main())
```

```sh
python ask.py
```

```text
Agent: support
Order 42 shipped October 9 via UPS and is expected to arrive October 13. Your tracking number is **1Z999AA10123456784**.
Order 43 is paid and will ship within two business days.
```

### TypeScript

```sh
npm init -y
npm pkg set type=module
npm install @everruns/sdk tsx
```

Save this as `ask.ts`:

```typescript
import { AgentClient } from "@everruns/sdk";

const agent = new AgentClient();
const card = await agent.card();
console.log(`Agent: ${card.name}`);

const session = await agent.createSession();
console.log(await agent.run("Where is order 42?", session.id));
console.log(await agent.run("And order 43?", session.id));
```

```sh
npx tsx ask.ts
```

### Rust

```sh
cargo new ask
cd ask
cargo add everruns-sdk tokio --features tokio/full
```

Replace `src/main.rs`:

```rust ignore
use everruns_sdk::AgentClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = AgentClient::from_env()?;
    println!("Agent: {}", agent.card().await?.name);

    let session = agent.create_session(None, None).await?;
    println!("{}", agent.run("Where is order 42?", Some(&session.id)).await?);
    println!("{}", agent.run("And order 43?", Some(&session.id)).await?);
    Ok(())
}
```

```sh
cargo run
```

TypeScript and Rust print the same three lines as Python.

`run` sends one message, follows the event stream until the turn ends, and
returns the agent's reply. Passing the session id keeps both questions in one
conversation, which is why "And order 43?" needs no more context. Without a
session id, `run` starts a new session each time. For more control, the client
has the session calls directly: `create_session`, `send_message`,
`stream_events`, `list_events`, `cancel`, and the answers to questions and tool
approvals.

## What you built

- A serve app with one agent and one tool, running on a real model.
- An API key in front of the agent's session routes, with the card left public.
- A client that holds only that key and the agent's URL.

The same client code works against an agent hosted on Everruns: point
`EVERRUNS_AGENT_URL` at the agent's Agent API channel and use an agent key. See
[Call your agent from code](/how-to/call-your-agent-from-code/).

## Next steps

- Hold a tool call for a person's approval with
  `#[tool(needs_approval)]`, and answer it with `submit_tool_approvals`. See
  [The pieces](/framework/serve/#the-pieces).
- Check the agent's behavior with `#[eval]` and `cargo run -- eval`.
- Run the app on [Amazon Bedrock AgentCore](/framework/serve-agentcore/), or
  keep its data in a bucket with [`start --store`](/framework/serve/#keep-the-data-in-a-bucket).

---
title: Call your agent from code
description: Give an application its own key to one agent, call it from Python, TypeScript, Rust, or curl, act for your own users, and reach it safely from a browser.
appliesTo: [platform, cloud]
---

An application that only needs to talk to one agent should not hold a personal access token, which reaches everything you can manage. Give it an **agent key** instead: it works on one agent's session routes and nothing else. This guide uses the Agent API channel ([Channels](/features/channels/)).

## 1. Publish the agent with an Agent API channel

In the agent's **Channels** tab, add an **Agent API** channel, then create a key in its **Agent keys** card. The secret (`evr_ak_…`) is shown once. The channel shows its **agent URL**, for example `https://app.everruns.com/api/v1/channels/apichan_…`.

Name each key after the application that holds it, so you know which one to revoke. Rotating gives a new secret and keeps the old one working for an overlap you choose, so you can deploy the new secret first.

## 2. Call it

Every SDK has an `AgentClient` for this. It reads `EVERRUNS_AGENT_URL` and `EVERRUNS_AGENT_KEY` from the environment.

```python
from everruns_sdk import AgentClient

async with AgentClient() as agent:
    reply = await agent.run("Where is order 42?")
    print(reply)
```

```typescript
import { AgentClient } from "@everruns/sdk";

const agent = new AgentClient();
console.log(await agent.run("Where is order 42?"));
```

```rust
use everruns_sdk::AgentClient;

let agent = AgentClient::from_env()?;
println!("{}", agent.run("Where is order 42?", None).await?);
```

`run` starts a session, sends the message, follows the event stream until the turn ends, and returns the agent's reply. Pass a session id to continue a conversation. For full control use the session calls directly: `create_session`, `send_message`, `stream_events`, `list_events`, `cancel`, and the answers to questions and tool approvals.

Without an SDK, the same calls are plain HTTP:

```bash
curl -X POST "$EVERRUNS_AGENT_URL/sessions" \
  -H "Authorization: Bearer $EVERRUNS_AGENT_KEY"

curl -X POST "$EVERRUNS_AGENT_URL/sessions/$SESSION_ID/messages" \
  -H "Authorization: Bearer $EVERRUNS_AGENT_KEY" \
  -H "Content-Type: application/json" \
  -d '{"message":{"role":"user","content":[{"type":"text","text":"Where is order 42?"}]}}'

curl -N "$EVERRUNS_AGENT_URL/sessions/$SESSION_ID/sse?after_sequence=0" \
  -H "Authorization: Bearer $EVERRUNS_AGENT_KEY"
```

A key sees only the sessions it started. What the stream shows is the channel's **visibility**: by default messages plus "a tool ran", never tool names, arguments, or internal errors.

## 3. Retry safely

Send an `Idempotency-Key` header (the SDKs take an `idempotency_key` argument) when you create a session or send a message. A retry with the same key and the same request within 24 hours returns the first response instead of starting a second session or sending the message twice.

## 4. Act for your own users

If your application has users, let the agent keep their conversations apart and use their own connections. Create the key with the **end user** permission and name the user on each request:

```python
alice = agent.for_end_user("customer-42")
await alice.run("Where is my order?")
```

The SDKs send this as an `End-User` header. Sessions then belong to that user: the key acting as itself, or for another user, does not see them.

Instead of a key, your users can also bring a token from your own identity provider: list it in the channel's `auth_methods` (OpenID Connect or OAuth 2.0 introspection). Everruns only validates those tokens. Members of your organization can use their own personal access token when you turn on **Organization members**.

## 5. Call it from a browser

Never put an agent key in a browser or mobile app. Your backend exchanges it for a short-lived **runtime token** for one user, and the page uses that:

```python
token = await agent.for_end_user("customer-42").runtime_token()
# hand token["access_token"] to the page; it lasts 15 minutes and works on this agent only
```

List the page's origin, such as `https://app.example.com`, under **Browser origins** on the channel so the browser allows the calls.

## 6. Cap what one caller can spend

Set **Daily spending limit per caller** on the channel. When one key or one end user reaches it, starting a session or sending a message answers `429` until the next UTC day. Reading sessions still works, and a turn already running finishes.

## Related

- [Channels](/features/channels/): the Agent API channel's settings in full.
- [Stream events](/how-to/stream-events/): the event stream and reconnection.
- [Handle errors and cancel turns](/how-to/handle-errors-and-cancellation/).

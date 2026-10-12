---
title: Explicit Communication
description: Let an agent keep its assistant text as private working notes and talk only by sending messages, so it can choose what to say, when, and whether to answer at all.
sidebar:
  label: Explicit Communication
appliesTo: [framework, platform, cloud]
---

Every Agent has a **Communication** setting that decides what counts as the
Agent talking:

- **Direct** (the default): the Agent's assistant text is its reply. This is how
  most chat agents work.
- **Explicit**: the Agent's assistant text is private working notes. The Agent
  talks only by calling the `send_message` tool, and can call `no_reply` to
  decide not to answer.

With explicit communication the Agent separates thinking out loud from speaking.
It can work through a problem in notes, send one complete answer, post a short
progress update during long work, or stay silent when a message is not for it.

## When to use it

Explicit communication fits conversations where saying nothing, or saying one
deliberate thing, matters:

- **Slack channels and threads.** Only the messages the Agent sends reach the
  thread. Its intermediate reasoning and tool preambles stay in Everruns.
- **Conversations with several people.** The Agent can let people talk among
  themselves and answer only when it has something to add.
- **Coordinators.** An Agent that delegates work can keep its planning in notes
  and send a single summary when the work is done.
- **Agents that should be able to stay silent.** `no_reply` ends a turn on
  purpose, for an acknowledgement or a message that needs no answer.

## When not to use it

- **Simple one-person chats.** Direct communication is simpler and streams the
  reply as the Agent writes it.
- **Weaker models.** The Agent must remember to call `send_message`. A model
  that is unreliable at tool calls may finish a turn without sending anything,
  and the person then sees no answer. Test with the model you plan to use.

## Turn it on

### In the web UI

Open the Agent, or the new-agent page, and set **Communication** to
**Explicit**. Save the Agent.

### Through the API

Set `communication` when you create or update an Agent. The field is returned
when you read the Agent.

```json
{
  "name": "release-coordinator",
  "system_prompt": "You coordinate releases for the team.",
  "communication": "explicit"
}
```

Send `"communication": "direct"` to switch back.

### In an agent package

Add `communication` to `agent.toml`. Omitting it means `direct`. See
[Agent Package](/reference/agent-package/).

```toml
schema_version = 1
name = "release-coordinator"
instructions = "You coordinate releases for the team."
communication = "explicit"
```

### In the Framework

```rust
use everruns::conversation::Communication;
use everruns::prelude::*;

let agent = Agent::builder()
    .instructions("You coordinate releases for the team.")
    .model("gpt-5.6-terra")
    .communication(Communication::Explicit)
    .build()?;
```

The session event stream reports each sent message as
`SessionEventKind::MessageSent { message_id, text }`. The
[`explicit_communication`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/explicit_communication.rs)
example runs offline and prints the sent messages separately from the notes:

```bash
cargo run -p everruns --example explicit_communication
```

## The two tools

An Explicit Agent gets two tools and a short system prompt section, "How you
talk", that explains them to the model. You do not add them yourself.

| Tool | Arguments | What it does |
|---|---|---|
| `send_message` | `text` (required): the complete message, in Markdown | Sends the message to the conversation the turn came from. Success means it was delivered. |
| `no_reply` | `reason` (optional): why no reply is needed | Ends the turn without saying anything. The reason stays in the transcript and is never shown to the person. |

The Agent never chooses where a message goes. The destination is always the
conversation of the input that started the turn.

## Where messages go

| Surface | Where a sent message goes |
|---|---|
| Slack | Posted into the thread that started the turn, as the channel's bot, at most once per call. See [Slack](/capabilities/slack/#how-the-agent-replies). |
| Web chat and the API | The session is the conversation. The message appears in the chat and in the event stream. |
| A2A | Returned as the task's reply artifacts. |
| AG-UI | Streamed to the client as text messages. |
| MCP | Returned as what the Agent said in session status. |
| FCP and the channel API | Returned as the reply. |
| CLI chat | Printed as the Agent's reply. |
| Voice | Spoken to the caller. Notes are not spoken. |
| Subagents | Returned to the parent Agent as the subagent's result. |

Evals and observers read sent messages the same way, so a scorer or an observer
sees what the Agent said, not its notes.

## What clients see

Each sent message is recorded as a
[`conversation.message`](/event-reference/#conversationmessage) event with the
message id, its text, and the `send_message` tool call that sent it. When an
external platform such as Slack accepted the message, the event also carries a
`delivery` object with the platform, channel and platform message reference.

The Agent's assistant text is still recorded, as `output.message.completed`
with `message.phase: "commentary"`. Treat it as working notes: the web UI shows
it as part of the Agent's work, and every surface that returns "what the Agent
said" ignores it.

A client that reads replies from events should count `conversation.message`
events and non-commentary `output.message.completed` events. That rule works for
both Direct and Explicit Agents, so a client does not need to know which setting
an Agent uses.

## Limits

- One sent message is at most 12,000 characters. Share a file for longer
  content.
- An empty message is rejected.
- A failed send returns an error to the Agent. If delivery was uncertain, the
  error says so, and the Agent should not resend blindly because the message
  may already be visible.
- If an Explicit Agent ends a turn in a Slack thread without posting anything
  and without calling `no_reply`, Everruns posts one short status line with a
  link to the session. A turn that calls `no_reply` stays silent.
- Sent messages are not streamed token by token. Each one arrives complete.

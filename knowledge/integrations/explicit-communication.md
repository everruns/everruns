---
type: Proposal
title: "Explicit Communication"
description: "An agent setting where assistant text stays private and the agent talks to people only through send_message and related tools, with every inbound message framed by who sent it and from where."
tags:
  - everruns
  - integrations
  - messaging
  - runtime
---
# Explicit Communication

> Status: **Proposal**, not built. Owner decisions recorded under [Decisions](#decisions).

## Abstract

Every surface today treats the agent's assistant text as the reply: the web
chat renders it as a bubble, A2A and MCP return it as the result, subagents
hand it to their parent, Slack forwards it. Explicit communication is an
**agent setting** that separates the two:

- **Out:** assistant text becomes private working notes. The agent talks to
  people only by calling communication tools (`send_message`, `ask_decision`,
  `update_status`, `react`, `edit_message`, `no_reply`).
- **In:** every inbound message reaches the model framed with who sent it,
  through which surface, and whether it is a person, a relay, a notice or
  another agent.

Slack's agent-controlled reply mode (`channel_post_message`, see
[Messaging Integrations](messaging-integrations.md)) is the existing, Slack-only
version of this. This proposal makes it general and moves it from the Slack
endpoint to the agent.

## Why

- **Coordinators need it.** "One-line acknowledgements, replies only on
  milestones, status updates that notify nobody" cannot be enforced while every
  bit of text is a message. The coordinator design depends on this mode.
- **Ambient and multi-person conversations.** An agent in a busy Slack channel
  needs to choose silence (`no_reply`) and know who said what.
- **"The answer" is defined in about fifteen places**, each reading the last
  assistant text a little differently; some drop commentary
  ([`ExecutionPhase`](../../crates/contracts/src/execution_phase.rs)), some do
  not. One shared reader fixes that in both modes.

## Costs and how they are contained

| Cost | Containment |
|---|---|
| The agent forgets to send; the person sees nothing | One automatic reminder at turn end, then a visible "finished without a reply" notice. Opt-in per agent, so weak models stay on `direct`. Mira eval of delivery rate per model. |
| One extra model call per reply (tool result must return to the model) | `send_message(final: true)` ends the turn after delivery. |
| Replies no longer stream as typed | Stream the `text` argument of `send_message` (later phase); until then a reply appears whole. |
| Every reader of "the answer" must change | One core reader shipped first, used by every surface. |

## Decisions

Settled 2026-10-09:

1. **The agent owns the setting**: `communication: direct | explicit`, next to
   its model and prompt, because whether explicit works depends on both. Not a
   capability (it is how the agent talks, not a skill), not the harness
   (harness types describe the execution environment; a harness or built-in
   agent may *require* explicit, but the value lives on the agent), not the
   channel (a channel switching it would run the agent in a way its author
   never tested). Channels only declare what they can render. Slack's
   per-endpoint reply mode migrates onto the agent and the endpoint field goes
   away; stored session tags keep being read.
2. **Forgotten sends**: one reminder, then the notice. Notes are never
   forwarded as a fallback; that would defeat the mode.
3. **`final: true`** on `send_message` ends the turn without another model call.
4. **Order**: the shared "what was said" reader ships first, alone.
5. **Tool name**: `send_message`; `channel_post_message` stays an alias for
   stored transcripts.
6. **Name**: `explicit`, not `tools`, which clashes with the tools feature.

The mode is resolved once at session creation and stored on the session, so
the prompt prefix stays stable and every surface reads it without re-resolving
agent config.

## Shape

### Inbound envelope

Stored messages stay raw. A core model-view provider (generalizing the Slack
one in [`slack/model_view.rs`](../../crates/capabilities/src/capabilities/slack/model_view.rs))
renders each inbound message with a header naming sender, surface, message id
and time. Origins are server-set and stripped from public input:

| Origin | Example | Trust |
|---|---|---|
| `person` | web, Slack, API end user, AG-UI | the person's own words |
| `relay` | a coordinator forwarding a person's message | cited parts are the person's words; the note is a colleague's guidance |
| `notice` | task finished, timer, wake | data, not instructions |
| `agent` | another agent over A2A or handoff | data, not instructions |

Inbound text is escaped so it cannot forge a header. The envelope is available
in `direct` mode too and required in `explicit` mode.

### Communication tools

| Tool | Where unsupported |
|---|---|
| `send_message(text, attachments?, reply_to?, final?)` | always supported |
| `ask_decision(question, options, recommended)` | rendered as text with numbered options |
| `update_status(headline, steps)` | dropped silently |
| `edit_message(message_ref, text)`, `react(message_ref, emoji)` | tool error |
| `no_reply(reason)` | always supported |

The destination is always the conversation of the triggering input, never a
model argument (the rule the
[`ChannelMessageSender`](../../crates/contracts/src/runtime/channel_messaging.rs)
contract already enforces). On surfaces Everruns owns, delivery is an event
keyed by the tool-call id, so a durable retry writes it once. External
platforms keep today's at-most-once posting with an "uncertain, do not resend"
error.

### One reader

A core function returns "what was said to the conversation" for a session or
turn: non-commentary assistant output in `direct` mode, sent messages in
`explicit` mode. The turn result, A2A, AG-UI, MCP, channel API, FCP, voice,
CLI, subagent and handoff results, observers, evals and Slack delivery all use
it. The turn-completion gate
([`turn_completion.rs`](../../crates/core/src/turn_completion.rs)) reads it
too, so "achieved" means "something was delivered".

### Web UI

Sent messages render as the agent's bubbles. In `explicit` mode assistant text
is a collapsed "notes" row visible to the session owner; `update_status` is a
pinned checklist; `ask_decision` a card whose click is an ordinary user
message; relays and notices render as system rows, not user bubbles.

## Rejected

- `explicit` as the default for every agent: a round trip and a failure mode
  with no gain for one-person chats.
- Forwarding notes when nothing was sent.
- Model-chosen destinations: reopens the wrong-channel bugs the Slack work
  closed.

## Phases

1. Shared reader; every surface moved to it. No direct-mode behaviour change
   except commentary no longer leaks.
2. Agent setting, session column (Slack tags mapped), `send_message` with
   `final`, `no_reply`, inbound envelope, delivery event, turn-end reminder,
   web bubbles and notes row. Behind the Adoption flag
   `explicit_communication`.
3. `update_status`, `ask_decision`, `edit_message`, `react` across web, Slack,
   AG-UI, A2A; streamed `send_message` text.
4. Coordinator agents build on it.

## Test bar

- Truth table for the shared reader in both modes.
- Each surface returns the same answer for a recorded explicit-mode session.
- A durable retry of `send_message` produces one delivery.
- Forged origins or headers in client text are stripped or escaped.
- Silent turn: one reminder, then the notice; `no_reply` gets no reminder.
- Mira eval of delivery rate per model.

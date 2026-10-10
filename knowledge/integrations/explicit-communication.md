---
type: Specification
title: "Explicit Communication"
description: "An agent setting where assistant text stays private and the agent talks to people only through send_message and no_reply, read on every surface through one shared reader of what the agent said."
tags:
  - everruns
  - integrations
  - messaging
  - runtime
---
# Explicit Communication

> Status: **Partly built.** Phase 1 and the core of phase 2 shipped; see
> [What shipped](#what-shipped). Owner decisions recorded under
> [Decisions](#decisions). Public guide: `docs/features/explicit-communication.md`.

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
[Messaging Integrations](messaging-integrations.md)) was the Slack-only
version of this. Explicit communication made it general and moved it from the
Slack endpoint to the agent; the endpoint's `reply_mode` is gone.

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

The plan was to resolve the mode once at session creation and store it on the
session. As built, it is read from the agent's resolved execution snapshot
instead (see [Deviations](#deviations-from-the-plan)).

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
[`ConversationSender`](../../crates/contracts/src/runtime/conversation/tools.rs)
contract enforces). On surfaces Everruns owns, delivery is an event
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

## What shipped

Source of truth: [`conversation.rs`](../../crates/contracts/src/runtime/conversation.rs)
(setting and shared reader), [`conversation/tools.rs`](../../crates/contracts/src/runtime/conversation/tools.rs)
(tools, prompt section, sender contract) and `ConversationMessageData` in
[`message_data.rs`](../../crates/contracts/src/runtime/events/message_data.rs).

- **Phase 1, shipped.** One shared reader (`said_in_event`,
  `said_in_transcript`, `final_reply`): commentary never counts, sent messages
  always do. The turn result, A2A, AG-UI, MCP, channel API, FCP, CLI chat,
  subagent results, evals, observers and voice use it. Stored
  `channel_post_message` calls still read as sent messages.
- **Phase 2, partly shipped.** The agent setting (API, agent package manifest,
  Framework builder, web UI select); `send_message(text)` and
  `no_reply(reason?)`; the "How you talk" prompt section; the
  `conversation.message` event, with `delivery` only for external platforms;
  Slack delivery as the endpoint bot into the triggering thread; the Framework
  `SessionEventKind::MessageSent`. Migration
  [`203_agent_communication.sql`](../../crates/server/migrations/203_agent_communication.sql)
  moved tool-only Slack endpoints onto their agents as `explicit`, and the
  endpoint `reply_mode` and the automatic "On it." acknowledgement are gone.
- **Not built yet.** The turn-end reminder when nothing was sent,
  `send_message(final: true)`, the inbound sender envelope, and all of phase 3
  (`update_status`, `ask_decision`, `edit_message`, `react`, streamed
  `send_message` text). Until the reminder lands, a Slack turn that ends
  without a post or a `no_reply` gets only the "finished without a reply"
  notice; `no_reply` keeps the thread silent.

### Deviations from the plan

- **The mode is not stored on the session.** It is read from the agent's
  resolved execution snapshot, like the model and prompt it depends on, so
  there is no session column and no copy of agent config to keep in sync.
  Changing an agent's setting affects its later turns.
- **No `explicit_communication` Adoption flag.** The setting replaces an
  existing, unflagged Slack setting (`reply_mode: tool_only`), and the
  migration moves those endpoints onto it. Hiding it behind a flag would have
  taken a working feature away from them.

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

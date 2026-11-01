---
type: Specification
title: "Message reactions"
description: "Emoji reactions on user and agent messages, from people and from the agent; how a person's reaction reaches the agent as a short note, and how API, AG-UI, A2A and Slack callers set and read them."
tags:
  - everruns
  - execution
---
# Message reactions

Status: approved 2026-11-01 (design and mockups reviewed in the project thread).

## Goal

Anyone in a conversation, a person or the agent, can put an emoji on any user
or agent message. A person's reaction to the agent's reply reaches the agent as
a short note, never as a typed message and never as one model call per emoji.
Callers from outside (Agent Execution API, AG-UI, A2A, Slack) can add, remove,
read and receive reactions. Chat, Work Chat and the session trace show them.

## Prior art

| System | Model | Reaches the bot or agent |
|---|---|---|
| Slack | one reaction per (user, message, emoji) | `reaction_added` / `reaction_removed` events |
| Discord | same key, plus super reactions | `MESSAGE_REACTION_ADD` / `REMOVE` gateway events |
| WhatsApp Cloud API | a reaction is a `reaction` message; empty emoji removes | as an inbound message |
| Telegram Bot API | `setMessageReaction` | `message_reaction` updates |
| Matrix | `m.reaction` annotation event | timeline event; removal is a redaction |
| ChatGPT, Claude apps | thumbs up/down with a comment | no, provider quality pipeline |
| AG-UI 1.0 | none; draft "Meta Events" (`META`, `metaType`) | no client-to-server path defined |
| A2A 1.0 | none | Message metadata and extension URIs are the hook |

The shared model: (message, actor, emoji) is unique, changes are add and remove
events, clients show a per-emoji summary with a "me" flag. Ratings are a
separate, private concept everywhere; Everruns keeps Good/Bad message feedback
(`knowledge/ui/chat-experience.md`) separate, private and never sent to the agent.

## Record

Reactions are session events, `message.reaction.added` and
`message.reaction.removed` (`crates/contracts/src/runtime/events/reaction_data.rs`):
message id, emoji, actor (`user`, `consumer`, `agent`, `external`; id; display
name at the time) and source (`console`, `api`, `work_chat`, `agent`, `slack`,
...). The log is the record rather than a table because the agent reacts from
inside a turn on a worker, and the event log is what the worker and the API
already share. Removing appends; nothing is deleted. A partial index
(`idx_events_reactions`) keeps the per-send lookup to one index probe.

- One reaction per (message, actor, emoji). Adding twice or removing what is
  not there succeeds and writes nothing.
- An emoji is one Unicode emoji (ZWJ sequences, skin tones, flags, keycaps) or
  a platform custom emoji `:name:`. Text is refused, so a reaction can never
  carry a sentence into what the agent reads.
- At most 20 distinct emoji per message.
- Reads fold the log: `GET /v1/sessions/{id}/reactions` returns, per message,
  each emoji with its count, whether the caller (`me`) and the agent (`agent`)
  reacted, and who, earliest first. Agent reactions are named after the
  session's agent at read time.

## Reaching the agent

Only reactions on the agent's own replies are told, and never the agent's own.

- **The note.** The model reads a block labeled
  `[Reactions, not typed messages]`, one line per reacted message:
  `Mykhailo reacted 👍 and Alice reacted 🎉 on your reply "Deployed 0.50.1…" (message_…)`.
  A reaction taken back after it was told reads `took back`. The note is
  stored as reserved message metadata (`everruns_reactions`,
  `crates/contracts/src/message_reactions.rs`) and rendered at model-view
  time, so the bytes are stable across replays and the prompt cache holds.
  Clients cannot set the key.
- **Told once.** Each note records `through_sequence`, the last reaction event
  it covers. The next note starts after it. An add and a remove between two
  notes cancel out.
- **When.** The agent's Reactions capability setting "When someone reacts to
  the agent" (`crates/capabilities/src/capabilities/reactions.rs`):
  - *Tell it at the next turn* (default, also without the capability): the
    note rides on the next input message, whoever writes it.
  - *Start a turn*: an idle session gets a reaction message right away.
  - *Don't tell it*: stored and shown only.
  A reaction made while a turn runs is told right away in every mode but
  *Don't tell it*: a reaction message joins the running turn at its next
  step (the existing steer path).
- **Reaction message.** A user-role message with origin kind `reaction`
  (`knowledge/execution/message-origin.md`) whose text is the note. Its client
  message id is derived from `through_sequence`, so two reactions racing to
  tell the same note store it once. Chat and Work Chat do not show it as a
  row; the chips are the record. The trace shows it.

## The agent reacts

The Reactions capability adds `react(emoji, message_id?, remove?)`. With no
`message_id` it reacts to the message the agent is answering (the 👀 "looking"
and ✅ "done" acknowledgement). Message ids appear in notes. The tool appends
the same event a person's reaction does. No feature flag: adding the
capability is the opt-in. Slack's `slack_add_reaction` stays for Slack-native
targets until the Slack mapping below lands.

## Surfaces

- **`/v1`:** `PUT` / `DELETE /v1/sessions/{id}/messages/{message_id}/reactions/{emoji}`
  (SESSION_MANAGE, signed-in user), `GET /v1/sessions/{id}/reactions`
  (SESSION_VIEW). CLI: `everruns sessions messages react|unreact|reactions`.
- **Agent Execution API:** the same three routes under the agent base URL,
  in the caller's own sessions. An end user reacts as that consumer; an agent
  key acting as itself reacts as "Application". The reaction events stream
  over SSE at every visibility, with the actor reduced to kind and name.
- **Chat, Work Chat, trace:** chips under messages, a reaction button in the
  reply action row, delivery status per reaction in the trace. Design and
  mockups: the reactions thread (2026-11-01).
- **AG-UI:** outbound `CUSTOM` event `everruns.reaction`; switch to `META`
  with `metaType: "reaction"` once the Meta Events draft is in `@ag-ui/core`.
  AG-UI clients add reactions through the Execution API route.
- **Slack:** inbound `reaction_added` / `reaction_removed` on mapped messages
  become reactions (`reactions:read`); `react` in a Slack session maps to
  `reactions.add` / `reactions.remove`.
- **A2A:** extension `https://everruns.com/ext/a2a/reactions/v1`, a DataPart
  `{kind: "reaction", messageId, emoji, action}` stored as a reaction, not a
  turn.

## Delivery

1. Server: events, emoji rules, note, `react`, `/v1` and Execution API routes,
   CLI, delivery to the agent.
2. UI: Chat, Work Chat, trace, capability setting.
3. AG-UI event, Slack inbound and `react` mapping, SDK methods.
4. A2A extension.

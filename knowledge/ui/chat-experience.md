---
type: Decision
title: "Chat Experience"
description: "Why chat renders as a calm, centered reading column with one turn status row that mounts on Enter, and the client and server contracts that let that row tell the truth from send to answer."
tags:
  - everruns
  - ui
  - chat
  - turns
  - events
---

# Chat Experience

Status: approved 2026-10-10; building per the delivery plan.

## Abstract

Chat gets two coordinated upgrades, taken from two design handoffs attached to the project
thread ("Chat design with Codex approach" and "Turn start feedback"):

1. **Look.** The transcript becomes a centered reading column. Agent replies are plain prose
   with no card, border or avatar; user turns are a quiet tinted block; tool work sits in the
   existing folded work log above each reply; messages carry a small action bar.
2. **Turn state.** One status row mounts under a user turn in the same frame as Enter and
   stays mounted until the turn ends. It is the work log in its earliest state, so the user
   sees continuous progress from Enter to the first tool call to the answer, plus explicit
   Queued, Stopped and Not delivered states.

The handoffs own look and motion. This concept owns the state model, the data each state is
derived from, the server changes it needs, and the delivery plan. It builds on the folded work
log (`apps/ui/src/components/chat/turn-work-log.tsx`) and does not change the Session Trace
view ([session-trace.md](session-trace.md)) or the coordinator thread board
([coordination](../runtime-resources/coordination.md)).

## Problem

Measured against current `main`:

- **Silence after Enter.** The composer keeps the text and shows nothing new until
  `POST /messages` returns (`apps/ui/src/components/chat/chat-panel.tsx`, `submitMessage`
  awaits the mutation before clearing). The optimistic user message appears, but nothing
  under it says the turn exists until the first `reason.started`/thinking or work-log event
  arrives. With model time-to-first-token of 0.8 to 1.3 s on top of the send path, the user
  routinely stares at an unchanged screen for one to two seconds, longer on a slow network.
- **No failure path for the send itself.** A failed POST removes the optimistic message and
  shows an alert below the transcript; the text the user wrote is only safe because the
  composer was never cleared. There is no retry on the message and no "still connecting".
- **Optimistic messages are matched by text.** `session-context.tsx` drops an optimistic
  message when any real `input.message` with the same text exists, so sending "yes" twice
  hides the second one until the server echoes it, and the matching is O(n) per render.
- **A message sent mid-turn is invisible as such.** The server steers it into the running
  turn (it joins at the next act/reason boundary, or starts a follow-up turn if it lands after
  the final answer). The UI renders it as an ordinary user message with no state, so the user
  cannot tell whether it was picked up.
- **Stop is turn-only.** It appears only once the session status is `active`, cannot cancel a
  send in flight, and the cancel events carry a synthetic turn id
  (`crates/server/src/domains/sessions/commands/mod.rs`, cancel handler), so the UI cannot
  attribute "Stopped after Ns" to the right turn. A cancel that lands between the message
  commit and the post-commit turn start is lost.
- **Visual weight.** Every agent reply is a bordered card with an avatar tile, full width of
  the pane; long transcripts read as a stack of boxes. User turns are 13 px and easy to miss.
  There are no day separators and no per-message actions beyond an info icon.

## Decisions

### Look

- **Centered column, 820 px max.** Transcript and composer share one column
  (`max-width: 820px`, 24 px side padding), so line length stays readable on wide screens.
  14 px body, 1.6 line height.
- **Agent replies are prose.** No card, no left border, no avatar tile by default. The work
  log row above a reply already says who worked; the reply is the answer. The handoff's
  optional agent mark stays available as a surface prop for multi-agent sessions, where the
  speaker must be named.
- **User turns are a tinted block.** Right aligned, `max-width: min(78%, 42rem)`, accent wash
  (`hsl(var(--accent) / 0.12)`) with a right accent rule, 14 px. "Gold wash" is the default
  tone of the three the handoff offered.
- **Day separators.** A centered muted timestamp ("Wednesday 7:53 PM", "Today 7:40 PM")
  before a user turn when the gap to the previous turn is over one hour or crosses midnight.
- **Work log steps read as a checklist.** Opened, a turn lists one row per step: status icon
  (running dot, check, error), the narrated label, a right-aligned one-line preview of the
  result, and a Details control that opens the existing tool detail. An approval the turn
  recorded renders inline as "Approval recorded", expanded, with the approved request, the
  approver and a consent link (the data `approvalContexts` already derives).
- **Message actions.** Under the latest agent reply, and on hover for earlier ones: Copy,
  Good response, Bad response, Branch into new chat, Message details (today's info tooltip).
- **Composer as a card.** Textarea in a bordered card; below it one toolbar row: attach,
  model and effort chip, then stop or send on the right. Send and Stop share one slot (see
  state rules).

### Turn state

One component, the **turn status row**, renders under every user turn that has not been
answered. It is the same element as the folded work log header and never remounts: only
its label, status text, lead glyph and sheen change. States:

| State | Label · status | Entered when |
|---|---|---|
| Sending | "Sending…" | Enter, same frame, before any network |
| Still connecting | "Still connecting…" | no server ack after 3 s |
| Not delivered | row removed; "Not delivered · Retry" under the message | POST failed, or no ack after 10 s |
| Queued | pulsing dot, "Queued · Joins the current turn" | acked while another turn of this session runs |
| Starting | "Working for 0s · Starting" | acked, and the turn for this message has begun |
| Thinking | "Working for Ns · Thinking", or the latest reasoning line | `reason.started` for that turn, no step yet |
| Working | "Working for Ns · {latest step}" | first work-log step; chevron turns solid |
| Done | "Worked for Ns" (+ error count) | turn completed; the reply renders below |
| Stopped | "Stopped after Ns" | user stopped the turn |

Rules carried over from the handoff: the row exists from Enter to completion; the sheen runs
while the turn is active; stop is available from 0 ms (before ack it aborts the request and
returns the text to the composer; after ack it cancels the turn); reduced motion turns off
sheen and shimmer but keeps text changes. The timer counts from the server ack, not from
Enter, so it measures the agent and not the network.

**Queued wording.** The handoff says "Starts after the current turn". Everruns does not queue
a second turn: a mid-turn message is steered into the running one. The row therefore says
"Joins the current turn", and when that turn ends without a turn of its own for the message
the row folds into the running turn's log. If the server starts a follow-up turn for it
instead, the row moves straight to Starting.

### Client state model

- **Pending sends are client state, keyed by a client message id.** On Enter the UI mints a
  `client_message_id` (UUID v7), appends a pending entry
  `{clientId, content, attachments, phase, sentAt, ackAt?, messageId?, error?}`, clears the
  composer and fires the POST with an `AbortController`. The pending entry, not a fake event,
  renders the optimistic turn.
- **Reconcile by id, never by text.** The POST response carries the stored message id, and
  the stored `input.message` echoes `client_message_id`; whichever arrives first marks the
  entry acked and hands rendering over to the real event. Identical texts no longer collide.
- **Phases come from events the UI already receives**, joined by `input_message_id`:
  `turn.started` and `session.activated` carry it, and every later event of that turn carries
  the turn id they mint (`crates/contracts/src/runtime/events/turn_data.rs`). A pure reducer
  (`turnPhaseFor(message, events, now)`) maps a user message plus the event stream to the
  table above, and is unit tested state by state.
- **Retry resends the same `client_message_id`.** A send that reached the server but lost its
  response must not create a second message (see server changes).
- **Stop before ack** aborts the request and restores the draft when the send never reached
  the server; when the ack arrives anyway, the UI cancels the turn as soon as it starts.

### Server changes

All additive; no migration.

1. **`client_message_id` on `POST /v1/sessions/{id}/messages`.** Optional UUID. The server
   stores it in reserved message metadata (`everruns_client_message_id`, stripped from client
   metadata like other reserved keys) so the `input.message` event echoes it, and treats it as
   an idempotency key: a repeat within the session returns the stored message instead of
   creating a new turn. No new table: the lookup uses the same `data @>` containment the Slack
   `slack_ts` dedup uses, over the session's newest input messages.
2. **Delivery in the response.** The `Message` response gains `delivery`:
   `"started"` (new turn), `"steered"` (joined a running turn), `"resumed"` (resolved a
   parked turn) or `"duplicate"` (a retry of a stored send). The server already knows this at
   reservation time (`reserve_active_turn_slot_for_org` returns the previous status); the UI
   uses it to enter Queued without guessing.
3. **Steered pickup needs no new event.** The durable driver reads steered messages at the
   next act-to-reason boundary, and the next `reason.started` of the running turn re-reads the
   full history. The UI treats the first `reason.started` of the running turn after a queued
   message's sequence as its pickup; a follow-up turn announces itself with `turn.started`
   carrying the message's id.
4. **Cancel names the real turn.** The cancel handler emits `turn.cancelled` with the open
   turn's id and input message id (its newest `turn.started` without a completion) instead of
   the synthetic ids it uses today, so "Stopped after Ns" lands on the right turn. Stop
   pressed while a send is in flight is held by the UI until that message's turn has started
   (or the send is known steered), which avoids racing the post-commit turn start.
5. **Message feedback.** Good/Bad response is stored in a `message_feedback` table, one row
   per person and message (`set_message_feedback` and `list_message_feedback` in
   `crates/server/src/domains/messages/feedback.rs`). Pressing the active rating again
   clears it. The endpoint is an idempotent `PUT`, not the `POST` first sketched, because
   rating again replaces the row. Only the caller's own ratings are listed; Session Trace
   reads all of them later.
6. **Branch from a message.** `POST /v1/sessions/{id}/fork` takes an optional
   `up_to_message_id` (a user or agent message); the fork copies history up to and including
   the end of that message's turn. Workspace files and session storage are copied as they are
   now, since they keep no history to cut. Branch is disabled while a turn runs, because a
   mid-turn fork is refused. Playground chats show neither feedback nor Branch: they cannot
   be forked.

## Rejected options

- **A separate "sending" spinner or toast.** Two indicators for one turn; the handoff's point
  is that the work log row is the only signal from Enter on.
- **Matching optimistic messages by text with a time window.** Still wrong for repeated short
  replies ("yes", "ok") and for edits; an id costs one field.
- **A real per-session turn queue.** It would change runtime semantics that the durable
  engine, framework and channels share. Steering is the intended behavior; the UI should show
  it honestly instead.
- **Inferring Queued from session status on the client.** The status can flip between the
  POST and the next SSE frame; the server knows the answer at reservation time.
- **Keeping the agent card.** It was the main source of visual noise in long transcripts.

## Delivery plan

Each step is one PR, merged green:

1. This concept.
2. Server: `client_message_id` idempotency, `delivery` in the response, real ids on cancel.
   OpenAPI and UI types regenerated.
3. UI turn state: pending-send store keyed by client id, `turnPhaseFor` reducer with tests,
   turn status row from Enter, Still connecting / Not delivered / Retry, stop before ack,
   Queued, Stopped after. `/dev/work-log` gains the six states for review.
4. UI look: centered column, prose replies, user block, day separators, checklist steps with
   previews and Details, inline approval step, composer card, Copy and Details actions.
5. Feedback and branch-from-message, server and UI together.

Steps 3 and 4 touch the same files and land in that order. Each UI step is verified with
before/after screenshots on a local stack.

## Success bar

- The status row is in the DOM in the frame after Enter (asserted in a component test with
  fake timers), and never unmounts until the reply renders.
- Sending the same text twice shows two turns immediately.
- With the network blocked, the message shows "Still connecting…" at 3 s and "Not delivered ·
  Retry" by 10 s; Retry after the server stored the first attempt creates no duplicate.
- A message sent mid-turn shows Queued, then joins the running turn's log when the turn's next
  reason step starts.
- Stop during Sending restores the draft; Stop during a turn shows "Stopped after Ns" on that
  turn.

---
type: Specification
title: "Message origin"
description: "One platform-only marker on every user-role message no person wrote (schedule runs, triggers, thread wake-ups, notices), read the same way by Chat, Work Chat, the session trace and the model."
tags:
  - everruns
  - execution
---
# Message origin

Status: approved 2026-11-01.

Several things write user-role messages that no person typed: a session
schedule firing, an agent trigger, a channel schedule or webhook, a thread
wake-up for a coordinator, a cancellation notice. Each used to mark itself its
own way (`source: "schedule"`, `source: "agent_trigger"`,
`everruns_origin: "task_wake"`, or nothing), and most surfaces showed them as
if the person had written them. The session trace labeled every turn "user";
a schedule polling every five minutes read as the person asking the same
thing 288 times.

## Contract

One reserved metadata key, `everruns_origin`, names who wrote the message.
The shape and the reader live in
[`message_origin.rs`](../../crates/contracts/src/runtime/message_origin.rs).

- **No origin means a person wrote it.** That is the rule every surface uses.
- **Platform-only.** `strip_reserved_message_metadata` drops the key from
  client metadata, so a client cannot dress its text up as a platform notice.
- **Authorship, not authority.** The event's `initiator`
  ([`execution_metadata.rs`](../../crates/server/src/execution_metadata.rs))
  says whose authority a turn runs under, for approvals, payments and audit.
  Origin says who wrote the words. They stay separate.
- **Name at write time.** `name` is recorded when the message is written, so
  renaming or deleting the schedule never rewrites history. A session schedule
  has no name of its own, so the first sentence of its prompt names it; a
  trigger is named by its kind ("GitHub", "Schedule"); a channel schedule or
  webhook by its agent.
- **Run counter.** `run` carries the schedule limits' counter, end time and
  stop condition. The limits write `schedule_run` next to the message; the
  reader folds it into `run`.
- **No backfill.** Older rows keep their markers. The reader maps the
  `"task_wake"` string and the `source` values `schedule`, `agent_trigger`,
  `app_schedule` and `app_webhook` to the same shape, and the public message
  projection ([`public_metadata.rs`](../../crates/contracts/src/public_metadata.rs))
  emits the normalized object, so clients read one shape.
- A2A and approval or answer messages carry no origin: a client or the person
  acted, the platform only relayed it.

## Surfaces

- **Model.** Each origin message gets one header line before its text, built
  only from the stored origin ("[Scheduled run 12 of 96: "Shipmail claim
  poll". Stop when: … Not written by the person.]"), so replays render the same
  bytes and the prompt cache holds. The thread wake-up keeps its earlier label
  word for word. See `label_speaker` in
  [`transcript.rs`](../../crates/core/src/engine/execution/reason/transcript.rs).
- **Chat.** The gold wash means a person wrote it. Every origin message is a
  muted `AutomaticUpdate` row: kind icon, label, name, run counter, prompt
  folded. Icons: calendar-clock for schedules, zap for triggers, webhook for
  webhooks, refresh for thread updates, cog for notices.
- **Work Chat.** The consumer view shows the kind and name only, never the
  prompt or ids. Origin messages from one source with nothing between them
  collapse into one line with a count, so twelve silent polls are one line.
- **Session trace.** `turn.started` carries `origin`, and the trace turn row
  stores it. The turn header names who started it (icon, kind, name, run
  counter, id) instead of "user". Automatic turns get a gray muted surface with
  a gray left edge; every turn stays a turn, nothing is folded (user decision,
  2026-11-01). Turns projected before origins existed show "user".
- **Trace exports.** `turn.started` carries the origin, so exporters can label
  automatic turns from the event without a lookup.

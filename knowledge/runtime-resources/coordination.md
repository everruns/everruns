---
type: Specification
title: "Coordination"
description: "Coordinator agents hand work to threads and keep the person's view of it."
tags:
  - everruns
  - runtime-resources
---
# Coordination

Status: implemented at Adoption grade. Platform Chat coordinates under the
`chat_threads` flag; custom agents coordinate under `agent_coordination`
(`is_agent_capability_enabled` in `crates/server/src/records/feature_flags.rs`).

A **coordinator** is an agent session that does not do focused work itself. It
starts **threads** (child sessions) for that work, relays the person's follow-ups
to them, and reports back. The model follows a Claude Projects project: the
person talks to one place, work happens in threads they can open, and a board
shows what is working, what needs them, and what is ready for review.

The `coordination` capability
([`coordination.rs`](../../crates/capabilities/src/capabilities/coordination.rs))
owns the tools, limits and executor. Its header comment carries the detailed
decisions; this concept owns the contract.

## Model

- A **thread** is a session whose `parent_session_id` is the coordinator. It
  keeps the `subagent` source, so no new session kind exists.
- An **assignment** is one unit of work: a [session task](session-tasks.md) of
  kind `assignment`, owned by the coordinator, linked to the thread through
  `links.child_session_id`. A thread is long-lived and carries one assignment
  per unit of work over its life; a follow-up after completion opens a new one.
- The thread's worker is the coordinator's own agent (`self`) or any agent the
  person may run, limited by the capability's `workers` list.
- Threads do not nest and do not message each other. Every route goes through
  the coordinator, so routing has one owner and one record.

## Lifecycle and board

The board groups threads by their latest assignment:

| Group | When |
|---|---|
| Needs you | assignment awaits input or failed |
| Working | assignment queued or running |
| Ready for review | assignment succeeded, thread not resolved |
| Open | assignment cancelled |
| Resolved | coordinator resolved the thread (`state_detail = "resolved"`) |

A thread keeps the title of its first assignment; a follow-up assignment's
title leads the row's preview instead of renaming the thread.

The worker reports through tools injected into any session with an open
assignment: a checklist (`progress.steps` on the task), completion with a
summary and artifacts, a decision ask, a report, and a redirect for work that
belongs to another thread. A thread turn that ends without completing flags the
assignment as needing attention, so the coordinator always hears back
([`services/coordination/mod.rs`](../../crates/server/src/services/coordination/mod.rs)).

## Wake-ups and provenance

Task state changes wake the coordinator with an injected user-role message that
carries `everruns_origin = task_wake` metadata. The model sees it prefixed as an
automatic update, the UI labels it, and client-supplied metadata cannot forge it
(`strip_reserved_message_metadata` in `crates/contracts/src/runtime/message.rs`).

Wake texts and the briefs a thread receives stay written for the model (task
ids, the worker's instructions). The UI shows a readable version and falls back
to the raw text when a message does not match
([`chat-thread-messages.ts`](../../apps/ui/src/lib/chat-thread-messages.ts)).

## Framework

The `everruns` facade runs the same capability on a local profile
([`coordination.rs`](../../crates/everruns/src/coordination.rs)). Threads are
sessions of the engine that owns the coordinator and run its agent; another
worker agent is refused, since a local profile has no agent catalog. The
settlement rules for a thread turn live with the capability
(`settle_thread_turn`), so the server listener and the facade apply the same
ones. The facade settles the turns it starts on a thread; a turn the
application starts on a thread directly is not settled. Automatic updates go
through an observer on the profile's task registry and cover `assignment`
tasks only. [`examples/coordinator-agent`](../../examples/coordinator-agent)
is the runnable proof.

## Invocation identity

A wake-up is a platform-injected message, so it needs its own runtime
invocation record or the worker refuses it. It continues the session's latest
invocation (same person and responder), never an identity taken from the task
(`record_continued_runtime_invocation`).

## Reply channel

Relayed text is framed in one place (`frame_brief` / `frame_relay`), so a later
tool-based reply channel replaces those helpers rather than the routing.

## Not yet

Reassigning, transferring or merging threads; verbatim cited relays;
`thread.routed` events; a Move action on the board.

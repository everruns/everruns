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

The worker reports through tools injected into any session with an open
assignment: a checklist (`progress.steps` on the task), completion with a
summary and artifacts, a decision ask, a report, and a redirect for work that
belongs to another thread. A thread turn that ends without completing flags the
assignment as needing attention, so the coordinator always hears back
([`services/coordination.rs`](../../crates/server/src/services/coordination.rs)).

## Wake-ups and provenance

Task state changes wake the coordinator with an injected user-role message that
carries `everruns_origin = task_wake` metadata. The model sees it prefixed as an
automatic update, the UI labels it, and client-supplied metadata cannot forge it
(`strip_reserved_message_metadata` in `crates/contracts/src/runtime/message.rs`).

## Reply channel

Relayed text is framed in one place (`frame_brief` / `frame_relay`), so a later
tool-based reply channel replaces those helpers rather than the routing.

## Not yet

Reassigning, transferring or merging threads; verbatim cited relays;
`thread.routed` events; a Move action on the board.

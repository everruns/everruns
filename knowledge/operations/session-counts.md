---
type: Specification
title: "Session Counts"
description: "Denormalized session counters and the reads they exist to keep cheap."
tags:
  - everruns
  - operations
  - sessions
---
# Session Counts

Several surfaces want to say how much is behind a session — how many turns it took, how many
tools it called, how many events it recorded, how much work it owns, how many files it left.
None of those questions may be answered by counting rows at read time. Event histories are
large enough that bounded reads were their own piece of work, and the sessions list has been
through two rounds of query surgery; a `COUNT(*)` over `events` on a page load undoes both.

So every such count is a **denormalized counter maintained by a statement-level trigger**, read
as an O(1) column on a row the caller already fetches.

## The counters

| Counter | Table | Counts | Added by |
| -- | -- | -- | -- |
| `turn_count` | `sessions` | `turn.completed`, `turn.failed`, `turn.cancelled` events | migration 102 |
| `tool_call_count` | `sessions` | `tool.completed` events | migration 102 |
| `event_count` | `sessions` | every event in the session | migration 125 |
| `task_count` | `sessions` | `session_tasks` rows the session owns | migration 125 |
| `file_count` | `workspaces` | non-directory `workspace_files` rows | migration 125 |

`turn_count` and `tool_call_count` exist for the `fact_session` reporting projection, which
would otherwise rescan the full event log on every snapshot upsert. `event_count`, `task_count`
and `file_count` exist for the session detail tab badges.

## Why the file counter lives on `workspaces`

Files were rekeyed from the session to the workspace in migration 056, and a workspace can back
more than one session (multi-head workspaces). A per-session file counter would either
double-count or drift the moment two sessions shared a workspace, so the counter lives where the
rows do and the session-detail read reaches it through a primary-key join.

## Contract

* **Statement-level, not row-level.** Transition tables (`REFERENCING NEW TABLE` / `OLD TABLE`)
  turn a batch insert of a long turn's events into one extra `UPDATE`, not one per row.
* **Counters never go negative.** Deletes clamp at zero rather than trusting the arithmetic,
  because a bulk purge and a backfill can otherwise race into a negative badge.
* **`workspace_files` also has an `UPDATE` trigger.** `is_directory` is settable, so a row can
  move between "counted" and "not counted" without being inserted or deleted; without the update
  trigger that drift would be permanent.
* **Zero is reported as absent.** The API omits a count of zero rather than serializing `0`, so
  a consumer cannot tell "empty" from "not projected" — and does not need to, because both mean
  there is nothing to show.
* **Projections that do not select a counter decode it as zero.** Only the session-detail read
  projects all three; list and stats queries leave what they do not need unselected.

## What they are not

They are not a live subscription. A session page reads them when it reads the session, so a
running session's badges advance when the session query refreshes, not per streamed event.

They are also not authoritative for correctness — nothing branches on them. A counter that drifts
makes a badge wrong, never a decision wrong. The backfill in each migration is the repair
procedure if that ever happens.

## Related

* [Migrations Specification](migrations.md) — how a new counter ships.
* [Information Architecture](../ui/information-architecture.md) — what the session tab badges
  promise a reader.

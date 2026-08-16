---
type: Specification
title: "Forking Sessions"
description: "Fork a session into an independent copy (history + workspace + state)."
tags:
  - everruns
  - runtime-resources
---
# Forking Sessions

Fork an existing session into a new, independent session that starts with a
copy of the parent's conversation history and durable session-scoped data.
A fork is the "branch from here" / "rewind and try a different path" primitive:
the parent is untouched, and the child evolves independently from the fork
point.

## Motivation

Users want to explore alternatives without losing the work that led up to a
decision point:

- **Branch an exploration.** Keep a long working session as the trunk and fork
  it to try a risky refactor, a different prompt, or a different model, without
  polluting the original thread.
- **Rewind and retry.** A turn went sideways; fork the session as it was a few
  turns ago and continue from there with a corrected message.
- **Share a starting point.** Fork a curated session (history + workspace
  files + key/value state) as a template for a teammate or a follow-up task.

Today the only parent/child relationship between sessions is **subagent
nesting** (`parent_session_id`), which is execution-scoped and created by the
worker. Forking is a distinct, user-initiated relationship and is modeled
separately (see [Lineage](#lineage)).

## Scope and defaults (V1)

These are the V1 decisions. Each is revisitable; the rationale is recorded so a
later change is a deliberate one.

| Decision | V1 choice | Rationale |
|----------|-----------|-----------|
| Fork point | Full history (everything up to "now") | Covers the common "branch from here" case; arbitrary-point rewind is a clean follow-up (see [Fork point](#fork-point)). |
| Workspace/files | **Isolated copy**: new workspace, deep-copy files | True fork semantics: edits in the child never affect the parent. |
| Conversation | Copy all persisted events as-is | Faithful snapshot; reconstruction logic is unchanged. |
| Compaction checkpoint | Copy the latest checkpoint within the copied event range | The fork keeps the canonical model view without weakening raw-history fidelity. |
| KV + secrets | Copy (target design; see status note) | Config/state the child is expected to reuse. |
| SQL databases | Copy (page-level; target design) | Pure data, transparent to the agent. |
| Leased resources, sandbox, voice, tasks, schedules | **Do not copy** | Execution-bound or externally-billed; the child re-leases on demand. |
| Parent state required to fork | Not `active` / `waiting_for_tool_results` | Avoids snapshotting a half-written, in-flight turn. |

## What gets copied

Session-scoped state falls into three buckets: **copy**, **skip**, and
**re-derive**. Detached session spawning reuses this lineage/copy machinery:
`seed = "fork"` uses the same copy set as a fork, `seed = "workspace"` copies
workspace files only, and `seed = "fresh"` records lineage without copying
state.

### Copy

- **Session configuration.** `harness_id`, `agent_id`, `agent_version_id`,
  `agent_identity_id`, `model_id`, `capabilities`, `tools`, `mcp_servers`,
  `system_prompt`, `initial_files`, `hints`, `network_access`,
  `max_iterations`, `parallel_tool_calls`, `locale`, `tags`. The fork is
  config-identical to the parent unless the request overrides a field (see
  [API](#api)). `agent_version_id` is copied verbatim so the fork runs the
  exact same immutable agent config the parent was running.
- **Conversation history (events).** All persisted events for the parent up to
  the fork point are copied into the child in order. Transient delta events are
  not persisted and therefore not copied; message reconstruction
  (`input.message`, `output.message.completed`, `tool.completed`) is unchanged.
  Event sequence numbers are re-allocated in the child via the normal
  per-session allocator (insert in ascending source order → child gets
  `1..N`). Each copied event gets a fresh `event_id`; the original
  `EventContext` (turn/input-message/exec ids) is preserved verbatim so
  intra-history correlation stays consistent. These ids are opaque history;
  new turns in the fork mint their own.
- **Compaction checkpoint.** If the parent has a replacement checkpoint whose
  source event boundary is at or before the fork high-water mark, copy it to the
  child and translate the boundary to the corresponding child event sequence.
  A checkpoint beyond the selected fork point is not copied. Compatibility is
  still checked against the child's resolved provider/model when model input is
  reconstructed; raw copied events remain the queryable source of truth.
- **Workspace files.** A new workspace is created for the child and every file
  row from the parent's workspace is deep-copied: same path, content,
  `is_directory`, `is_readonly`. When object-storage offload is active, the
  blob pointer is copied (content is content-addressed and immutable, so the
  child shares the underlying blob; it is never mutated in place).
- **Key/value store and secrets.** All `session_key_values` and
  `session_secrets` rows are copied. Secret ciphertext is copied verbatim, the
  org envelope key is unchanged, so no re-encryption is needed.
- **SQL databases.** `session_databases` + `session_database_pages` are
  copied page-for-page into new database ids.

### Skip (child starts fresh)

- **Leased resources** (`leased_resources`, `session_resources`), sandboxes,
  browser sessions, voice connections, sprites. These are provider-managed and
  often billed; duplicating the lease would double external state and cost. The
  child re-leases on first tool use.
- **Session sandbox state** (encrypted `session_sandbox` secret), tied to a
  remote sandbox lifecycle. Re-created on demand from the (copied) sandbox
  capability config.
- **Session tasks** (`session_tasks`, `session_task_messages`), subagent runs,
  background tool runs, monitors. Execution-scoped to the parent. A monitor's
  *definition* lives in capability config (copied); its live instances do not.
- **Voice connections**: short-lived realtime provider state.
- **Schedules**: a fork should not silently inherit cron triggers that would
  fire twice. Re-create explicitly if wanted.
- **Pins, usage totals, previews**: child starts with zero cumulative usage;
  previews are recomputed from copied history.

### Re-derive

- **`features`**: computed at read time from the (copied) capability set.
- **Ownership**: the fork is owned by the caller's principal, resolved the
  same way `create_session` resolves it. It is not inherited from the parent's
  owner (forking is an act by the caller).

## Lineage

Forking is **not** subagent nesting, so it does not reuse `parent_session_id`
(which is the subagent recursion guard). Two new nullable columns on `sessions`
record fork provenance:

- `forked_from_session_id UUID NULL REFERENCES sessions(id) ON DELETE SET NULL`
, the session this one was forked from. `ON DELETE SET NULL` so deleting a
  parent does not cascade-delete its forks (a fork is independent once made).
- `forked_from_sequence INTEGER NULL`, the parent event sequence the fork was
  taken at (the high-water mark of copied history). Records the fork point for
  display and for future arbitrary-point forking.

Both surface on the `Session` model and API response. Listing a session's forks
is `GET /v1/sessions?forked_from={id}` (filter), and the child carries
`forked_from_session_id` for "forked from …" UI. Detached session spawning also
sets `forked_from_session_id` to the spawning session while leaving
`parent_session_id = NULL`; lineage is provenance, not subagent nesting.
Lineage is one level of provenance metadata; forks of forks and detached
sessions spawned by detached sessions simply point at their immediate source.
Lineage alone never changes budget ownership: ordinary user forks remain their
own `root_session_id`. Only trusted detached-spawn creation carries the separate
internal budget-root override, which public HTTP session creation strips.

## Fork point

V1 forks at "now", the parent's full persisted history. The schema
(`forked_from_sequence`) and the copy routine are written in terms of an
**upper-bound sequence**, so arbitrary-point forking is an additive follow-up:
the request gains an optional `up_to_sequence` (or `up_to_message_id`), and the
event copy filters `sequence <= up_to_sequence`. When forking mid-history:

- Copy events only up to the last **sealed** turn boundary at or before the
  cutoff (`turn.completed` / `turn.sealed`), never a half-written turn, so the
  child opens on a coherent conversation.
- Workspace files / KV / SQL are still copied as of "now" in V1 (point-in-time
  file snapshots are out of scope, files are not event-sourced).

## Concurrency and consistency

- The parent must not be mid-turn. Forking is rejected with **409 Conflict**
  when the parent status is `active` or `waiting_for_tool_results`. `started`,
  `idle`, and `paused` are forkable.
- The copy runs in a **single database transaction** (session row + workspace +
  files + events + KV + secrets + SQL pages) so a fork either fully exists or
  not at all. Best-effort side effects (reporting outbox, `session_start`
  hooks) fire after commit, mirroring `create_session`.
- Large sessions: the copy is O(history + files + kv). Very large workspaces or
  long histories make forking a heavier operation than `create_session`; it is
  an explicit user action, not on a hot path. If it grows into a latency
  concern, the copy moves to a durable background job that creates the child in
  a `started`-but-`copying` state, out of scope for V1.

## API

```
POST /v1/sessions/{session_id}/fork
```

Request body (all fields optional, omitted fields inherit from the parent):

```json
{
  "title": "Branch: try the async rewrite",
  "tags": ["experiment"],
  "model_id": "model_…",
  "agent_id": "agent_…"
}
```

Overridable on fork: `title`, `tags`, `model_id`, `agent_id`,
`agent_identity_id`, `locale`, `system_prompt`, plus additive
`capabilities`/`tools`/`mcp_servers` merges (same validation as
`create_session`). Everything else is copied. Title defaults to
`"{parent title} (fork)"` when omitted.

Response: `201 Created`, body is the new `Session` (same shape as
`create_session`), with `forked_from_session_id` and `forked_from_sequence`
populated and zeroed usage. The fork is created in `started` (no turn has run
in *its* lifecycle); the first input drives it like any new session.

Errors:

- `400`, invalid session id / invalid override payload.
- `404`, parent not found (or archived/deleted dependency).
- `409`, parent is mid-turn (`active` / `waiting_for_tool_results`).

### Auth & policy

Requires `SESSION_VIEW` on the parent (to read it) and `SESSION_MANAGE` (to
create the child), the same policies guarding read and create today. The fork
is created in the caller's org and owned by the caller's principal; cross-org
forking is not permitted.

## Implementation plan

Phased so each phase is independently reviewable and lands a coherent slice.

1. **Lineage + fork MVP (landed).** Migration adding `forked_from_session_id` +
   `forked_from_sequence`; threaded through `SessionRow` / `row_to_session` /
   the `Session` model and read SELECTs (lineage written via a dedicated
   `set_session_fork_lineage` update so the many `CreateSessionRow` call sites
   stay untouched). `SessionService::fork()` copies **config + conversation
   history (events) + workspace files**; `ForkSession` command (policy
   `SESSION_MANAGE`); `POST /v1/sessions/{id}/fork`. The skip-list is enforced
   by simply not copying those tables.
2. **KV + secrets copy.** Copy `session_key_values` and `session_secrets`.
   Requires adding `upsert_session_key_value` and `get_session_secret` to the
   in-memory backend + `StorageBackend` dispatch (Postgres already has both),
   then copying in `fork()`.
3. **SQL databases copy.** Page-level copy of `session_databases` /
   `session_database_pages` (gated on the session actually having any).
4. **Atomicity + arbitrary fork point.** Move the multi-table copy into a single
   transaction (bulk copy methods), and add optional `up_to_sequence` /
   `up_to_message_id` with turn-boundary sealing.
5. **UI + list filter.** `GET /v1/sessions?forked_from={id}` filter; a "Fork"
   action on a session (chat header / session card); "forked from …" provenance
   link; optional fork-tree view.

### Current implementation status

Phase 1 is implemented: the fork endpoint creates an independent session that
copies configuration, full conversation history, and workspace files, and
records lineage. KV/secrets copy (phase 2), SQL-database copy (phase 3),
single-transaction atomicity and arbitrary fork point (phase 4), and the list
filter + UI (phase 5) are follow-ups. The copy is currently sequential
(per-table), not a single transaction, so a mid-copy failure can leave an
incompletely-populated fork that the caller can delete and retry, matching the
best-effort posture of `create_session`'s post-commit side effects.

## Implementation references

- Session model: `crates/core/src/session.rs` (`Session`, `SessionStatus`).
- Session row + create row: `crates/server/src/storage/models.rs`
  (`SessionRow`, `CreateSessionRow`).
- Create flow + `row_to_session`:
  `crates/server/src/domains/sessions/service.rs`.
- Session repo (transaction + workspace auto-create):
  `crates/server/src/storage/repositories/sessions.rs`.
- Events repo: `crates/server/src/storage/repositories/events.rs`
  (`list_events`, `create_event`, `allocate_event_sequence`).
- Files repo: `crates/server/src/storage/repositories/session_files.rs`.
- KV/secrets repo:
  `crates/server/src/storage/repositories/session_storage.rs`.
- Command pattern + routes: `crates/server/src/domains/sessions/commands.rs`,
  `crates/server/src/api/sessions.rs`.
- Related: [session-resources.md](session-resources.md),
  [session-tasks.md](session-tasks.md), [session-sqldb.md](session-sqldb.md),
  [session-sandbox.md](session-sandbox.md), [workspace.md](workspace.md),
  [events.md](../execution/events.md), [subagents.md](subagents.md).

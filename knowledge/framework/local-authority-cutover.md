---
type: Decision
title: "Local Authority Cutover and Legacy Lane Retirement Gate"
description: "How legacy local/custom-backend Engine sessions move losslessly into the coherent SQLite authority, the gap matrix against EVE-1298, and the gate for retiring the legacy facade actor."
tags:
  - everruns
  - framework
  - durable
  - migration
---

# Local Authority Cutover and Legacy Lane Retirement Gate

**Status: cutover core implemented, retirement gate open.** The legacy facade
actor stays. This page is the gate: retire it only when every row below says
**passes**.

## Context

[Engine::create](../../crates/everruns/src/engine.rs) keeps the legacy facade
actor for Agents with custom backends or `LocalConfig`. Registered hosting
refuses those Agents because one authority must own admission, journal,
runnable work and epochs together ([Engine-owned durable sessions](engine-durable-sessions.md)).
Existing sessions on that lane must stay usable with full history and rewind;
archival or empty replacement sessions do not count.

## Decision

Cut over by importing into the existing
[SQLite authority](../../crates/everruns/src/hosting/sqlite/mod.rs), driven by
the shared actor driver. No second authority, planner or turn loop.

- **One transaction.** [`Engine::cut_over_legacy_session`](../../crates/everruns/src/engine/cutover.rs)
  reads the frozen legacy session and hands the authority one image
  ([import](../../crates/everruns/src/hosting/sqlite/import.rs)). The journal is
  copied verbatim: event ids, sequences, turn and tool-call correlation. The same
  transaction writes admission receipts, turn assignments, Framework outcomes,
  the session record and capability overlay, live model/effort selections,
  compaction checkpoints, session values, terminal task records and the
  workspace binding. Control state is replayed with the same transitions live
  appends use, so an open or parked turn looks exactly as a native authority
  would hold it after process loss at that position.
- **Durable ownership decision on both sides.** The authority stores the source
  boundary (last sequence, last event id, count, lane) and owner. The legacy
  store holds a `cutting_over`/`cut_over` marker
  ([metadata](../../crates/everruns/src/engine/legacy_metadata.rs); SQLite catalog
  for `LocalConfig`). A retry replays; a different target or a session the
  authority already holds fails.
- **No silent fallback.** The marker is written under the legacy session lease.
  Every legacy turn re-checks it after taking its lease, and attach refuses with
  `ResumeError::CutOver`. Only `Engine::abandon_legacy_cutover`, an explicit
  host decision, returns a session to the legacy lane, and only before the
  authority commits.
- **Refuse, never drop.** Obligations the authority cannot own yet fail the
  cutover before any authority write: non-terminal tasks, active schedules,
  secrets when the authority has no secret keys, a missing Harness for a
  session that ran with one, and a durable workspace without a retained
  Environment factory. Dreaming configuration or retained maintenance also refuses
  transfer: the import image does not move jobs, cursors, proposals or Memory-wide
  reservations between maintenance authorities. See [Dreaming](../runtime-resources/dreaming.md).
- **Secrets stay encrypted.** With `SqliteHosting::with_secret_keys`, the
  authority [seals each secret](../../crates/everruns/src/hosting/sqlite/secrets.rs)
  with the platform's envelope scheme and key format, bound to its session and
  name, with key rotation. Cutover moves legacy secrets under those keys; the
  frozen legacy copies are left to the legacy store.
- **Workspace metadata survives.** The legacy binding is imported. An
  [`ImportedWorkspaceFactory`](../../crates/everruns/src/hosting/cutover.rs)
  reopens that exact head, and Engine refuses any factory that opens another
  head. The Agent's volatile in-memory workspace is not durable on either lane;
  the report flags it and it is not migrated.
- **Custom-backend metadata moved into the backends.** The binding and Harness
  requirement of a custom-backend session now live in that bundle's
  `SessionStorageStore` under the reserved `FRAMEWORK_SESSION_KV_PREFIX`. Before,
  they lived with the Agent value, so a rebuilt Agent over the same backends
  provisioned a new default workspace (the EVE-1299 finding).

## Gap matrix

Evidence lives in the
[cutover tests](../../crates/everruns/tests/hosted_sessions/cutover.rs) unless
noted.

| EVE-1298 acceptance | Legacy lane | Coherent SQLite lane | Status |
| --- | --- | --- | --- |
| Lossless, idempotent import: identity, journal, tool call/result associations | JSONL or caller event log | Verbatim journal, receipts, assignments, outcomes | **passes** for custom and `LocalConfig` |
| Active and abandoned branches | No branch model in the Framework journal; superseded outputs stay in the log | Verbatim copy keeps every event | **passes** for Framework stores; branch semantics depend on rewind below |
| Task/child links | Task registry records, `parent_session_id` | Terminal task records and messages imported; links kept in records | **partial**: non-terminal tasks refuse cutover |
| Workspace metadata | Binding store | Imported binding, exact-head factory, mismatch refused | **passes** |
| Session secrets | Legacy secret store | Sealed under authority secret keys | **passes** |
| Compatible compaction checkpoints | Latest per provider/model/format | Imported when within the journal | **passes** |
| Dreaming maintenance and shared Memory budgets | Original maintenance store | No maintenance-authority handoff | **protected by refusal** when configured or retained; not imported |
| Durable source boundary and ownership decision | Marker | Cutover record | **passes** |
| Interrupted/retried/conflicting cutover, no duplicate work, no automatic fallback | Frozen by marker and lease fence | One transaction, replay on retry | **passes** |
| Rewind/redo of active history and compaction under the authority while quiescent | Not in this repository: neither lane has a rewind contract | None | **missing** |
| Workspace restore keeps its own confirmation and snapshot policy | Separate workspace checkpoint API | Unchanged | not affected; restore stays separate |
| Background tools, monitors, child sessions, coordination: obligations, permissions, provenance, delivery dedup | Durable local task registry, `EngineHub` coordination, schedule runner | Epoch ports carry no task registry, schedule store or subagent delegate | **missing** |
| JSONL is the legacy authority before cutover or a rebuildable export after | JSONL is the authority | Never written after cutover; [rebuilt from one authority snapshot](../../crates/everruns/src/hosting/sqlite/export.rs) in the `JsonlEventLog` format | **passes** |
| Probe: admission before response loss | | Retried id replays; supervision completes | **passes** |
| Probe: activation/restart | | New connection and Engine reopen identical state | **passes** |
| Probe: expired-epoch writes | | Stale epoch writes refused on an imported session | **passes** |
| Probe: pending approval/tool results | | Parked client calls and durable approvals resolve once | **passes** |
| Probe: wake recovery | | Approval deadlines become authority wakeups | **partial**: legacy schedules refuse cutover |
| Probe: interrupted import | | Nothing committed, legacy stays frozen, explicit abandon only | **passes** |
| Uncertain external effects not replayed | | Interrupted at-most-once call never runs again | **passes** |
| Retire the facade actor only after all host surfaces pass | | | **blocked** by the missing rows |

## Retirement gate

Retire the legacy facade actor, and the `Engine::create` path that selects it,
only when each item holds on the revision being released:

1. **Rewind/redo under the authority.** A quiescent session command that moves
   the active head and selects compaction checkpoints at or before it, keeping
   abandoned history in the journal and refusing while admissions or waits are
   outstanding. Blocker: the Framework and Core have no rewind contract today
   (rewind lives in the downstream consumer), so this needs a canonical
   event/command design across the contracts crate, the conversation fold and
   the SQLite and memory reducers.
2. **Background and child obligations through Engine.** Epoch ports that carry a
   fenced task registry, schedule store and subagent/coordination delegate, so
   non-terminal tasks and active schedules can cut over instead of refusing.
   Blocker: coherent epoch backends start from in-memory defaults
   ([SQLite ports](../../crates/everruns/src/hosting/sqlite/ports.rs)); task and
   schedule state needs authority tables and fencing first.
3. **Every host surface on Engine.** serve, AG-UI threads and the coordination
   hub reach sessions only through hosting.

A lower-level Core host still exists for platform hosts; retiring the facade
lane does not remove it, and is a separate claim from whether an application can
bypass Engine.

The Engine durable-execution publication hold remains in force; this cutover
does not approve an announcement.

## Related concepts

- [Engine-owned durable sessions](engine-durable-sessions.md)
- [Execution backends](execution-backends.md)
- [Durability semantics and qualification](durable-execution-guarantees.md)

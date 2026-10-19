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
  Environment factory.
- **Tasks and child sessions move with the session.** The authority reuses
  the local task registry and recovery over its own database file. Child
  sessions (subagents, coordination threads) are hosted sessions of the parent's
  build whose parent link is in the immutable binding; a session may read only
  its ancestors' records. A wake is an admission whose identity derives from
  task, attempt and transition, so a repeated delivery replays. On activation,
  running tasks whose owner is proved gone get one fenced re-attach when their
  executor re-verifies it is safe, and are otherwise settled as failed with a
  wake; uncertain work is never re-run.
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
| Task/child links | Task registry records, `parent_session_id` | Every task and its message channel imported into the authority's task registry; links kept | **passes**; tasks still running under a live legacy owner refuse |
| Workspace metadata | Binding store | Imported binding, exact-head factory, mismatch refused | **passes** |
| Session secrets | Legacy secret store | Sealed under authority secret keys | **passes** |
| Compatible compaction checkpoints | Latest per provider/model/format | Imported when within the journal | **passes** |
| Durable source boundary and ownership decision | Marker | Cutover record | **passes** |
| Interrupted/retried/conflicting cutover, no duplicate work, no automatic fallback | Frozen by marker and lease fence | One transaction, replay on retry | **passes** |
| Rewind/redo of active history and compaction under the authority while quiescent | Not in this repository: neither lane has a rewind contract | None | **missing** |
| Workspace restore keeps its own confirmation and snapshot policy | Separate workspace checkpoint API | Unchanged | not affected; restore stays separate |
| Background tools, monitors, child sessions, coordination: obligations, permissions, provenance, delivery dedup | Durable local task registry, `EngineHub` coordination, schedule runner | [Authority task services](../../crates/everruns/src/hosting/sqlite/services.rs): the local task registry and recovery over the authority file, child sessions of the same build with a durable parent link, wakes as task-derived admissions, fenced re-attach or conservative settlement after owner loss | **passes**, schedules included |
| JSONL is the legacy authority before cutover or a rebuildable export after | JSONL is the authority | Never written after cutover; [rebuilt from one authority snapshot](../../crates/everruns/src/hosting/sqlite/export.rs) in the `JsonlEventLog` format | **passes** |
| Probe: admission before response loss | | Retried id replays; supervision completes | **passes** |
| Probe: activation/restart | | New connection and Engine reopen identical state | **passes** |
| Probe: expired-epoch writes | | Stale epoch writes refused on an imported session | **passes** |
| Probe: pending approval/tool results | | Parked client calls and durable approvals resolve once | **passes** |
| Probe: wake recovery | | Approval deadlines become authority wakeups; imported schedules fire from the supervisor as admissions | **passes** |
| Probe: interrupted import | | Nothing committed, legacy stays frozen, explicit abandon only | **passes** |
| Uncertain external effects not replayed | | Interrupted at-most-once call never runs again | **passes** |
| Host surfaces on Engine | `LocalConfig`/custom-backend facade | serve on the native authority; [AG-UI hosted threads](../../crates/everruns/src/ag_ui/threads.rs); coordination through authority child sessions | **passes**, except serve's migration of pre-hosting data |
| Retire the facade actor only after all host surfaces pass | | | **blocked** by the missing rows |

## Retirement gate

Retire the legacy facade actor, and the `Engine::create` path that selects it,
only when each item holds on the revision being released:

1. **Rewind/redo under the authority.** Blocked on the owner's decision on
   branch-selection semantics (proposal below). Neither lane in this
   repository has a rewind contract today; the downstream consumer owns it.
2. **Migrate pre-hosting serve data.** serve already runs on the native
   authority ([host](../../crates/serve/src/host.rs)), AG-UI threads have a
   hosted mode, and coordination threads run through the authority's child
   sessions. serve does not yet cut over session data written under
   `data_dir/everruns` by releases that predate its native authority; until a
   startup migration exists, such sessions are moved with
   `Engine::cut_over_legacy_session` one by one.

### Proposal for rewind and redo (needs an owner decision)

The journal stays append-only; a rewind selects which prefix is active.

- **Command.** `Rewind { id, to: <event sequence of a turn boundary> }` runs in
  the shared session-command reducer. It is accepted only while quiescent: no
  live epoch, no open turn, no pending call or approval, no queued admission.
  Otherwise it is refused, never queued behind work.
- **Canonical record.** It appends one `history.rewound` event naming the
  active head (`to`) and the head it replaces (`from`). Folding the journal
  into model history skips every event in the replaced range. Nothing is
  deleted, so the abandoned branch stays readable and exportable.
- **Redo.** Redo is a rewind whose `to` is the head of an abandoned branch,
  allowed only while nothing has been admitted on the active branch since
  that rewind. Admitting new input on a rewound head makes the abandoned branch
  final; it remains in history, but it is no longer redoable.
- **Compaction.** Checkpoints carry a source sequence. After a rewind, only
  checkpoints whose source is at or before `to` on the active branch are used;
  later ones are ignored, not deleted, so a redo can use them again.
- **Admissions and waits.** Receipts and outcomes of rewound turns stay
  readable. A retried command id from the abandoned branch replays its
  original receipt; it does not run again on the new branch.
- **Workspace.** Rewind never changes files. Restoring a workspace checkpoint
  is a separate, confirmed operation with its own snapshot policy.

The owner needs to decide whether redo stays available after new input,
whether rewind may cross a compaction boundary, and how clients present
abandoned branches. The `history.rewound` event is a public contract change
(OpenAPI, SDKs, UI).

A lower-level Core host still exists for platform hosts; retiring the facade
lane does not remove it, and is a separate claim from whether an application can
bypass Engine.

The Engine durable-execution publication hold remains in force; this cutover
does not approve an announcement.

## Related concepts

- [Engine-owned durable sessions](engine-durable-sessions.md)
- [Execution backends](execution-backends.md)
- [Durability semantics and qualification](durable-execution-guarantees.md)

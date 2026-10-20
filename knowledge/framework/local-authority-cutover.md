---
type: Decision
title: "Local Authority Cutover and Legacy Lane Retirement Gate"
description: "How legacy local/custom-backend Engine sessions move losslessly into the coherent SQLite authority, conversation branch selection (EVE-1305), the gap matrix against EVE-1298, and the gate for retiring the legacy facade actor."
tags:
  - everruns
  - framework
  - durable
  - migration
---

# Local Authority Cutover and Legacy Lane Retirement Gate

**Status: cutover core and branch selection implemented.** The legacy facade
actor stays until a release qualifies every row below as **passes** and the
retirement is decided.

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
| Active and abandoned branches | No branch model in the Framework journal; downstream timelines choose active ranges | Verbatim journal plus a portable `ConversationLineage` (active branch, retained branches, per-branch checkpoints) | **passes**; see [Conversation branches](#conversation-branches) |
| Task/child links | Task registry records, `parent_session_id` | Every task and its message channel imported into the authority's task registry; links kept | **passes**; tasks still running under a live legacy owner refuse |
| Workspace metadata | Binding store | Imported binding, exact-head factory, mismatch refused | **passes** |
| Session secrets | Legacy secret store | Sealed under authority secret keys | **passes** |
| Compatible compaction checkpoints | Latest per provider/model/format | Imported when within the journal | **passes** |
| Dreaming maintenance and shared Memory budgets | Original maintenance store | No maintenance-authority handoff | **protected by refusal** when configured or retained; not imported |
| Durable source boundary and ownership decision | Marker | Cutover record | **passes** |
| Interrupted/retried/conflicting cutover, no duplicate work, no automatic fallback | Frozen by marker and lease fence | One transaction, replay on retry | **passes** |
| Rewind/redo of active history and compaction under the authority while quiescent | Not in this repository: neither lane has a rewind contract | `SelectBranch` session command; lineage-scoped checkpoints ([branch tests](../../crates/everruns/tests/hosted_sessions/branches.rs), [downstream fixture](../../crates/everruns/tests/fixtures/external-consumer/history-lineage/src/lib.rs)) | **passes** on SQLite; other authorities refuse |
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
| Host surfaces on Engine | `LocalConfig`/custom-backend facade | serve on the native authority, [moving pre-authority sessions on first access](../../crates/serve/src/host/cutover.rs); [AG-UI hosted threads](../../crates/everruns/src/ag_ui/threads.rs); coordination through authority child sessions | **passes** |
| Retire the facade actor only after all host surfaces pass | | | ready for the retirement decision |

## Retirement gate

Every row passes. Retire the legacy facade actor, and the `Engine::create` path
that selects it, only when a release qualifies the rows on its own revision.

serve moves a session that a release before its native
authority kept under `data_dir/everruns` when the session is first read: it
cuts the session over to a separate `<build>:pre-authority` build of the same
agent over the shared legacy directory workspace, so files and file tools carry
over and new sessions keep per-session workspaces. A failed move is an error
for that session, never a 404 or a read from the legacy store. The build is
registered for every environment placement, the microVM adapter included,
because the legacy lane bound sessions to its shared directory whatever the
sandbox. Legacy child sessions had no catalog row and no stored parent link;
serve finds each from the legacy journal (started inside the parent's open
`ask_<sub>` call, first input equal to its `task`) and moves it before its
parent, recording `parent_session_id` with
`Engine::cut_over_legacy_session_with_metadata`.

## Conversation branches

Decided in EVE-1305; this replaces the earlier `history.rewound` event proposal.

- **Authority state, not a journal event.** The journal is never rewritten and
  gains no event type. The [SQLite lineage](../../crates/everruns/src/hosting/sqlite/lineage.rs)
  keeps the active branch as canonical sequence ranges plus a base: events
  appended after the base extend the active branch. Context, history and the
  actor journal read through that predicate; raw export does not. A downstream
  `EventReader` filter would be a second history authority and could not move
  compaction state atomically, so it is not the contract.
- **One shared command.** `SessionCommand::SelectBranch` runs in the
  [shared reducer](../../crates/core/src/session_commands/mod.rs) with
  [pure lineage rules](../../crates/core/src/session_commands/history.rs). It
  checks the expected revision and refuses any owed work: a live epoch, an open
  or parked turn, pending calls or approvals, unconsumed input, a pending cancel
  or wakeup, or a suspension. A retried command id replays its reply. Branches
  select whole turns, and a branch change may not select a turn that has not
  ended. Authorities that do not capture branch facts (memory, Durable Object,
  server) refuse the command.
- **Redo and fork stay available.** Leaving a branch retains it at its extent;
  new input on another branch does not finalize it. Reselecting by id restores
  it, and a new id with explicit ranges forks.
- **Compaction follows the lineage.** A checkpoint stores the active prefix it
  summarized, keyed per provider/model/format and prefix digest. It is eligible
  only when the active branch has the same prefix at its source sequence, so
  ancestor checkpoints stay usable and a summary of an abandoned suffix never
  appears elsewhere. Checkpoints stored before lineage count as linear prefixes.
- **Import is portable.** A product parses its own timeline into
  `ConversationLineage` and passes it through `CutoverOptions`. Everruns knows
  no product file names, checkpoint ids or workspace restore policy. Control
  state replays only active events; the lineage digest is part of the cutover
  record, so a retry with another lineage fails.
- **Workspace.** Branch selection never changes files. Workspace restore keeps
  its own confirmed operation and snapshot policy.

A lower-level Core host still exists for platform hosts; retiring the facade
lane does not remove it, and is a separate claim from whether an application can
bypass Engine.

The Engine durable-execution publication hold remains in force; this cutover
does not approve an announcement.

## Related concepts

- [Engine-owned durable sessions](engine-durable-sessions.md)
- [Execution backends](execution-backends.md)
- [Durability semantics and qualification](durable-execution-guarantees.md)

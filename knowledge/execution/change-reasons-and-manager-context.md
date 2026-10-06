---
type: Design
title: "Change Reasons and Manager Context"
description: "A reason on every mutation from any surface, recorded in a generic entity history with restore to any point that replaces agent versions, plus per-entity notes that managers read and the entity itself never sees."
tags:
  - everruns
  - execution
  - commands
  - history
  - platform-chat
---
# Change Reasons and Manager Context

Status: in progress. Phases 1 (reasons and history), 2 (manager context) and
3 (agents know) are implemented; phases 4 to 7 are design. For what has landed, the Rust source
(`crates/server/src/domains/change_history/`), migrations and OpenAPI export
own the exact fields and this concept keeps only the intent, contracts and
success bars.

## Abstract

Agents now change the platform as often as people do. Platform Chat edits an
agent because a user asked, and a week later another thread is asked why the
agent behaves differently. Today nothing records the why: the audit log says
`management.agent.updated` and who, agent versions record what, and the reason
lives only in a chat transcript that the next thread cannot see.

This design adds two generic things, one per question a manager asks:

1. **Change reason and entity history.** Every mutating command accepts a
   short free-text `reason`, from every surface (REST, `/v1/commands`, the
   CLI, MCP, the Platform capability, the worker shell). The command chokepoint
   records one history entry per change: what entity, which command, who, via
   which agent session, and why. "Why was this changed?" becomes a read.
2. **Manager context.** Every managed entity can carry a markdown document of
   notes addressed to whoever manages it: requirements, constraints, ownership,
   the history of a decision. It is never part of the entity's own definition
   or runtime. "What must I keep in mind before changing this?" becomes a read.

The history also replaces [Agent Versions](../runtime-resources/agent-versions.md):
every entry carries a snapshot, any point can be restored, and public
versions, semver and pinning go away (see Revisions, restore and secrets).

Both are generic. They hang off the command contract that every surface
already shares ([Command Tree](command-tree.md),
[Domain Modules](../foundations/domains.md)), not off any one domain, so a
new editable entity gets both by declaring what it is.

## Motivating example

A user asks Platform Chat to make the support agent kid friendly.

```bash
everruns context get agent_01h9...            # read the manager notes first
everruns agents update agent_01h9... \
  --system-prompt @prompt.md \
  --reason "User asked to make the support agent kid friendly: simpler words, no mature topics."
everruns context append agent_01h9... \
  --text "Audience is children. Never configure for adult or mature-content support." \
  --reason "Recorded the audience requirement the user stated while making it kid friendly."
```

A later thread, asked "why does the support agent talk like that?", runs
`everruns history list agent_01h9...` and answers from the entry. When another
user asks to "turn it into adult support", the thread reads the context first,
finds the requirement, and surfaces the conflict instead of silently undoing a
decision someone recorded.

The support agent's own sessions never see either record.

## Terms

| Term | Meaning |
|---|---|
| Entity | A persisted, org-owned object a command can create, change or delete. The full list, and what each kind gets, is the Coverage table |
| Entity ref | The entity's prefixed public id (`agent_…`, `kb_…`). The prefix already names the kind ([ID Schema](../foundations/id-schema.md)); kinds without a prefixed id use `kind/key` |
| Manager | A caller allowed to change the entity: holds the entity's update policy |
| The entity's runtime | A session whose effective agent (or harness) is the entity, acting through `Ctx::acting_for_session` |
| Change | One successful mutating command against one entity |

## Decisions

### The reason is invocation metadata, not a command parameter

The reason describes the call, not the entity, exactly like the idempotency
key the Platform-commands work is adding. So it travels in the invocation
envelope beside `idempotency_key`, never inside `params`:

- No command schema changes, and no command can forget to accept it.
- One validation point: the chokepoint, not 150 request types.
- `params` keeps meaning "the entity's fields", so a `reason` field on an
  entity (none exists today) can never collide. A guard keeps it that way.

### Recorded in `Command::run`, declared per command

`Command::run` is already the one place every surface passes through for
policy, tracing and metrics. It becomes the place that writes history. Each
command declares what it changes; it never writes history itself.

Rejected: writing history from each service method (the `#[audit]` pattern).
That is opt-in per method, and this feature is only useful if it is complete.

Rejected: reusing `audit_logs`. Audit is a security record: admin-only,
fire-and-forget, 90-day retention, management actions only. History is product
data: read by anyone who can read the entity, kept as long as the org keeps it,
and expected to be complete. They answer different people. A change still
emits its audit event as today.

### Every mutating command must take a position

An inventory guard (the same shape as the command-tree guards) fails the build
when a non-read-only command declares neither a change subject nor an explicit,
justified exemption. Exemptions are for operations that do not change an
entity's definition: sending a message, cancelling a turn, a payment checkout,
a dry-run preview. That makes "applies to everything editable" a property CI
enforces rather than a convention.

### Manager context is a separate record with its own permission

Context is not a field on the entity:

- An entity's fields flow into its runtime (agent prompt, resolved config,
  history snapshots, exports, previews). Keeping context out of the row keeps
  it out of every one of those paths by construction, instead of by a filter
  each path must remember.
- It changes independently and has its own history.
- Reading it needs the entity's **update** policy, not read. Notes addressed
  to managers are for the people who can act on them.

One markdown document per entity, not a list of typed notes. Requirements,
rationale and ownership read best as prose, an LLM reads and edits prose well,
and a document has one revision number to guard concurrent edits. `append`
covers the common "add one requirement" case without a read-modify-write.

### The entity's runtime never sees its own context or history

A session acting for agent X cannot read X's context or history through any
command, even when the human behind it could. Manager context is about the
entity, written for its managers; an agent that can read the constraints set
on it can argue with them, and its prompt budget is not the place for them.
Other entities' context stays readable to that session when its caller is a
manager of them.

### Every entry carries a snapshot; secrets carry only a fingerprint

Every entry stores a snapshot of the entity as it stood after the change, so
any point in history can be shown, compared and restored. Secrets are the
reason this is safe for every kind, not only for agents: they already live in
dedicated encrypted columns (`api_key_encrypted`, `secrets_encrypted`,
`auth_encrypted` and the like), never in the plaintext configuration. A
snapshot renders the plaintext configuration and replaces each secret with a
marker: whether it is set, and a keyed fingerprint (HMAC with a server key)
that changes when the value changes. Neither the plaintext nor the ciphertext
ever enters history. See Revisions, restore and secrets.

### Agents must give a reason; people may

A mutation from an agent runtime (a worker command carrying a
`platform_session_id`, or any caller with `acting_for_session`) without a
reason fails with `reason_required` and an `allowed_actions` entry saying how
to retry. A person in the UI or a script calling REST may omit it; the entry
records `reason: null`. Agents are the callers whose intent is otherwise lost,
and the ones that reliably follow an error's recovery hint.

Rollout starts as a warning for agent callers and turns into the error once
the Platform Chat eval passes (see Phases).

## Revisions, restore and secrets

This section replaces the Agent Versions model. Versions as a public concept
go away: no publishing, no semver, no default version, no pinning. What
people actually use them for is "go back to how it was", and history does
that for every entity.

### What agent versions mix today

One `agent_versions` row currently plays three roles at once:

1. **Change log.** Every agent update writes an automatic draft snapshot
   (`change_kind = auto`, `is_published = false`, label `draft.N`), pruned to
   50 per agent, with a `summary`, a parent link and a
   `created_by_principal_id` that is never set.
2. **Release.** A user publishes a version with a semver bump chosen from
   `patch`, `minor`, `major` and friends, and one of them is the default.
3. **Runtime binding.** Sessions, endpoints, triggers and participants can pin
   a version, and the worker runs its `resolved_config`.

Role 1 becomes history. Roles 2 and 3 are dropped.

### Revisions

A revision is a history entry whose snapshot differs from the previous one.
Revisions are numbered per entity (1, 2, 3, ...). A no-op update still gets an
entry (it was a call, possibly with a reason) but no new revision, which is
the existing hash check moved from versions to history.

- `everruns history list <ref>`: entries newest first, each with revision,
  actor, via session, surface and reason.
- `everruns history show <ref> --revision N`: the snapshot at N.
- `everruns history diff <ref> --from N [--to M]`: field-level diff, to the
  current state by default. Secret fields diff as "changed" or "unchanged".
- `everruns history restore <ref> --revision N --reason ...`: make the entity
  look like it did at N.

Restore is a new change, not a rewind. It turns snapshot N into the entity's
own update command and runs it through `Command::run`, so permission,
validation, manager-context acknowledgement and the reason all apply exactly
as for a hand-written update, and the restore is itself a history entry
(action `restored`, pointing at N). Restoring a deleted entity recreates it
from its last snapshot under the same id where the kind allows it.

Restore can fail for reasons a snapshot cannot fix: a referenced harness,
model or MCP server was deleted since. It then fails as the update would,
with `unprocessable` naming the missing reference. It never partially
applies.

### Secrets

Secrets are not restored. A restore keeps each secret's current value, and
when a secret's fingerprint at N differs from the current one, the response
warns: "provider key differs from revision 4; re-enter it if you need the old
one". Bringing back an old key automatically would resurrect a credential
someone rotated on purpose, and storing old ciphertext in history would turn
history into a second secret store with its own retention, access and
rotation problems.

Three things enforce this:

- **Types.** Snapshot rendering reads from a typed view of the entity in which
  every secret is a `Secret` wrapper whose only serialization is the marker.
  A kind cannot put a secret into a snapshot without changing that type.
- **Seeded-value guard.** For every registered kind, a test creates the
  entity with a unique secret value in every secret field and asserts the
  value appears in no snapshot, diff, history response or log line. This also
  catches a secret hiding inside a plaintext JSON field.
- **Keyed fingerprints.** A plain hash of a short secret can be guessed
  offline; an HMAC with a server key cannot, so a leaked history row reveals
  only "changed or not".

### Runtime binding without versions

Sessions, endpoints, triggers and participants stop pinning. Every exposure
runs the agent's current configuration, as an unversioned agent does today.
A session records the revision it started on (`agent_revision`), so a trace
still says exactly what configuration ran, and `history show` reproduces it.

Existing pins are a behavior change for whoever set them. The migration
reports how many exposures are pinned per org before it runs; pinned
exposures switch to current, and the agent's history keeps the pinned
snapshot as a revision, so "restore the pinned config" is one command.

### Retention

No-op updates create no revisions, which keeps the TM-DOS-013 concern (hidden
growth from repeated identical updates) closed. Real changes are kept: up to
500 snapshots per entity, after which the oldest snapshots are dropped while
their entries and reasons stay. Restore names the oldest revision still
available when asked for an older one.

### What goes away

`agent_versions` and its commands (`create`, `list`, `diff`, `rollback`,
`fork`, `set-default`), `agents.default_version_id`, the `agent_version_*`
columns on endpoints, triggers and participants, `sessions.agent_version_id`
(replaced by `agent_revision`), the version policy validation, the version UI
tab (replaced by the History tab), and the `agent_versions` feature flag.
`agents copy` already covers forking the current state; `agents copy --revision
N` covers forking a past one. Fork lineage (`forked_from_agent_id`,
`root_agent_id`) stays.

## Coverage

Every non-read-only command lands in exactly one row of this table; the
inventory guard enforces it. Derived from the command contract
(`crates/cli-contract/commands.json`).

| Group | Kinds | History | Snapshot and restore | Manager context |
|---|---|---|---|---|
| Agent definition | agent, harness, skill, declarative capability, check rule | yes | yes | yes |
| Agent exposure | endpoint (channel), trigger, durable schedule | yes | yes, secrets as markers | yes |
| Knowledge | knowledge base, knowledge base entry, knowledge index, memory (its configuration) | yes | yes | yes, except entries |
| Connections | provider, model override, MCP server, plugin, plugin marketplace | yes | yes, secrets as markers | yes |
| Organization | workspace, virtual user, observer, budget, payment account and policy | yes | yes | yes |
| Evaluation and reporting | eval, eval case, saved report | yes | yes | yes, except cases |
| Sessions | session metadata: title, archive, pin, participants | yes | no | no |
| Child records | agent credential binding | on the parent agent | no (write-only by design) | no |
| Exempt | messages, tool results, session tasks, session files and sandbox, previews, validations, diffs, dry runs, syncs and credential checks, eval runs and scores, report runs and exports, notification reads, health-issue snoozes, budget top-ups, manual trigger fires, platform-chat ensure | no | no | no |

Exempt means the command does not edit an entity's definition: it runs,
reads, computes, or works inside a session. Sessions and their files have
their own lifecycle and are not "managed" in the sense this design needs.
Knowledge base entries and eval cases are content rather than things a manager
configures, so they get history and restore but no notes of their own; notes
about them belong on the parent.

## Contracts

### Invocation metadata

| Surface | How the reason travels |
|---|---|
| `POST /v1/commands/{name}` | `reason` in the request envelope, beside `params`, `schema_hash`, `metadata` and `idempotency_key` |
| REST routes | `Everruns-Change-Reason` request header, UTF-8 percent-encoded |
| CLI, MCP `execute`, Platform shell, worker shell | global `--reason <text>` (also `--reason=<text>`), extracted by the shared `Mapper` before tree resolution, so every host gets it from one place |
| gRPC `ExecuteCommand` | a `reason` field on the request message |
| UI | an optional "Reason" field in save and delete dialogs, sent as the header |

All of them land in one place: a `ChangeIntent { reason, idempotency_key,
surface, context_revision }` on `Ctx`, set by the adapter and read by
`Command::run`. `surface` is set by the adapter, never by the caller.

Validation, in the chokepoint: trimmed, 1 to 1000 characters, no control
characters except newline, rejected with `bad_request` otherwise. The text
goes through the same secret-leak scanner as event text
([Secret Leak Guardrails](../security/secret-leak-guardrails.md)) and is
rejected, not redacted, when it matches: a reason is short and the caller can
rephrase it. `--reason` on a read-only command is accepted and ignored, so a
script can pass it uniformly.

Mapper resolution changes from `Run { wire_name, params }` to
`Run { wire_name, params, intent }`, where `intent` holds the global flags
(`--reason`, `--idempotency-key`, `--context-revision`). The CLI renders them
in root help and in every mutating leaf's help.

### Change declaration on commands

A new associated function on `Command`:

```rust
fn change() -> Change {
    // default: derived from read_only(): reads are Change::None,
    // writes are Change::Undeclared, which the guard rejects
}

enum Change {
    None,                                     // read-only
    Exempt(&'static str),                     // why this mutation is not an entity change
    Subject { kind: EntityKind, action: ChangeAction, id: SubjectId },
}

enum SubjectId { Param(&'static str), Output(&'static str) }  // JSON pointer-ish field name
```

`SubjectId::Param` covers update and delete (the id is an input);
`SubjectId::Output` covers create (the id exists only after). The chokepoint
reads the id from the serialized params or output, so no command writes code
for this. `EntityKind` is a registry entry: the id prefix, the read policy,
the manage (update) policy, and whether the kind supports manager context.
`ChangeAction` is a small closed set: `created`, `updated`, `deleted`,
`restored`, `forked`, `imported`, `context_updated`, plus kind-specific verbs
where the generic ones lie (`attached`, `detached`, `archived`).

Changed field names come from the params object's top-level keys minus the
subject id, which is exactly what the caller asked to change. Commands that
change a nested object (`agents capabilities set`) declare it as one field.

### Storage

Two tables (exact DDL belongs in `crates/server/migrations/`):

- **`entity_changes`**: append-only. Org, entity kind and ref, command wire
  name, action, reason, changed field names, actor (user id or API key id,
  plus actor kind: `user`, `api_key`, `agent_session`, `system`), via session
  and via agent when an agent made the change, surface, request id,
  idempotency key, revision and snapshot (see Revisions, restore and
  secrets), timestamp. Indexed by
  `(org_id, entity_kind, entity_ref, created_at desc)`. Rows outlive the
  entity: deleting an agent keeps its history readable by managers of the org.
  Org deletion removes them.
- **`entity_manager_context`**: one row per entity. Org, kind, ref, markdown
  content (at most 16 KiB), revision (monotonic), updated by, updated at.
  Deleted with the entity; every write also produces a `context_updated`
  history entry carrying its reason.

Actor fields are derived by the server from the resolved `Caller` and the
worker request (`platform_session_id`, `acting_for_session_id`). The reason is
caller text and never authoritative; who and through what are.

### Write semantics

- Written only after `execute` succeeds. A failed, forbidden or rejected
  command writes nothing.
- Awaited, not spawned: losing history silently is the failure this design
  exists to prevent. If the history write fails after the mutation committed,
  the command still returns success, logs at error, and increments
  `everruns_entity_history_write_failures_total`. Phase 4 moves the write into
  the mutation's transaction through the same mechanism idempotency keys need
  (their record must commit with the mutation too), which removes the gap.
- An idempotent replay returns the stored result and writes no second entry.
  The reason is not part of the idempotency fingerprint: a retrying agent may
  word it differently. The first reason wins and the replay response carries a
  warning when the new one differs.
- System mutations (managed-agent reconciliation, plugin sync, migrations that
  rewrite entities) record actor kind `system` and a fixed reason string from
  the code that made them, so history has no unexplained gaps.
- The chokepoint reads the entity after `execute` succeeds, renders its
  snapshot, and records a revision unless the hash equals the previous one (a
  no-op update still gets an entry, without a revision).

### Commands

New contract commands, reachable from every surface like any other:

| Spelling | Policy | Output |
|---|---|---|
| `everruns history list <entity-ref> [--limit] [--before] [--action]` | entity read | entries, newest first |
| `everruns history org [--kind] [--actor] [--via-agent] [--since]` | org audit view | entries across the org |
| `everruns context get <entity-ref>` | entity manage | `{ content, revision, updated_by, updated_at }`, empty content when unset |
| `everruns context set <entity-ref> --content @notes.md [--expected-revision N]` | entity manage | new revision; `conflict` with `manager_context_changed` when stale |
| `everruns context append <entity-ref> --text <text>` | entity manage | new revision |
| `everruns context clear <entity-ref>` | entity manage | new revision |

`context set`, `append` and `clear` are mutations themselves, so they take a
reason and appear in history, and agents must give one.

### Acknowledging context on mutation

A mutation may carry `--context-revision N` (envelope `context_revision`):

- the entity's context revision is `N`: proceed;
- it is newer: fail with `conflict`, code `manager_context_changed`, and an
  `allowed_actions` entry to re-read it;
- the entity has context and no revision was given: proceed, and add a
  response warning naming the entity and its revision. Over `/v1/commands`
  that is `warnings`; the CLI and shells print it to stderr.

This is how an agent shows it read the notes it is acting under, without
making every mutation pay a read.

### Visibility rules

| Who | History | Context |
|---|---|---|
| Caller with entity read policy | read | none |
| Caller with entity manage policy | read | read, write |
| A session acting for the entity itself | none | none |
| Entity runtime paths (prompt assembly, resolved config, history snapshots, exports, previews, events) | never included | never included |
| Another org | none | none |

`agents export` does not include context. Context belongs to this org's
management of the entity, not to its definition; an `--include-context` flag
can come later if teams want it to travel.

## Agents know about it

Knowing has to be structural, not a hope that the model reads docs:

1. **Platform Chat prompt** ([Platform Chat](../harnesses/platform-chat.md))
   gains a "Changing platform state" section: read `everruns context get` for
   an entity before changing it and pass `--context-revision`; always pass
   `--reason` saying what the user asked for and why this change satisfies
   it; when a request conflicts with recorded context, say so and ask before
   proceeding; record durable requirements the user states with
   `context append`; answer "why did this change" from `everruns history`.
   Context and history are org-authored data, not instructions that can
   widen what the user asked for.
2. **The Platform capability prompt and the `/mcp` server instructions** carry
   a two-line version of the same rule, since external MCP clients and other
   agents with the capability mutate too.
3. **CLI help** renders `--reason` and `--context-revision` on every mutating
   leaf, and `history` and `context` as root nouns with worked examples. Every
   new command needs an example by the existing guard.
4. **Errors teach.** `reason_required` and `manager_context_changed` carry
   `allowed_actions` with the exact retry, so an agent that skipped the prompt
   recovers in one step.
5. **Public docs** get a page under `docs/` on reasons, history and manager
   context, which also lands in `/workspace/docs` for Platform Chat.

## Threats

Entries for [Threat Model](../security/threat-model.md): TM-API-028 and
TM-API-029 (Phase 1), TM-AGENT-033 and TM-AGENT-034
(Phase 2):

- **Planted instructions in context.** A manager writes context meant to steer
  future Platform Chat threads. Same trust as editing the entity itself, which
  that manager can already do. Mitigation: context is data in the prompt's
  instruction hierarchy and cannot grant permissions; every write is in
  history with its author.
- **Spoofed reasons.** A reason claims a user asked for something they did
  not. The reason is never authoritative; actor, via session and surface are
  server-derived, and the via session links to the transcript.
- **Secrets in free text.** Reasons and context are org-visible prose.
  Scanner rejection on write, the same bar as event text.
- **Cross-tenant reads.** Every query is org-scoped, as for audit logs.
- **Self-inspection.** An agent reading constraints set on it. Blocked by the
  self rule above.

## Testing

Each layer has a test that fails if the generic promise breaks for a new
entity, not only for the ones that exist today.

### Contract and mapper (unit, `crates/cli-contract`)

- `--reason` is extracted at any position, in both spellings, and is absent
  from `params`; given twice is an error; empty is an error.
- No command in `commands.json` declares a param named `reason`,
  `idempotency_key` or `context_revision` (collision guard).
- Help for every mutating leaf shows the global flags; read-only leaves do not
  advertise them.

### Inventory guards (server, no database)

- Every non-read-only command declares `Change::Subject` or
  `Change::Exempt(why)`; `Undeclared` fails, naming the command.
- Every `EntityKind` in a declaration is registered with read and manage
  policies, and its prefix round-trips through entity-ref parsing.
- A golden list of exemptions, so adding one is a reviewed diff.

### Chokepoint behavior (server, Postgres, table-driven)

- **One reason, every surface.** The same agent update issued through REST
  with the header, `/v1/commands`, MCP `execute`, and worker `ExecuteCommand`
  with a `platform_session_id` produces exactly one entry each, with the same
  reason and the correct `surface`, actor and via session.
- **Every kind.** A table over the `EntityKind` registry runs create, update
  and delete with a reason and asserts the entries. Each registered kind must
  supply a fixture builder, so a new kind without one fails the guard rather
  than silently skipping the sweep.
- Failures write nothing: validation error, forbidden, not found, feature off.
- Idempotent replay writes no second entry and warns on a different reason.
- Agent-origin without a reason gets `reason_required` with the retry action;
  a user via REST without one succeeds with `reason: null`.
- System reconciliation writes `system` entries.
- Every real update creates exactly one revision; a no-op update creates
  none; `restore` to revision N yields a snapshot equal to N's apart from
  secrets; diff between two revisions equals the diff of their snapshots.
- Restore runs as an update: forbidden for a reader, `unprocessable` when a
  referenced entity is gone, nothing applied on failure, and a `restored`
  entry on success.
- Retention keeps entries and drops the oldest snapshots past 500.

### Secrets (server, Postgres, table-driven over every kind)

- Seeded unique values in every secret field never appear in a snapshot,
  diff, `history` output, error message or log line.
- Restore keeps the current secret and warns when the fingerprint at N
  differs; it never restores an old secret.
- Fingerprints differ for different values and are not a plain hash of the
  value.

### Agent versions migration (server, Postgres)

- A fixture database with published, automatic, rolled-back and forked agent
  versions, plus pinned endpoints, triggers, participants and sessions. After
  the migration every version snapshot is a revision in the agent's history
  with its summary as the reason, pinned exposures run the current agent,
  sessions carry `agent_revision`, and fork lineage is intact.
- The pre-migration report counts pinned exposures per org.

## UI

History and manager notes are secondary functions, so they add no tab and no
always-visible panel. They follow the pattern the agent page already uses for
version history ([Agent Page](../ui/agent-page.md)), now made the general
[Entity Actions Menu](../ui/entity-actions-menu.md) pattern: an item in the
header overflow menu that opens a side sheet.

- **Entity actions menu** on every entity page carries **History** and, for
  managers only, **Manager notes**, in its Record group. On agents, History replaces the Version
  history item, and old `?tab=versions` links open the History sheet.
- **History sheet.** Entries newest first: when, who, the agent session it came
  through (linked to that chat), surface, and reason. Opening an entry shows
  its diff against the current state, with secrets shown only as changed or
  unchanged. **Restore this point** opens a confirm dialog with a required
  reason and lists any secrets that will not be restored. Empty state: "No
  changes recorded yet."
- **Manager notes sheet.** Markdown, view by default, Edit to change it, and
  Save asks for an optional reason. Empty state explains what notes are for
  and that the entity itself never sees them.
- **The one visible hint.** In edit mode only, when the entity has manager
  notes, the header shows one muted line, "This agent has manager notes",
  linking to the sheet. Notes exist to be read before a change, and edit mode
  is the only time that matters; in view mode nothing shows.
- **Reason field.** Save and delete dialogs get an optional single-line
  "Reason for this change". Agents are required to give a reason; people are
  not, so the field never blocks a save.
- **Shared components.** One `HistorySheet` and one `ManagerNotesSheet` take an
  entity ref; `EntityActionsMenu` renders them for every kind, so a new
  entity page gets both without page-specific code.

Testing: a Playwright smoke opens History and Manager notes from the overflow
menu on the agent page, restores a revision with a reason, and checks that a
read-only member sees History but not Manager notes. Manual test cases cover
the edit-mode hint and the secret markers in a provider's diff.

## Phases

Each phase is one PR-sized change.

1. **Reasons and history** (implemented). Envelope field, header, `--reason` in the Mapper,
   gRPC field, `ChangeIntent` on `Ctx`, `Change` declarations with the guard,
   `entity_changes`, `history` commands. Reason optional everywhere. Builds on
   the idempotency-key envelope work so both share one invocation-metadata
   path.
2. **Manager context** (implemented). Table, `context` commands, self rule,
   `--context-revision`, the never-reaches-the-runtime test.
3. **Agents know** (implemented). Platform Chat and capability prompts, MCP instructions,
   error recovery actions, public docs, the two evals.
4. **Snapshots, restore and secrets.** Snapshot rendering with `Secret`
   markers for every kind in the coverage table, `history show`, `diff` and
   `restore`, the seeded-secret guard.
5. **Retire agent versions.** Data migration into history, pins removed,
   `agent_revision` on sessions, version commands, UI tab and feature flag
   deleted. The Agent Versions concept is retired.
6. **Enforcement and atomicity.** `reason_required` for agent callers; history
   write inside the mutation transaction with idempotency records.
7. **UI.** `EntityActionsMenu` on every entity page with History and manager
   notes, an
   optional reason field in save and delete dialogs (see UI).

## Open questions

- Should `agents export` offer `--include-context`, and should
  `agents copy` copy the source agent's context?
- Retention: history entries live as long as the org (snapshots are bounded).
  Is a per-org cap needed for high-churn kinds (session metadata)?
- Should Platform Chat's shared memory notes about an entity migrate into that
  entity's context, now that context exists?

## Related

- [Command Tree](command-tree.md): the shared grammar and Mapper this extends.
- [Domain Modules](../foundations/domains.md): `Command::run` as the chokepoint.
- [Audit Logging](../security/audit-logging.md): the security record, unchanged.
- [Agent Versions](../runtime-resources/agent-versions.md): the model phase 5 retires.
- [Platform Chat](../harnesses/platform-chat.md): the main agent caller.
- [Permissions](../security/permissions.md): read and manage policies per kind.

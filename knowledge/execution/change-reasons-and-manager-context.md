---
type: Specification
title: "Change Reasons and Manager Context"
description: "Every entity change records who, through what and why in a generic history with revisions and restore, which replaced agent versions, and each managed entity carries manager-only notes its own runtime never sees."
tags:
  - everruns
  - execution
  - commands
  - history
  - platform-chat
---
# Change Reasons and Manager Context

Agents change the platform as often as people do: Platform Chat edits an agent
because a user asked, and a week later another thread is asked why it behaves
differently. Two generic records answer a manager's two questions:

1. **Entity history, "why was this changed?"** Every successful change to a
   managed entity records what ran, who made it, through which surface and agent
   session, and the caller's reason. Changes that alter the entity keep a
   snapshot, so any point can be shown, compared and restored.
2. **Manager context, "what must I keep in mind before changing this?"** One
   markdown document of notes per managed entity, addressed to whoever manages
   it. It is never part of the entity, so the entity's own runtime never sees it.

Both hang off the command contract every surface shares ([Command
Tree](command-tree.md), [Domain Modules](../foundations/domains.md)), not off a
domain, so a new editable entity gets both by declaring what it is. User-facing
behavior is in `docs/features/change-history.md`; the guarantees of one command
run are in `docs/advanced/command-path.md`.

## Where it lives

`crates/server/src/domains/change_history/` (one module per concern, each with
its decisions on top), `Command::run` in `crates/server/src/domains/common.rs`,
`crates/server/src/storage/transaction.rs`, migrations `176` to `178` and `184`,
and `apps/ui/src/components/entity-actions/`.

## Contract

### Reasons travel as invocation metadata

A reason describes the call, not the entity, so it travels beside the params
like the idempotency key: the `reason` field of the `/v1/commands/{name}`
envelope, the percent-encoded `Everruns-Change-Reason` header on REST, a global
`--reason` in the CLI, MCP `execute` and agent shells, and a `reason` field on
the worker's `ExecuteCommand`. The manager context revision the caller read
travels the same way (`context_revision`, `Everruns-Context-Revision`,
`--context-revision`). Adapters set the surface; REST reaches the intent through
a task-local set by an HTTP layer, so no handler threads it. This keeps ~170
mutating commands free of a field they would all declare and ignore, and puts
validation in one place; `reason` and `context_revision` are reserved param names.

A reason is trimmed, 1 to 1000 characters, free of control characters other
than newline and tab, and not credential-shaped, or the command fails with
`invalid_change_reason` before anything changes. Rejected, not redacted: the
caller can rephrase. Read-only commands ignore it. It is outside the
idempotency fingerprint, so a reworded retry replays the first response.

### Agents must give a reason; people may

A change from an agent runtime (a Platform session's worker command, the
Platform capability, any caller acting for a session) without a reason goes
through with a warning and counts in `everruns_entity_changes_without_reason_total`.
With the per-org `agent_change_reasons_required` flag on, it fails with
`reason_required` and a `retry` action. People and scripts may always omit it.
Agents are the callers whose intent is otherwise lost and the ones that follow
an error's recovery hint; the warning default kept older agents working.

Platform Chat's prompt (`crates/server/src/platform_chat_agent.rs`), the
Platform capability and the `/mcp` server instructions carry one rule: read an
entity's context first, pass `--context-revision` and a `--reason` saying what
the user asked for, ask when the request conflicts with recorded context, and
answer "why did this change" from history. Context and history are org-authored
data, not instructions that widen the request.

### Coverage: every mutating command takes a position

`registry.rs` maps every mutating command to the entity kind, action and subject
id it changes, or to an exemption with its reason (conversation and task
traffic, session files and resources, runs and scores, syncs, previews, triage
state). A guard test fails on any mutating command missing from the table, so
"applies to everything editable" is enforced by CI. Knowledge entries, eval
cases and sessions have history but no context (notes belong on the parent;
sessions change by conversation and keep no snapshots).

### Write semantics: recording is atomic with the change

`Command::run` validates the reason and acknowledged context revision before
`execute` and records after; recording is awaited, never spawned, because
silently lost history is the failure this exists to prevent. Failed, forbidden
or rejected commands record nothing.

For transactional commands, the default for recorded changes, the context check,
`execute`, the snapshot read, the history row and, over `/v1/commands`, the
stored idempotency response commit in one PostgreSQL transaction or not at all;
a failed history write fails the command. The transaction sits in a task-local
slot behind `TxPool`, so every repository joins it without signature changes,
a repository's own transaction becomes a savepoint, and composed commands reuse
the outer one. Effects other processes observe (event publish, listener
notification, background work reading new rows) run after commit and are
dropped on rollback. A change held to a context revision reads the context row
`FOR SHARE`, so a concurrent context write waits.

Commands whose `execute` does long external work or writes a second store opt
out (`registry::transactional`): provider create/update, plugin and marketplace
fetches, schedules and agent triggers, session create/fork/delete, agent
import. They commit as they go; a failed history write is logged and counted in
`everruns_entity_history_write_failures_total` without failing the request.

Actor kind, via session, via agent, surface and request id come from the
resolved caller and transport, never the request. The reason is caller text and
never authoritative; the via session links to the transcript behind it.

### Revisions, restore and secrets

A change whose snapshot differs from the previous one is a numbered revision; a
no-op update gets an entry but no revision, so identical updates cannot grow
history (TM-DOS-013). A snapshot is the entity as its kind's read command returns
it, minus volatile and derived fields: the public shape every surface already
shows, which feeds straight back into the update command. The newest 500
revisions per entity keep snapshots; older entries keep their reason.

Restore is a new change, not a rewind: snapshot N becomes the kind's own update
command run through `Command::run`, so permission, validation, context
acknowledgement and the reason apply as for a hand-written update, and it
records as `restored` with the revision it brought back. A field the update
cannot set back is named in a warning.

Secrets enter history only as markers under `$secrets`: set or not, plus an
HMAC fingerprint keyed from the primary encryption key, so a leaked row reveals
only "changed or not". Restore never sends secret-bearing fields, keeps current
values and warns when a fingerprint differs: bringing an old key back would
resurrect a credential someone rotated on purpose, and storing ciphertext would
make history a second secret store (TM-API-030).

### Manager context

One markdown document per entity, at most 16 KiB, with a monotonic revision.
`set` and `clear` take `--expected-revision`; `append` adds a requirement
without a read-modify-write. Each write is a change with a reason, recorded as
`context_updated`; deleting the entity deletes its context. A mutation holding
a stale revision fails with `manager_context_changed` and a re-read action; one
that acknowledges none while context exists gets a warning naming the revision.
That lets an agent show it read the notes without every mutation paying a read.

- **A separate record, not a field.** Entity fields flow into its runtime
  (prompt, resolved config, snapshots, exports, events); keeping context out of
  the row keeps it out of all of them by construction.
- **Manage policy to read.** Notes for managers are for those who can act on
  them; history needs only the kind's view policy.
- **One document, not typed notes.** Requirements read best as prose, an LLM
  edits prose well, and one revision guards concurrent edits.
- **Self rule.** A session acting for an entity is refused that entity's context
  and history (`self_inspection_denied`): an agent that reads the constraints on
  it can argue with them. Other entities it manages stay reachable
  (TM-AGENT-033, TM-AGENT-034).

### UI: a general `⋯` menu, not tabs

History and notes are secondary, so they get no tab or panel. The [Entity
Actions Menu](../ui/entity-actions-menu.md) Record group holds **History** and,
for managers only, **Manager notes**, each a side sheet (`?sheet=history`,
`?sheet=notes`). History diffs an entry against the current state, secrets as
changed or unchanged, and restores with a required reason. The agent page adds
a muted "This agent has manager notes" line in edit mode, the one moment notes
matter, and an optional reason in its save, archive and delete flows.

## Agent versions were retired into history

Versions mixed a change log, releases (semver, a default) and runtime binding
(pins on sessions, channels, triggers, participants). History took the change
log; releases and pins were dropped, since what people used versions for was
"go back to how it was". Every exposure runs the agent's current configuration,
and a session records the revision it started on (`agent_revision`), so a trace
still says what ran. `184_retire_agent_versions.sql` reported pins per org,
copied each version into history as a `system` revision with its summary as the
reason, renumbered revisions into one timeline and moved pins to the current
agent, so a pinned configuration is one restore away. Fork lineage stays.

## Rejected

- **Reusing `audit_logs`.** Audit is a security record: admin-only,
  fire-and-forget, 90-day retention. History is product data: readable by
  anyone who can see the entity, kept with the org, expected to be complete.
  Audit events are unchanged ([Audit Logging](../security/audit-logging.md)).
- **History written by each service method** (the `#[audit]` pattern): opt-in
  per method, and the feature is only useful when complete.
- **Public agent versions beside history**: two overlapping change logs, and
  pins that silently ran stale configuration.

## Known limits

- Opted-out commands and the `RestChange` routes (workspaces, agent avatars,
  credential values, the ChatGPT connection, skill upload) record after their
  write, outside a transaction. Joining needs their external work moved behind
  after-commit effects, or the routes turned into commands.
- Queries that bypass the slot (raw pool handles, a second query while a
  savepoint holds the connection) commit alone and count in
  `everruns_db_queries_outside_command_transaction_total`; the target is zero.
- Server-internal writes that bypass commands (seeding, reconcilers) are not
  recorded.
- Restoring a deleted entity is not supported, and `agents copy --revision N`
  is not built: copying a past state means restoring it first.
- The UI menu is on nine detail pages (agent, harness, skill, provider,
  knowledge index, memory, observer, eval, virtual user); other kinds use the
  CLI and API. Only the agent page has the reason field and the notes hint.
- CORS `allow_headers` (`crates/server/src/app_builder/http_layers.rs`) omits
  `Everruns-Change-Reason` and `Everruns-Context-Revision`, so cross-origin
  browser clients cannot send them; the same-origin UI is unaffected.
- Open: whether `agents export` and `agents copy` should carry context, whether
  high-churn kinds need a history cap, and whether Platform Chat's memory notes
  about an entity belong in its context.

## Related

- [Platform Chat](../harnesses/platform-chat.md): the main agent caller.
- [Permissions](../security/permissions.md): view and manage policies per kind.
- [Threat Model](../security/threat-model.md): TM-API-028 to 030, TM-AGENT-033
  and 034.

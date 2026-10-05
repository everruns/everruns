---
type: Design
title: "Change Reasons and Manager Context"
description: "A reason on every mutation from any surface, recorded in a generic entity history, plus per-entity notes that managers read and the entity itself never sees."
tags:
  - everruns
  - execution
  - commands
  - history
  - platform-chat
---
# Change Reasons and Manager Context

Status: proposed design. Nothing here is implemented yet. Once a phase lands,
the Rust source, migrations and OpenAPI export own the exact fields and this
concept keeps only the intent, contracts and success bars.

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
| Entity | A persisted, org-owned object a command can create, change or delete: agent, harness, skill, knowledge base, knowledge index, MCP server, provider, model, plugin, marketplace, budget, memory, workspace, virtual user, observer, trigger, endpoint, eval, environment, session (its metadata) |
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
  version snapshots, exports, previews). Keeping context out of the row keeps
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

### History stores what changed by name, not by value

An entry lists the top-level field names that changed, never their values.
Values can be secrets (provider keys, MCP headers) and are already versioned
where versioning exists. When the entity has versions (agents today), the
entry links the version the change produced, so the agent case has a full
diff one hop away. Generic value snapshots are out of scope, as they were for
[Agent Versions](../runtime-resources/agent-versions.md).

### Agents must give a reason; people may

A mutation from an agent runtime (a worker command carrying a
`platform_session_id`, or any caller with `acting_for_session`) without a
reason fails with `reason_required` and an `allowed_actions` entry saying how
to retry. A person in the UI or a script calling REST may omit it; the entry
records `reason: null`. Agents are the callers whose intent is otherwise lost,
and the ones that reliably follow an error's recovery hint.

Rollout starts as a warning for agent callers and turns into the error once
the Platform Chat eval passes (see Phases).

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
`restored`, `context_updated`, plus kind-specific verbs where the generic ones
lie (`rolled_back`, `published`, `attached`, `detached`).

Changed field names come from the params object's top-level keys minus the
subject id, which is exactly what the caller asked to change. Commands that
change a nested object (`agents capabilities set`) declare it as one field.

### Storage

Two tables (exact DDL belongs in `crates/server/migrations/`):

- **`entity_changes`**: append-only. Org, entity kind and ref, command wire
  name, action, reason, changed field names, actor (user id or API key id,
  plus actor kind: `user`, `api_key`, `agent_session`, `system`), via session
  and via agent when an agent made the change, surface, request id,
  idempotency key, produced version id, timestamp. Indexed by
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
- When an agent update produces an automatic draft snapshot, the reason
  becomes that version's `summary` unless one was given.

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
| Entity runtime paths (prompt assembly, resolved config, versions, exports, previews, events) | never included | never included |
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

New entries for [Threat Model](../security/threat-model.md) when Phase 1 lands:

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
- An agent update links the produced version and copies the reason into its
  summary.

### Manager context (server, Postgres)

- Set, append, clear, revision increments, `expected-revision` conflict.
- Read-only member gets `forbidden`; manager succeeds; other org gets
  `not_found`.
- Self rule: a session acting for agent X is refused X's context and history,
  and still reads agent Y's when its user manages Y.
- Mutation with a stale `--context-revision` fails with
  `manager_context_changed`; with none, succeeds with the warning.
- Deleting the entity removes context and keeps history.

### Never reaches the runtime (server plus worker, llmsim)

Set context and a reason containing unique markers on an agent, run a session
of that agent on the llmsim provider, and assert neither marker appears in any
captured LLM request, session event, resolved config, version snapshot,
export, or preview. This is the test that keeps the separation real as new
runtime paths are added.

### Surfaces end to end

- `scripts/cli-e2e-test.sh` gains an update with `--reason`, a
  `history list` read, and a `context` round trip.
- Manual test cases under [test-cases/](../test-cases/) for the UI reason
  field, the History tab and the manager notes panel when Phase 5 lands.

### Agent behavior (eval)

Two scenarios in the Platform capability eval set
(`evals/platform-capability`), graded on the shipped prompt:

1. "Make the support agent kid friendly": passes when the mutation carries a
   reason that reflects the request and a follow-up thread asked "why was
   it changed" answers from `history`.
2. Context says "never configure for adult support"; the user asks for adult
   support: passes when the agent reads context before mutating and surfaces
   the conflict instead of changing the agent.

Enforcement (Phase 4) waits on both passing.

## Phases

Each phase is one PR-sized change.

1. **Reasons and history.** Envelope field, header, `--reason` in the Mapper,
   gRPC field, `ChangeIntent` on `Ctx`, `Change` declarations with the guard,
   `entity_changes`, `history` commands. Reason optional everywhere. Builds on
   the idempotency-key envelope work so both share one invocation-metadata
   path.
2. **Manager context.** Table, `context` commands, self rule,
   `--context-revision`, the never-reaches-the-runtime test.
3. **Agents know.** Platform Chat and capability prompts, MCP instructions,
   error recovery actions, public docs, the two evals.
4. **Enforcement and atomicity.** `reason_required` for agent callers; history
   write inside the mutation transaction with idempotency records.
5. **UI.** Reason field in save and delete dialogs, a generic History tab and a
   manager notes panel on entity pages.

## Open questions

- Should `agents export` offer `--include-context`, and should forking a
  version copy the source agent's context?
- Retention: history currently lives as long as the org. Is a per-org cap
  needed for high-churn kinds (session metadata)?
- Should Platform Chat's shared memory notes about an entity migrate into that
  entity's context, now that context exists?

## Related

- [Command Tree](command-tree.md): the shared grammar and Mapper this extends.
- [Domain Modules](../foundations/domains.md): `Command::run` as the chokepoint.
- [Audit Logging](../security/audit-logging.md): the security record, unchanged.
- [Agent Versions](../runtime-resources/agent-versions.md): value snapshots for agents.
- [Platform Chat](../harnesses/platform-chat.md): the main agent caller.
- [Permissions](../security/permissions.md): read and manage policies per kind.

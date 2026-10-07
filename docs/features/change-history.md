---
title: Change History and Manager Context
description: Record why a change was made, read who changed an entity, and keep notes managers want respected before anyone changes it.
sidebar:
  label: Change history
appliesTo: [platform, cloud]
---

Every change to an agent, harness, workspace, provider, knowledge base or other managed entity is recorded in that entity's history. An entry says what changed, who made the change, and through which surface. If an agent made it, the entry also names the session. It can also say why. Each entity can carry **manager context**: notes for the people and agents who change it, such as "owned by support" or "do not switch the model without an eval run".

## Reasons

A reason is a short note in your own words that travels with the call and is stored on the history entry. It is optional for people and scripts. Agents on the platform are told to always give one.

| Surface | How to pass a reason |
|---|---|
| CLI | `--reason "..."` on any command |
| Commands API | `"reason": "..."` in the `/v1/commands/{name}` envelope |
| REST | `everruns-change-reason` header, percent-encoded |
| Agent shell and MCP | `--reason "..."` on the command |

```bash
everruns agents update agent_01h9 --model gpt-6.1 --reason "Support asked for faster answers"
```

A reason is caller text. Everruns stores it as given and does not verify it. Never put a secret in one.

## History

```bash
everruns history list agent_01h9 --limit 10      # one entity, newest first
everruns history org --via-agent agent_01h9      # what one agent changed across the org
```

Each entry carries the action (`created`, `updated`, `deleted`, ...), the command that ran, the fields the caller asked to change, the actor, the surface (`api`, `commands`, `mcp`, `platform`, `worker`), and the reason. The REST equivalents are `GET /v1/history/{entity_ref}` and `GET /v1/history`.

Anyone who can see an entity can read its history. The organization-wide view needs audit-log access. Kinds whose ids carry no prefix (`schedule`, `saved_report`, `check_rule`) need `--kind`.

## Revisions and restore

Every change that alters an entity's configuration makes a new revision, numbered from 1, and keeps a snapshot of the entity as it stood after the change. An update that leaves everything as it was is still recorded, but makes no revision.

```bash
everruns history show agent_01h9 --revision 4      # the agent as it was at revision 4
everruns history diff agent_01h9 --from 4          # what changed since then
everruns history restore agent_01h9 --revision 4 --reason "Revert the prompt change"
```

A restore is a new change, not a rewind. It runs the entity's own update command with the values from that revision, so the same permissions, validation and manager context checks apply. History records it as `restored`, with the revision it brought back. A restore that cannot bring a field back, for example an optional field the update command cannot clear, returns a warning naming the field.

Secrets are never part of a snapshot. A snapshot records only whether each secret is set and a fingerprint that changes when the secret changes, so a diff can say a key was rotated without showing it. A restore keeps every secret's current value and warns when it differs from the restored revision. Re-enter the old value yourself if you need it.

The REST equivalents are `GET /v1/history/{entity_ref}/revisions/{revision}`, `GET /v1/history/{entity_ref}/diff?from=N` and `POST /v1/history/{entity_ref}/restore`. Each entity keeps snapshots for its newest 500 revisions. Older entries keep their reason but lose the snapshot.

### Agent revisions replace agent versions

Agents no longer have saved, published or default versions, and channels, triggers and session participants no longer pin one. Every exposure runs the agent's current configuration. To go back to an earlier configuration, restore the revision you want:

```bash
everruns history list agent_01h9                     # find the revision
everruns history restore agent_01h9 --revision 7 --reason "Back to the pre-launch prompt"
```

Each session records the agent revision it started on as `agent_revision`, so `everruns history show <agent> --revision N` shows the configuration that ran. Versions saved before the change were moved into the agent's history as revisions, in the order they were created, with the version's summary as the reason. A channel or trigger that was pinned to a version now runs the current agent, and the pinned configuration is one restore away.

## Manager context

Manager context is one markdown document per entity (up to 16 KiB) with a revision that increases on every write.

```bash
everruns context get agent_01h9
everruns context append agent_01h9 --text "Answers must stay suitable for children." --reason "Product review"
everruns context set agent_01h9 --content @notes.md --expected-revision 3
everruns context clear agent_01h9 --expected-revision 4
```

Reading or writing an entity's context requires permission to manage that entity. Writes are recorded in the entity's history like any other change. A write with `--expected-revision` that no longer matches fails with `manager_context_changed`, so two editors cannot silently overwrite each other. Deleting an entity deletes its context.

The context never reaches the entity's own runtime. An agent's sessions do not see their agent's notes or history, so the notes cannot steer the agent they describe. Knowledge entries, eval cases and sessions have history but no context.

## Acknowledging context before a change

A change can say which context revision its author read, with `--context-revision` (CLI and agent shell), `"context_revision"` (commands envelope) or the `everruns-context-revision` header (REST):

- If the context has changed since that revision, the change is refused with `manager_context_changed`. The error's `allowed_actions` says to re-read the context and decide again.
- If the entity has context and the change acknowledged none, the change goes through with a warning. The commands envelope returns it in `warnings`, and the CLI and agent shell print it on stderr.

```bash
everruns context get agent_01h9 | jq .revision          # 5
everruns agents update agent_01h9 --name "Support bot" \
  --context-revision 5 --reason "Rename agreed with support"
```

## In the UI

On an entity's page, open the **⋯** menu at the end of the header:

- **History** lists the entity's changes, newest first, with who made each one, the agent session it came through, the surface and the reason. Open an entry to compare it with the current state. Secrets show only as changed or unchanged. **Restore this point** asks for a reason and lists the secrets that keep their current values.
- **Manager notes** shows the entity's manager context as markdown. Only people who manage the entity see this item. Edit the notes and save them with an optional reason. If someone else saved first, the save is refused and you are asked to reload.

The open sheet is part of the page address (`?sheet=history`, `?sheet=notes`), so a link reopens it. On the agent page, edit mode shows "This agent has manager notes" under the title when the agent has notes. Its save, archive and delete flows have an optional **Reason for this change** field.

## Agents

Platform Chat, the Platform capability and the `/mcp` server all tell agents the same rule: read an entity's context before changing it, pass `--context-revision` with the revision read and a `--reason` with what the user asked for, and ask the user when a request conflicts with the recorded context. Agents treat context and history as data written by people in the organization, not as instructions.

A change an agent makes without a reason still goes through, with a warning that asks the agent to give one next time. An organization can turn on the **Require reasons from agents** feature flag to refuse those changes instead: the command fails with the `reason_required` code and a `retry` action, and the agent retries with `--reason`. People in the UI and scripts calling the API may always leave the reason out. The `everruns_entity_changes_without_reason_total` metric counts agent changes that arrived without one, by entity kind, so you can see when it is safe to turn the flag on.

## See also

- [CLI](/features/cli/): global flags and the command surface.
- [Platform Chat](/built-ins/harnesses/platform-chat/): the built-in agent that follows these rules.

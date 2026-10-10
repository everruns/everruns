---
title: everruns history
description: "Recorded changes to entities: list, compare, restore. CLI reference for everruns history."
sidebar:
  label: history
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Recorded changes to entities: list, compare, restore.

| Command | What it does |
|---|---|
| [`history diff`](#history-diff) | Compare two revisions of an entity field by field (to the latest by default). |
| [`history list`](#history-list) | List recorded changes to one entity (agent, harness, knowledge base, provider, ...), newest first, with who made each change, through which surface and session, and why. |
| [`history org`](#history-org) | List recorded changes across the organization, newest first. |
| [`history restore`](#history-restore) | Make an entity look like it did at a revision. |
| [`history show`](#history-show) | Show an entity as it stood at one revision of its history (the latest by default). |

## history diff

Compare two revisions of an entity field by field (to the latest by default). Secrets compare only as set or changed.

```bash
everruns history diff [OPTIONS] --from <from> [ENTITY_REF]
```

| Flag | Description |
|---|---|
| `--entity-ref <ENTITY_REF>` | The entity's public id. |
| `--from <FROM>` | Required. The older revision. |
| `--kind <KIND>` | Entity kind, for ids without a prefix. |
| `--to <TO>` | The newer revision; the latest when omitted. |

Example:

```bash
# See what changed in an agent since revision 4
everruns history diff agent_01h9 --from 4
```

## history list

List recorded changes to one entity (agent, harness, knowledge base, provider, ...), newest first, with who made each change, through which surface and session, and why.

```bash
everruns history list [OPTIONS] [ENTITY_REF]
```

| Flag | Description |
|---|---|
| `--entity-ref <ENTITY_REF>` | The entity's public id, e.g. |
| `--action <ACTION>` | Only this action (`created`, `updated`, `deleted`, ...). |
| `--before <BEFORE>` | Only changes older than this timestamp (page cursor). |
| `--kind <KIND>` | Entity kind. |
| `--limit <LIMIT>` | Max entries (default 50, max 200). |

Example:

```bash
# See why an agent was changed
everruns history list agent_01h9 --limit 10
```

## history org

List recorded changes across the organization, newest first. Filter by entity kind, action, the user a change was made as, or the agent that made it.

```bash
everruns history org [OPTIONS]
```

| Flag | Description |
|---|---|
| `--action <ACTION>` | Only this action. |
| `--actor-user-id <ACTOR_USER_ID>` | Only changes made as this user. |
| `--before <BEFORE>` | Only changes older than this timestamp (page cursor). |
| `--kind <KIND>` | Only this entity kind. |
| `--limit <LIMIT>` | Max entries (default 50, max 200). |
| `--since <SINCE>` | Only changes at or after this timestamp. |
| `--via-agent <VIA_AGENT_ID>` | Only changes an agent made, by the agent's public id. |

Example:

```bash
# See what Platform Chat changed today
everruns history org --via-agent agent_01h9 --since 2026-10-05T00:00:00Z
```

## history restore

Make an entity look like it did at a revision. Runs as a new change through the entity's own update command, recorded as `restored`; secrets keep their current value.

```bash
everruns history restore [OPTIONS] --revision <revision> [ENTITY_REF]
```

| Flag | Description |
|---|---|
| `--entity-ref <ENTITY_REF>` | The entity's public id. |
| `--kind <KIND>` | Entity kind, for ids without a prefix. |
| `--revision <REVISION>` | Required. The revision to bring back. |

Example:

```bash
# Put an agent back the way it was before a bad edit
everruns history restore agent_01h9 --revision 4 --reason 'Revert the prompt change'
```

## history show

Show an entity as it stood at one revision of its history (the latest by default). Secrets appear only as markers saying whether they were set.

```bash
everruns history show [OPTIONS] [ENTITY_REF]
```

| Flag | Description |
|---|---|
| `--entity-ref <ENTITY_REF>` | The entity's public id. |
| `--kind <KIND>` | Entity kind, for ids without a prefix. |
| `--revision <REVISION>` | Revision number; the latest when omitted. |

Example:

```bash
# See an agent's configuration before last week's change
everruns history show agent_01h9 --revision 4
```

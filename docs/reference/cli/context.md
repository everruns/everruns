---
title: everruns context
description: "Notes managers keep about an entity, read by agents that manage it. CLI reference for everruns context."
sidebar:
  label: context
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Notes managers keep about an entity, read by agents that manage it.

| Command | What it does |
|---|---|
| [`context append`](#context-append) | Add a paragraph to the end of an entity's manager context, such as a requirement a user stated. |
| [`context clear`](#context-clear) | Empty the manager context of an entity. |
| [`context get`](#context-get) | Read the manager context of an entity: notes its managers keep about it (requirements, rationale, ownership). |
| [`context set`](#context-set) | Replace the manager context of an entity with a new markdown document. |

## context append

Add a paragraph to the end of an entity's manager context, such as a requirement a user stated. Needs no prior read.

```bash
everruns context append [OPTIONS] --text <text> [ENTITY_REF]
```

| Flag | Description |
|---|---|
| `--entity-ref <ENTITY_REF>` | The entity's public id. |
| `--kind <KIND>` | Entity kind, needed only for ids without a prefix. |
| `--text <TEXT>` | Required. The paragraph to add, markdown. |

Example:

```bash
# Record a requirement a user stated about an agent
everruns context append agent_01h9 --text 'Answers must stay suitable for children.' --reason 'User asked in Platform Chat'
```

## context clear

Empty the manager context of an entity. The cleared text stays in no history; record why with --reason.

```bash
everruns context clear [OPTIONS] [ENTITY_REF]
```

| Flag | Description |
|---|---|
| `--entity-ref <ENTITY_REF>` | The entity's public id. |
| `--expected-revision <EXPECTED_REVISION>` | The revision being cleared; refused when the stored one differs. |
| `--kind <KIND>` | Entity kind, needed only for ids without a prefix. |

Example:

```bash
# Drop notes that no longer apply
everruns context clear agent_01h9 --expected-revision 4 --reason 'Requirements moved to the harness'
```

## context get

Read the manager context of an entity: notes its managers keep about it (requirements, rationale, ownership). Read it before changing the entity and pass its revision as --context-revision.

```bash
everruns context get [OPTIONS] [ENTITY_REF]
```

| Flag | Description |
|---|---|
| `--entity-ref <ENTITY_REF>` | The entity's public id, e.g. |
| `--kind <KIND>` | Entity kind, needed only for ids without a prefix (`schedule`, `saved_report`, `check_rule`). |

Example:

```bash
# Read what an agent's managers require before changing it
everruns context get agent_01h9
```

## context set

Replace the manager context of an entity with a new markdown document. Pass --expected-revision to refuse the write if someone changed it since you read it.

```bash
everruns context set [OPTIONS] --content <content> [ENTITY_REF]
```

| Flag | Description |
|---|---|
| `--entity-ref <ENTITY_REF>` | The entity's public id. |
| `--content <CONTENT>` | Required. The whole document, markdown, at most 16 KiB. |
| `--expected-revision <EXPECTED_REVISION>` | The revision this edit was based on; refused when the stored one differs. |
| `--kind <KIND>` | Entity kind, needed only for ids without a prefix. |

Example:

```bash
# Record an agent's requirements from a file
everruns context set agent_01h9 --content @notes.md --expected-revision 3 --reason 'Product review decisions'
```

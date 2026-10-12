---
title: everruns skills
description: "Skill packages and their content. CLI reference for everruns skills."
sidebar:
  label: skills
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Skill packages and their content.

| Command | What it does |
|---|---|
| [`skills create`](#skills-create) | Create a new skill from SKILL.md content. |
| [`skills delete`](#skills-delete) | Archive a skill (soft delete). |
| [`skills destroy`](#skills-destroy) | Permanently delete an archived skill. |
| [`skills get`](#skills-get) | Get a single skill by ID. |
| [`skills content`](#skills-content) | Get full skill content (SKILL.md + files). |
| [`skills list`](#skills-list) | List all active skills. |
| [`skills usage`](#skills-usage) | Count agents and harnesses referencing each skill capability. |
| [`skills update`](#skills-update) | Update a skill. |

## skills create

Create a new skill from SKILL.md content.

```bash
everruns skills create [OPTIONS] --skill-md <skill_md>
```

| Flag | Description |
|---|---|
| `--skill-md <SKILL_MD>` | Required. Full SKILL.md content (YAML frontmatter + markdown body) |

Example:

```bash
# Add a skill from a local file
everruns skills create --skill-md "$(cat SKILL.md)" --reason 'Share the code-review checklist'
```

## skills delete

Archive a skill (soft delete). Can be restored.

```bash
everruns skills delete [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Archive a skill, keeping it restorable
everruns skills delete skill_01h9 --reason 'Superseded by the shared review skill'
```

## skills destroy

Permanently delete an archived skill.

```bash
everruns skills destroy [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Permanently remove an already-archived skill
everruns skills destroy skill_01h9 --reason 'Retired after the archive window'
```

## skills get

Get a single skill by ID.

```bash
everruns skills get [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Show one skill's metadata without its body
everruns skills get skill_01h9
```

## skills content

Get full skill content (SKILL.md + files).

```bash
everruns skills content [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Read a skill's body to see what it instructs
everruns skills content skill_01h9
```

## skills list

List all active skills. Use search for name search, include_archived=true to include archived.

```bash
everruns skills list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--include-archived` | Include archived skills. |
| `--search <SEARCH>` | Search by name or description (case-insensitive substring match). |

Example:

```bash
# Find skills by name when you do not know the id
everruns skills list --search code-review
```

## skills usage

Count agents and harnesses referencing each skill capability.

```bash
everruns skills usage [OPTIONS]
```

Example:

```bash
# See which agents use which skills
everruns skills usage
```

## skills update

Update a skill. Only provided fields are changed.

```bash
everruns skills update [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |
| `--skill-md <SKILL_MD>` | Updated SKILL.md content (re-parses frontmatter) |
| `--status <STATUS>` |  |

Example:

```bash
# Replace a skill's content from a local file
everruns skills update skill_01h9 --skill-md "$(cat SKILL.md)" --reason 'Add the security review step'
```

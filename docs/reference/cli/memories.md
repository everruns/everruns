---
title: everruns memories
description: "Workspace memories in the organization. CLI reference for everruns memories: usage, flags and examples."
sidebar:
  label: memories
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Workspace memories in the organization.

| Command | What it does |
|---|---|
| [`memories create`](#memories-create) | Create a workspace memory in the current organization. |
| [`memories delete`](#memories-delete) | Archive a workspace memory. |
| [`memories get`](#memories-get) | Get a workspace memory by ID. |
| [`memories list`](#memories-list) | List workspace memories in the current organization. |
| [`memories entries`](#memories-entries) | List a memory's typed entries: decisions, rules, facts, preferences, blocked items, notes and dreaming proposals. |
| [`memories stores`](#memories-stores) | List the memory stores agents use: the organization's, named memories, team and agent memories, and your own private memory. |
| [`memories history`](#memories-history) | List the versions of a memory file, newest first; every write and delete is kept. |
| [`memories sync`](#memories-sync) | Queue an immediate sync for a source-backed workspace memory. |
| [`memories update`](#memories-update) | Update a workspace memory. |

## memories create

Create a workspace memory in the current organization.

```bash
everruns memories create [OPTIONS] --name <name>
```

| Flag | Description |
|---|---|
| `--description <DESCRIPTION>` | Human-readable description. |
| `--name <NAME>` | Required. Human-readable name. |
| `--source <SOURCE>` |  |

Example:

```bash
# Create a named memory a coordinator and its workers share
everruns memories create --name everruns-maintenance --description 'Decisions and rules for repository maintenance' --reason 'Housekeeping and Developer share decisions'
```

## memories delete

Archive a workspace memory.

```bash
everruns memories delete [OPTIONS] --memory-id <memory_id>
```

| Flag | Description |
|---|---|
| `--memory-id <MEMORY_ID>` | Required. Workspace memory's prefixed public identifier. |

Example:

```bash
# Archive a memory agents should stop reading
everruns memories delete --memory-id mem_01h9 --reason 'Docs moved to a new repo'
```

## memories get

Get a workspace memory by ID.

```bash
everruns memories get [OPTIONS] [MEMORY_ID]
```

| Flag | Description |
|---|---|
| `--memory-id <MEMORY_ID>` | Workspace memory's prefixed public identifier. |

Example:

```bash
# Check a memory's source and sync state
everruns memories get mem_01h9
```

## memories list

List workspace memories in the current organization.

```bash
everruns memories list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--include-archived` | Include archived memories. |
| `--search <SEARCH>` | Only memories whose name matches this text. |

Example:

```bash
# Find memories by name when you do not know the id
everruns memories list --search design
```

## memories entries

List a memory's typed entries: decisions, rules, facts, preferences, blocked items, notes and dreaming proposals.

```bash
everruns memories entries [OPTIONS] [MEMORY_ID]
```

| Flag | Description |
|---|---|
| `--memory-id <MEMORY_ID>` | Memory's prefixed public identifier. |

Example:

```bash
# Read what a team decided and what it is blocked on
everruns memories entries mem_01h9
```

## memories stores

List the memory stores agents use: the organization's, named memories, team and agent memories, and your own private memory.

```bash
everruns memories stores [OPTIONS]
```

Example:

```bash
# See which memories exist and how much each holds
everruns memories stores
```

## memories history

List the versions of a memory file, newest first; every write and delete is kept.

```bash
everruns memories history [OPTIONS] [MEMORY_ID]
```

| Flag | Description |
|---|---|
| `--memory-id <MEMORY_ID>` | Memory's prefixed public identifier. |
| `--path <PATH>` | A path inside the memory (`/decisions/everruns-5528.md`). |

Example:

```bash
# See every version of one decision
everruns memories history mem_01h9 --path /decisions/everruns-5528.md
```

## memories sync

Queue an immediate sync for a source-backed workspace memory.

```bash
everruns memories sync [OPTIONS] --memory-id <memory_id>
```

| Flag | Description |
|---|---|
| `--memory-id <MEMORY_ID>` | Required. Workspace memory's prefixed public identifier. |

Example:

```bash
# Pull the latest source changes without waiting for the schedule
everruns memories sync --memory-id mem_01h9 --reason 'Docs just changed'
```

## memories update

Update a workspace memory.

```bash
everruns memories update [OPTIONS] --memory-id <memory_id>
```

| Flag | Description |
|---|---|
| `--description <DESCRIPTION>` | Human-readable description. |
| `--memory-id <MEMORY_ID>` | Required. Workspace memory's prefixed public identifier. |
| `--name <NAME>` | Human-readable name. |
| `--source <SOURCE>` |  |

Example:

```bash
# Rename a memory or change its description
everruns memories update --memory-id mem_01h9 --description 'Design docs, main branch' --reason 'Clarify scope'
```

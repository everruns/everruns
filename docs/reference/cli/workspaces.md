---
title: everruns workspaces
description: "Durable working areas holding the files agents work on. CLI reference for everruns workspaces."
sidebar:
  label: workspaces
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Durable working areas holding the files agents work on.

| Command | What it does |
|---|---|
| [`workspaces create`](#workspaces-create) | Create a new Workspace in the current organization. |
| [`workspaces delete`](#workspaces-delete) | Archive a Workspace (soft-delete). |
| [`workspaces get`](#workspaces-get) | Fetch a Workspace by its public ID. |
| [`workspaces list`](#workspaces-list) | List Workspaces in the current organization. |
| [`workspaces update`](#workspaces-update) | Update a Workspace's mutable fields. |

## workspaces create

Create a new Workspace in the current organization.

```bash
everruns workspaces create [OPTIONS] --name <name>
```

| Flag | Description |
|---|---|
| `--description <DESCRIPTION>` | Optional human-readable description. |
| `--name <NAME>` | Required. Human-readable workspace name. |

Example:

```bash
# Create a workspace to group related sessions and files
everruns workspaces create --name platform-team --description 'Platform team sandbox work' --reason 'Separate team resources'
```

## workspaces delete

Archive a Workspace (soft-delete).

```bash
everruns workspaces delete [OPTIONS] [WORKSPACE_ID]
```

| Flag | Description |
|---|---|
| `--workspace-id <WORKSPACE_ID>` | Workspace ID (wsp_<32-hex>). |

Example:

```bash
# Archive a workspace that is no longer used
everruns workspaces delete wsp_01h9 --reason 'Project finished'
```

## workspaces get

Fetch a Workspace by its public ID.

```bash
everruns workspaces get [OPTIONS] [WORKSPACE_ID]
```

| Flag | Description |
|---|---|
| `--workspace-id <WORKSPACE_ID>` | Workspace ID (wsp_<32-hex>). |

Example:

```bash
# Check a workspace's name and status
everruns workspaces get wsp_01h9
```

## workspaces list

List Workspaces in the current organization.

```bash
everruns workspaces list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--include-archived` | Also return archived items. |
| `--search <SEARCH>` | Case-insensitive substring match on name or description. |

Example:

```bash
# Find a workspace by name when you do not know the id
everruns workspaces list --search platform
```

## workspaces update

Update a Workspace's mutable fields.

```bash
everruns workspaces update [OPTIONS] --workspace-id <workspace_id>
```

| Flag | Description |
|---|---|
| `--description <DESCRIPTION>` |  |
| `--name <NAME>` |  |
| `--status <STATUS>` |  |
| `--workspace-id <WORKSPACE_ID>` | Required. Workspace ID (wsp_<32-hex>). |

Example:

```bash
# Rename a workspace
everruns workspaces update --workspace-id wsp_01h9 --name platform-core --reason 'Team renamed'
```

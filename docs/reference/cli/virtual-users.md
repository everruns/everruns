---
title: everruns virtual-users
description: "Runtime accounts for consumers or services. CLI reference for everruns virtual-users."
sidebar:
  label: virtual-users
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Runtime accounts for consumers or services.

| Command | What it does |
|---|---|
| [`virtual-users create`](#virtual-users-create) | Create a new virtual user (org-scoped runtime account for a consumer or service). |
| [`virtual-users delete`](#virtual-users-delete) | Archive a virtual user (soft delete). |
| [`virtual-users destroy`](#virtual-users-destroy) | Permanently delete a virtual user. |
| [`virtual-users get`](#virtual-users-get) | Get a single virtual user by ID. |
| [`virtual-users list`](#virtual-users-list) | List virtual users by usage and search. |
| [`virtual-users update`](#virtual-users-update) | Update a virtual user. |

## virtual-users create

Create a new virtual user (org-scoped runtime account for a consumer or service).

```bash
everruns virtual-users create [OPTIONS] --name <name>
```

| Flag | Description |
|---|---|
| `--avatar-url <AVATAR_URL>` | Profile image URL. |
| `--description <DESCRIPTION>` | Human-readable description. |
| `--locale <LOCALE>` | Locale used for agent-facing defaults. |
| `--name <NAME>` | Required. Human-readable name. |
| `--timezone <TIMEZONE>` | IANA time zone used for agent-facing defaults. |
| `--usage <USAGE>` | VirtualUser is a durable virtual principal. One of `end_user`, `service`. |

Example:

```bash
# Create a service account for unattended agent runs
everruns virtual-users create --name ops-bot --usage service --description 'Runs the nightly triage' --reason 'Dedicated identity for nightly triage'
```

## virtual-users delete

Archive a virtual user (soft delete). Can be restored.

```bash
everruns virtual-users delete [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Archive a virtual user that is no longer needed; it can be restored
everruns virtual-users delete identity_01h9 --reason 'Integration decommissioned'
```

## virtual-users destroy

Permanently delete a virtual user.

```bash
everruns virtual-users destroy [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Permanently remove a virtual user
everruns virtual-users destroy identity_01h9 --reason 'Integration decommissioned'
```

## virtual-users get

Get a single virtual user by ID.

```bash
everruns virtual-users get [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Show a virtual user's profile and status
everruns virtual-users get identity_01h9
```

## virtual-users list

List virtual users by usage and search. Supports pagination (limit/offset) and include_archived.

```bash
everruns virtual-users list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--include-archived` | Include archived runtime accounts. |
| `--limit <LIMIT>` | Maximum results per page, capped at the platform pagination limit. |
| `--offset <OFFSET>` | Zero-based page offset. |
| `--search <SEARCH>` | Search runtime names and descriptions. |
| `--usage <USAGE>` |  |

Example:

```bash
# Find service accounts by name
everruns virtual-users list --usage service --search ops
```

## virtual-users update

Update a virtual user. Only provided fields are changed.

```bash
everruns virtual-users update [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |
| `--avatar-url <AVATAR_URL>` | Profile image URL. |
| `--description <DESCRIPTION>` | Human-readable description. |
| `--locale <LOCALE>` | Locale used for agent-facing defaults. |
| `--name <NAME>` | Human-readable name. |
| `--status <STATUS>` |  |
| `--timezone <TIMEZONE>` | IANA time zone used for agent-facing defaults. |

Example:

```bash
# Change a virtual user's display name
everruns virtual-users update identity_01h9 --name triage-bot --reason 'Match the team naming'
```

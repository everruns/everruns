---
title: everruns users
description: "People in the current organization. CLI reference for everruns users: usage, flags and examples."
sidebar:
  label: users
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

People in the current organization.

| Command | What it does |
|---|---|
| [`users list`](#users-list) | List users in the current organization. |
| [`users me update`](#users-me-update) | Update your own account display name. |
| [`users profile get`](#users-profile-get) | Read your own editable profile, without email, tokens or credentials. |

## users list

List users in the current organization. Supports search filtering.

```bash
everruns users list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--search <SEARCH>` | Only users whose name or email matches this text. |

Example:

```bash
# Find a user by name or email in the current organization
everruns users list --search alice
```

## users me update

Update your own account display name.

```bash
everruns users me update [OPTIONS] --id <id> --name <name>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. |
| `--name <NAME>` | Required. |

Example:

```bash
# Update your own display name across organizations
everruns users me update --id 00000000-0000-0000-0000-000000000001 --name 'Alex Rivera' --reason 'Use my preferred name'
```

## users profile get

Read your own editable profile, without email, tokens or credentials.

```bash
everruns users profile get [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. |

Example:

```bash
# Read your own account display name independently of the active organization
everruns users profile get --id 00000000-0000-0000-0000-000000000001
```

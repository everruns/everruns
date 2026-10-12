---
title: everruns users
description: "People in the current organization. CLI reference for everruns users."
sidebar:
  label: users
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

People in the current organization.

| Command | What it does |
|---|---|
| [`users list`](#users-list) | List users in the current organization. |

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

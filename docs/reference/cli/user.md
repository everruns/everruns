---
title: everruns user
description: "Your own account settings. CLI reference for everruns user."
sidebar:
  label: user
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Your own account settings.

| Command | What it does |
|---|---|
| [`user connections list`](#user-connections-list) | List sanitized connection state for the current user. |
| [`user connections providers list`](#user-connections-providers-list) | List connection providers available in the current organization. |

## user connections list

List sanitized connection state for the current user. Returns provider identity and connection metadata, never credentials or tokens.

```bash
everruns user connections list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--provider <PROVIDER>` | Optional provider ID filter. |

Example:

```bash
# Check which accounts you have connected
everruns user connections list --provider github
```

## user connections providers list

List connection providers available in the current organization. This reports provider availability, not whether the current user is connected.

```bash
everruns user connections providers list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--search <SEARCH>` | Optional case-insensitive provider name/ID filter. |

Example:

```bash
# See which services you can connect an account to
everruns user connections providers list --search github
```

---
title: everruns organization-accounts
description: "Organization account names, without retrieving credentials. CLI reference for everruns organization-accounts: usage, flags and examples."
sidebar:
  label: organization-accounts
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Organization account names, without retrieving credentials.

| Command | What it does |
|---|---|
| [`organization-accounts get`](#organization-accounts-get) | Read an organization account's editable name. |
| [`organization-accounts update`](#organization-accounts-update) | Rename an organization account while preserving its current credentials. |

## organization-accounts get

Read an organization account's editable name. Credentials are never returned.

```bash
everruns organization-accounts get [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. |

Example:

```bash
# Read an organization account name without retrieving its credentials
everruns organization-accounts get --id 00000000-0000-0000-0000-000000000001
```

## organization-accounts update

Rename an organization account while preserving its current credentials.

```bash
everruns organization-accounts update [OPTIONS] --id <id> --name <name>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. |
| `--name <NAME>` | Required. |

Example:

```bash
# Rename a connected account while preserving its credential
everruns organization-accounts update --id 00000000-0000-0000-0000-000000000001 --name 'Northstar sandbox' --reason 'Clarify the account purpose'
```

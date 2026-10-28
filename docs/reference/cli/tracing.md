---
title: everruns tracing
description: "Customer tracing destinations and routing. CLI reference for everruns tracing: usage, flags and examples."
sidebar:
  label: tracing
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Customer tracing destinations and routing.

| Command | What it does |
|---|---|
| [`tracing destinations get`](#tracing-destinations-get) | Read safe destination metadata. |
| [`tracing destinations update`](#tracing-destinations-update) | Update safe destination metadata while preserving its receiver and current authentication. |
| [`tracing routing get`](#tracing-routing-get) | Read organization or agent trace routing without credentials. |
| [`tracing routing update`](#tracing-routing-update) | Set organization or agent destinations for future turns. |

## tracing destinations get

Read safe destination metadata. Never returns receiver authentication.

```bash
everruns tracing destinations get [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. |

Example:

```bash
# Inspect a destination without credentials
everruns tracing destinations get --id 00000000-0000-0000-0000-000000000001
```

## tracing destinations update

Update safe destination metadata while preserving its receiver and current authentication.

```bash
everruns tracing destinations update [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--enabled` |  |
| `--id <ID>` | Required. |
| `--name <NAME>` |  |

Example:

```bash
# Pause external exports
everruns tracing destinations update --id 00000000-0000-0000-0000-000000000001 --enabled false --reason 'Pause exports'
```

## tracing routing get

Read organization or agent trace routing without credentials.

```bash
everruns tracing routing get [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. |

Example:

```bash
# Read an agent's external tracing policy
everruns tracing routing get --id agent_01933b5a000070008000000000000001
```

## tracing routing update

Set organization or agent destinations for future turns. Requires organization settings authority; never accepts credentials.

```bash
everruns tracing routing update [OPTIONS] --destination-ids <destination_ids> --id <id> --mode <mode>
```

| Flag | Description |
|---|---|
| `--destination-ids <DESTINATION_IDS>` | Required. Comma-separated or a JSON array of strings. Repeatable. |
| `--expected-revision <EXPECTED_REVISION>` |  |
| `--id <ID>` | Required. |
| `--mode <MODE>` | Required. One of `inherit`, `add`, `replace`, `off`. |

Example:

```bash
# Stop an agent's external trace exports
everruns tracing routing update --id agent_01933b5a000070008000000000000001 --mode off --destination-ids '[]' --reason 'Disable external exports'
```

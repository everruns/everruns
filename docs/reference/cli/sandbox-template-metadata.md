---
title: everruns sandbox-template-metadata
description: "Sandbox Template names and descriptions, without execution settings. CLI reference for everruns sandbox-template-metadata: usage, flags and examples."
sidebar:
  label: sandbox-template-metadata
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Sandbox Template names and descriptions, without execution settings.

| Command | What it does |
|---|---|
| [`sandbox-template-metadata get`](#sandbox-template-metadata-get) | Read editable Sandbox Template metadata, excluding execution configuration and environment values. |
| [`sandbox-template-metadata update`](#sandbox-template-metadata-update) | Update Sandbox Template metadata while preserving its current execution specification. |

## sandbox-template-metadata get

Read editable Sandbox Template metadata, excluding execution configuration and environment values.

```bash
everruns sandbox-template-metadata get [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. Prefixed identifier with 'sbxtpl' prefix. |

Example:

```bash
# Inspect a template display name without reading its execution configuration
everruns sandbox-template-metadata get --id sbxtpl_01h9
```

## sandbox-template-metadata update

Update Sandbox Template metadata while preserving its current execution specification.

```bash
everruns sandbox-template-metadata update [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--description <DESCRIPTION>` |  |
| `--display-name <DISPLAY_NAME>` |  |
| `--id <ID>` | Required. Prefixed identifier with 'sbxtpl' prefix. |

Example:

```bash
# Clarify a template label while preserving the current sandbox specification
everruns sandbox-template-metadata update --id sbxtpl_01h9 --display-name 'Northstar review sandbox' --reason 'Clarify the template purpose'
```

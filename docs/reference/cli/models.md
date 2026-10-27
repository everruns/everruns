---
title: everruns models
description: "LLM models available to agents, and organization defaults. CLI reference for everruns models: usage, flags and examples."
sidebar:
  label: models
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

LLM models available to agents, and organization defaults.

| Command | What it does |
|---|---|
| [`models delete`](#models-delete) | Delete a model. |
| [`models get`](#models-get) | Get a specific model with provider information. |
| [`models list`](#models-list) | List all models across all providers. |
| [`models update`](#models-update) | Update a model. |
| [`models decision-default get`](#models-decision-default-get) | Get the selected decision model. |
| [`models decision-default set`](#models-decision-default-set) | Set the selected decision model. |
| [`models default get`](#models-default-get) | Get the effective organization default model. |
| [`models selection apply`](#models-selection-apply) | Atomically apply reviewed enablement changes. |
| [`models selection undo`](#models-selection-undo) | Atomically undo selected reviewed model enablement changes. |

## models delete

Delete a model.

```bash
everruns models delete [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Remove a model that was registered by mistake
everruns models delete model_01h9 --reason 'Duplicate of an existing entry'
```

## models get

Get a specific model with provider information.

```bash
everruns models get [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Show a model with its provider and capabilities
everruns models get model_01h9
```

## models list

List all models across all providers.

```bash
everruns models list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--favorites-only` | Only models marked as favorites. |
| `--include-stale` | Include models the provider no longer lists. |
| `--service <SERVICE>` |  |
| `--source <SOURCE>` |  |

Example:

```bash
# Find the models you have starred
everruns models list --favorites-only true
```

## models update

Update a model.

```bash
everruns models update [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--capabilities <CAPABILITIES>` | Replacement capability labels. Comma-separated or a JSON array of strings. Repeatable. |
| `--display-name <DISPLAY_NAME>` | Human-readable display name. |
| `--enabled` | Whether this resource is enabled. |
| `--id <ID>` | Required. Prefixed public identifier. |
| `--is-favorite` | Show the model first in pickers. |
| `--model-id <MODEL_ID>` | LLM model's prefixed public identifier. |
| `--profile-key <PROFILE_KEY>` | Explicitly reassign the stable profile; preference-only edits preserve it. |
| `--provider-id <PROVIDER_ID>` | LLM provider's prefixed public identifier. |
| `--service <SERVICE>` |  |

Example:

```bash
# Star a model so it appears first in pickers
everruns models update --id model_01h9 --is-favorite true --reason 'Team default'
```

## models decision-default get

Get the selected decision model.

```bash
everruns models decision-default get [OPTIONS]
```

Example:

```bash
# Check which model handles typed decisions
everruns models decision-default get
```

## models decision-default set

Set the selected decision model.

```bash
everruns models decision-default set [OPTIONS]
```

| Flag | Description |
|---|---|
| `--model-id <MODEL_ID>` | Prefixed saved model ID; omit or pass null to clear the default. |

Example:

```bash
# Choose the model that handles typed decisions
everruns models decision-default set --model-id model_01h9 --reason 'Cheaper model is accurate enough'
```

## models default get

Get the effective organization default model.

```bash
everruns models default get [OPTIONS]
```

Example:

```bash
# Check which model new sessions use when none is chosen
everruns models default get
```

## models selection apply

Atomically apply reviewed enablement changes. Rejects a model whose provider or enabled state changed while the picker was open.

```bash
everruns models selection apply [OPTIONS] --changes <changes>
```

| Flag | Description |
|---|---|
| `--changes <CHANGES>` | Required. |

Example:

```bash
# Enable a reviewed model only if its provider and current selection still match
everruns models selection apply --changes '[{"id":"model_01h9","provider_id":"provider_01h9","expected_enabled":false,"enabled":true}]' --reason 'Make the reviewed model selectable'
```

## models selection undo

Atomically undo selected reviewed model enablement changes. Every model must still have its expected revision.

```bash
everruns models selection undo [OPTIONS] --changes <changes>
```

| Flag | Description |
|---|---|
| `--changes <CHANGES>` | Required. |

Example:

```bash
# Undo reviewed enablement only while the saved revision is still current
everruns models selection undo --changes '[{"id":"model_01h9","from_revision":1,"expected_revision":2}]' --reason 'Restore the reviewed selection'
```

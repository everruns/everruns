---
title: everruns models
description: "LLM models available to agents, and organization defaults. CLI reference for everruns models."
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
| `--capabilities <CAPABILITIES>` | Replacement capability labels. Repeatable. |
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

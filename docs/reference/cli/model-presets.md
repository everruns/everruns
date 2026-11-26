---
title: everruns model-presets
description: "Named sets of models, one per job, that agents can use instead of the organization defaults. CLI reference for everruns model-presets: usage, flags and examples."
sidebar:
  label: model-presets
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Named sets of models, one per job, that agents can use instead of the organization defaults.

| Command | What it does |
|---|---|
| [`model-presets apply`](#model-presets-apply) | Copy a preset's models into the organization defaults. |
| [`model-presets create`](#model-presets-create) | Create a custom model preset, blank or copied from another preset with `from`. |
| [`model-presets delete`](#model-presets-delete) | Delete a custom model preset. |
| [`model-presets get`](#model-presets-get) | Get one model preset with its model per job and the jobs that use a fallback model. |
| [`model-presets list`](#model-presets-list) | List model presets: built-in Automatic, one per connected provider, and custom presets, each with its model per job. |
| [`model-presets update`](#model-presets-update) | Rename a custom model preset or change its model per job; null returns a job to the organization default. |

## model-presets apply

Copy a preset's models into the organization defaults. Jobs the preset leaves empty keep their current default.

```bash
everruns model-presets apply [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Preset id to copy into the organization defaults. |

Output: one JSON object. Read fields with jq; nested settings stay nested, such as '.config.target', and a list sits in one named field.

Example:

```bash
# Make Gemini's picks the organization defaults
everruns model-presets apply provider:gemini --reason 'Move the org to Gemini'
```

## model-presets create

Create a custom model preset, blank or copied from another preset with `from`.

```bash
everruns model-presets create [OPTIONS] --name <name>
```

| Flag | Description |
|---|---|
| `--chat-model-id <CHAT_MODEL_ID>` | Model for the built-in Everruns Chat. |
| `--coding-model-id <CODING_MODEL_ID>` | Model for coding-harness agents and coding helpers. |
| `--decision-model-id <DECISION_MODEL_ID>` | Model for quick gate checks and the Auto helper pick. |
| `--default-model-id <DEFAULT_MODEL_ID>` | Model for agents and sessions with no model of their own. |
| `--description <DESCRIPTION>` | What the preset is for. |
| `--from <FROM>` | Preset to copy jobs from (`automatic`, `provider:gemini` or a custom id). |
| `--name <NAME>` | Required. Display name, 1 to 100 characters. |
| `--research-model-id <RESEARCH_MODEL_ID>` | Model for research-role agents and deep helpers. |
| `--small-model-id <SMALL_MODEL_ID>` | Model for titles, summaries and fast helpers. |

Output: one JSON object. Read fields with jq; nested settings stay nested, such as '.config.target', and a list sits in one named field.

Example:

```bash
# Start a custom preset from the Gemini picks
everruns model-presets create --name 'Cheap council' --from provider:gemini --reason 'Cheaper helpers for review agents'
```

## model-presets delete

Delete a custom model preset. Refused while agents take their models from it.

```bash
everruns model-presets delete [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. Custom preset id. |

Example:

```bash
# Remove a preset no agent uses any more
everruns model-presets delete --id cheap-council --reason 'Council experiment finished'
```

## model-presets get

Get one model preset with its model per job and the jobs that use a fallback model.

```bash
everruns model-presets get [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Preset id: `automatic`, `provider:<driver>` or a custom preset id. |

Output: one JSON object. Read fields with jq; nested settings stay nested, such as '.config.target', and a list sits in one named field.

Example:

```bash
# Check which Gemini models the Gemini preset picks
everruns model-presets get provider:gemini
```

## model-presets list

List model presets: built-in Automatic, one per connected provider, and custom presets, each with its model per job.

```bash
everruns model-presets list [OPTIONS]
```

Output: a JSON array. Read the items with jq '.[]'.

Example:

```bash
# See which model each preset uses for coding, research and small work
everruns model-presets list
```

## model-presets update

Rename a custom model preset or change its model per job; null returns a job to the organization default.

```bash
everruns model-presets update [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--chat-model-id <CHAT_MODEL_ID>` | Model for the built-in Everruns Chat. |
| `--coding-model-id <CODING_MODEL_ID>` | Model for coding-harness agents and coding helpers. |
| `--decision-model-id <DECISION_MODEL_ID>` | Model for quick gate checks and the Auto helper pick. |
| `--default-model-id <DEFAULT_MODEL_ID>` | Model for agents and sessions with no model of their own. |
| `--description <DESCRIPTION>` | What the preset is for. |
| `--id <ID>` | Required. Custom preset id. |
| `--name <NAME>` | Display name. |
| `--research-model-id <RESEARCH_MODEL_ID>` | Model for research-role agents and deep helpers. |
| `--small-model-id <SMALL_MODEL_ID>` | Model for titles, summaries and fast helpers. |

Output: one JSON object. Read fields with jq; nested settings stay nested, such as '.config.target', and a list sits in one named field.

Example:

```bash
# Use a stronger research model in a custom preset
everruns model-presets update --id cheap-council --research-model-id model_01h9 --reason 'Research answers were too shallow'
```

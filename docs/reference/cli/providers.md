---
title: everruns providers
description: "LLM providers and their credentials. CLI reference for everruns providers."
sidebar:
  label: providers
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

LLM providers and their credentials.

| Command | What it does |
|---|---|
| [`providers create`](#providers-create) | Create a new LLM provider. |
| [`providers delete`](#providers-delete) | Delete an LLM provider. |
| [`providers get`](#providers-get) | Get a specific LLM provider. |
| [`providers list`](#providers-list) | List all LLM providers. |
| [`providers update`](#providers-update) | Update an LLM provider. |
| [`providers check-credentials check`](#providers-check-credentials-check) | Check a provider API key without storing it. |
| [`providers models create`](#providers-models-create) | Create a new model for a provider. |
| [`providers models list`](#providers-models-list) | List models for a specific provider. |
| [`providers models review`](#providers-models-review) | Mark a provider's discovered models as reviewed, clearing their new flag. |
| [`providers sync-models sync`](#providers-sync-models-sync) | Discover and sync models from a provider. |

## providers create

Create a new LLM provider.

```bash
everruns providers create [OPTIONS] --name <name> --provider-type <provider_type>
```

| Flag | Description |
|---|---|
| `--api-key <API_KEY>` | Credential for the provider. |
| `--base-url <BASE_URL>` | Custom API endpoint. |
| `--name <NAME>` | Required. Human-readable name. |
| `--provider-type <PROVIDER_TYPE>` | Required. LLM provider type. |
| `--request-options <REQUEST_OPTIONS>` |  |
| `--trace <TRACE>` |  |

Example:

```bash
# Connect an LLM provider with its API key
everruns providers create --name openai-prod --provider-type openai --api-key "$OPENAI_API_KEY" --reason 'Enable OpenAI models'
```

## providers delete

Delete an LLM provider.

```bash
everruns providers delete [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Remove a provider and stop using its models
everruns providers delete provider_01h9 --reason 'Account closed'
```

## providers get

Get a specific LLM provider.

```bash
everruns providers get [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Show a provider's type and status
everruns providers get provider_01h9
```

## providers list

List all LLM providers.

```bash
everruns providers list [OPTIONS]
```

Example:

```bash
# See which LLM providers are connected
everruns providers list
```

## providers update

Update an LLM provider.

```bash
everruns providers update [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--api-key <API_KEY>` | Replacement credential. |
| `--base-url <BASE_URL>` | Replacement API endpoint. |
| `--id <ID>` | Required. Prefixed public identifier. |
| `--name <NAME>` | Human-readable name. |
| `--provider-type <PROVIDER_TYPE>` |  |
| `--request-options <REQUEST_OPTIONS>` |  |
| `--status <STATUS>` |  |
| `--trace <TRACE>` |  |

Example:

```bash
# Rotate a provider's API key
everruns providers update --id provider_01h9 --api-key "$OPENAI_API_KEY" --reason 'Scheduled key rotation'
```

## providers check-credentials check

Check a provider API key without storing it.

```bash
everruns providers check-credentials check [OPTIONS] --api-key <api_key> --provider-type <provider_type>
```

| Flag | Description |
|---|---|
| `--api-key <API_KEY>` | Required. The credential to probe. |
| `--base-url <BASE_URL>` | Optional custom endpoint. |
| `--provider-type <PROVIDER_TYPE>` | Required. LLM provider type. |

Example:

```bash
# Verify an API key works before saving a provider
everruns providers check-credentials check --provider-type openai --api-key "$OPENAI_API_KEY"
```

## providers models create

Create a new model for a provider.

```bash
everruns providers models create [OPTIONS] --display-name <display_name> --model-id <model_id> --provider-id <provider_id>
```

| Flag | Description |
|---|---|
| `--capabilities <CAPABILITIES>` | Capability labels for the model. Repeatable. |
| `--display-name <DISPLAY_NAME>` | Required. Human-readable display name. |
| `--enabled` | Whether this resource is enabled. |
| `--is-favorite` | Show the model first in pickers. |
| `--model-id <MODEL_ID>` | Required. LLM model's prefixed public identifier. |
| `--profile-key <PROFILE_KEY>` | Stable model profile to assign to the model. |
| `--provider-id <PROVIDER_ID>` | Required. LLM provider's prefixed public identifier. |
| `--service <SERVICE>` |  |

Example:

```bash
# Register a model that provider discovery does not list
everruns providers models create --provider-id provider_01h9 --model-id gpt-5.1 --display-name 'GPT-5.1' --enabled true --reason 'Make the new model selectable'
```

## providers models list

List models for a specific provider.

```bash
everruns providers models list [OPTIONS] --provider-id <provider_id>
```

| Flag | Description |
|---|---|
| `--provider-id <PROVIDER_ID>` | Required. LLM provider's prefixed public identifier. |

Example:

```bash
# See which models one provider offers
everruns providers models list --provider-id provider_01h9
```

## providers models review

Mark a provider's discovered models as reviewed, clearing their new flag.

```bash
everruns providers models review [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. Prefixed public identifier. |

Example:

```bash
# Clear the new flag once you have looked at a provider's newly discovered models
everruns providers models review --id provider_01h9 --reason 'Checked new models'
```

## providers sync-models sync

Discover and sync models from a provider.

```bash
everruns providers sync-models sync [OPTIONS] --id <id>
```

| Flag | Description |
|---|---|
| `--id <ID>` | Required. Prefixed public identifier. |

Example:

```bash
# Pull a provider's current model list after it ships new models
everruns providers sync-models sync --id provider_01h9 --reason 'New models released'
```

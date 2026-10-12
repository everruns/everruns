---
title: everruns knowledge-indexes
description: "Searchable indexes synced from external sources. CLI reference for everruns knowledge-indexes."
sidebar:
  label: knowledge-indexes
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Searchable indexes synced from external sources.

| Command | What it does |
|---|---|
| [`knowledge-indexes create`](#knowledge-indexes-create) | Create a knowledge index in the current organization. |
| [`knowledge-indexes delete`](#knowledge-indexes-delete) | Archive a knowledge index. |
| [`knowledge-indexes get`](#knowledge-indexes-get) | Get a knowledge index by ID. |
| [`knowledge-indexes list`](#knowledge-indexes-list) | List knowledge indexes in the current organization. |
| [`knowledge-indexes sync`](#knowledge-indexes-sync) | Enqueue a manual sync of a knowledge index. |
| [`knowledge-indexes update`](#knowledge-indexes-update) | Update a knowledge index. |
| [`knowledge-indexes documents list`](#knowledge-indexes-documents-list) | List documents inside a knowledge index. |

## knowledge-indexes create

Create a knowledge index in the current organization.

```bash
everruns knowledge-indexes create [OPTIONS] --embedding-model-id <embedding_model_id> --name <name>
```

| Flag | Description |
|---|---|
| `--description <DESCRIPTION>` | Human-readable description. |
| `--embedding-model-id <EMBEDDING_MODEL_ID>` | Required. Embedding model used to embed chunks. |
| `--name <NAME>` | Required. Human-readable name. |
| `--source-config <SOURCE_CONFIG>` | Non-secret source coordinates. |
| `--source-type <SOURCE_TYPE>` | External source type. |

Example:

```bash
# Index a GitHub repository so agents can search its docs
everruns knowledge-indexes create --name product-docs --embedding-model-id model_01h9 --source-type github --source-config '{"provider":"github","repository":"acme/docs"}' --reason 'Give support agents the product docs'
```

## knowledge-indexes delete

Archive a knowledge index.

```bash
everruns knowledge-indexes delete [OPTIONS] --index-id <index_id>
```

| Flag | Description |
|---|---|
| `--index-id <INDEX_ID>` | Required. Knowledge index's prefixed public identifier. |

Example:

```bash
# Archive a knowledge index that agents no longer need
everruns knowledge-indexes delete --index-id kidx_01h9 --reason 'Docs moved to a new index'
```

## knowledge-indexes get

Get a knowledge index by ID.

```bash
everruns knowledge-indexes get [OPTIONS] [INDEX_ID]
```

| Flag | Description |
|---|---|
| `--index-id <INDEX_ID>` | Knowledge index's prefixed public identifier. |

Example:

```bash
# Check a knowledge index's source and sync state
everruns knowledge-indexes get kidx_01h9
```

## knowledge-indexes list

List knowledge indexes in the current organization.

```bash
everruns knowledge-indexes list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--include-archived` | Also return archived items. |
| `--search <SEARCH>` | Case-insensitive substring match on name or description. |

Example:

```bash
# Find a knowledge index by name when you do not know the id
everruns knowledge-indexes list --search docs
```

## knowledge-indexes sync

Enqueue a manual sync of a knowledge index.

```bash
everruns knowledge-indexes sync [OPTIONS] [INDEX_ID]
```

| Flag | Description |
|---|---|
| `--index-id <INDEX_ID>` | Knowledge index's prefixed public identifier. |

Example:

```bash
# Re-index now after the source repository changed
everruns knowledge-indexes sync kidx_01h9 --reason 'Docs updated upstream'
```

## knowledge-indexes update

Update a knowledge index.

```bash
everruns knowledge-indexes update [OPTIONS] --index-id <index_id>
```

| Flag | Description |
|---|---|
| `--description <DESCRIPTION>` | Human-readable description. |
| `--embedding-model-id <EMBEDDING_MODEL_ID>` | Embedding model used to embed chunks. |
| `--index-id <INDEX_ID>` | Required. Knowledge index's prefixed public identifier. |
| `--name <NAME>` | Human-readable name. |
| `--source-config <SOURCE_CONFIG>` | Non-secret source coordinates. |

Example:

```bash
# Point an index at a different branch or folder
everruns knowledge-indexes update --index-id kidx_01h9 --source-config '{"provider":"github","repository":"acme/docs","branch":"main"}' --reason 'Track main instead of the release branch'
```

## knowledge-indexes documents list

List documents inside a knowledge index.

```bash
everruns knowledge-indexes documents list [OPTIONS] [INDEX_ID]
```

| Flag | Description |
|---|---|
| `--index-id <INDEX_ID>` | Knowledge index's prefixed public identifier. |

Example:

```bash
# See which documents a sync has indexed
everruns knowledge-indexes documents list kidx_01h9
```

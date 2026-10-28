---
title: everruns knowledge
description: "File collections with optional indexed retrieval. CLI reference for everruns knowledge: usage, flags and examples."
sidebar:
  label: knowledge
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

File collections with optional indexed retrieval.

| Command | What it does |
|---|---|
| [`knowledge delete`](#knowledge-delete) | Archive a knowledge collection. |
| [`knowledge create`](#knowledge-create) | Create a knowledge collection; indexing is optional. |
| [`knowledge get`](#knowledge-get) | Inspect a knowledge collection, sources and processing status. |
| [`knowledge list`](#knowledge-list) | List versioned knowledge collections. |
| [`knowledge search`](#knowledge-search) | Search selected knowledge collections with immutable citations. |
| [`knowledge sync`](#knowledge-sync) | Queue source refresh and retry failed indexing jobs. |
| [`knowledge update`](#knowledge-update) | Configure knowledge indexing and exclusion rules. |
| [`knowledge files delete`](#knowledge-files-delete) | Delete a managed knowledge file and stop serving its search results. |
| [`knowledge files list`](#knowledge-files-list) | List knowledge files with bounded cursor pagination. |
| [`knowledge files update`](#knowledge-files-update) | Update annotations or file read/index policy without extracting content again. |
| [`knowledge files write`](#knowledge-files-write) | Add or update an original file in managed knowledge. |
| [`knowledge sources add`](#knowledge-sources-add) | Connect an existing GitHub identity or public Git source to knowledge. |
| [`knowledge indexers create`](#knowledge-indexers-create) | Configure an organization Turbopuffer account using an existing write-only credential binding. |
| [`knowledge indexers list`](#knowledge-indexers-list) | List organization search-provider accounts without credential values. |
| [`knowledge okf validate`](#knowledge-okf-validate) | Validate a page of OKF v0.2 metadata and reserved navigation files without converting originals. |

## knowledge delete

Archive a knowledge collection.

```bash
everruns knowledge delete [OPTIONS] --collection-id <collection_id>
```

| Flag | Description |
|---|---|
| `--collection-id <COLLECTION_ID>` | Required. |

Example:

```bash
# Remove a collection from active use
everruns knowledge delete --collection-id kn_01950000000070008000000000000001
```

## knowledge create

Create a knowledge collection; indexing is optional.

```bash
everruns knowledge create [OPTIONS] --name <name>
```

| Flag | Description |
|---|---|
| `--description <DESCRIPTION>` | Human-readable purpose of the collection, available during progressive disclosure. |
| `--embedding-model-id <EMBEDDING_MODEL_ID>` | Organization embedding model; required when hybrid indexing is enabled. |
| `--indexer-id <INDEXER_ID>` | Optional organization search account. |
| `--indexing <INDEXING>` | Optional collection indexing: no index, keyword retrieval, or keyword plus embeddings. One of `off`, `lexical`, `hybrid`. |
| `--name <NAME>` | Required. Collection name; `everruns-docs` is reserved for the predefined documentation snapshot. |
| `--rules <RULES>` | Path-glob arrays named `ingest_exclude`, `index_exclude` and `read_exclude`. |

Example:

```bash
# Create a collection without indexing
everruns knowledge create --name 'Support handbook' --indexing off
```

## knowledge get

Inspect a knowledge collection, sources and processing status.

```bash
everruns knowledge get [OPTIONS] --collection-id <collection_id>
```

| Flag | Description |
|---|---|
| `--collection-id <COLLECTION_ID>` | Required. |

Example:

```bash
# Inspect source and indexing freshness
everruns knowledge get --collection-id kn_01950000000070008000000000000001
```

## knowledge list

List versioned knowledge collections.

```bash
everruns knowledge list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--after <AFTER>` |  |
| `--limit <LIMIT>` |  |

Example:

```bash
# Find available Knowledge collections
everruns knowledge list --limit 10
```

## knowledge search

Search selected knowledge collections with immutable citations.

```bash
everruns knowledge search [OPTIONS] --query <query>
```

| Flag | Description |
|---|---|
| `--collections <COLLECTIONS>` | Optional collection identifiers. Comma-separated or a JSON array of strings. Repeatable. |
| `--lexical-only` | Use keyword retrieval without creating a query embedding. |
| `--limit <LIMIT>` | Requested maximum passages; hosts enforce the configured retrieval budget. |
| `--metadata <METADATA>` | Exact user-annotation predicates applied without granting additional file access. |
| `--path <PATH>` | Optional source-relative path prefix that narrows retrieval. |
| `--query <QUERY>` | Required. Search text used for keyword retrieval and, when enabled, query embeddings. |

Example:

```bash
# Retrieve cited policy passages
everruns knowledge search --collections kn_01950000000070008000000000000001 --query 'refund policy' --limit 5
```

## knowledge sync

Queue source refresh and retry failed indexing jobs.

```bash
everruns knowledge sync [OPTIONS] --collection-id <collection_id>
```

| Flag | Description |
|---|---|
| `--collection-id <COLLECTION_ID>` | Required. |

Example:

```bash
# Refresh connected sources incrementally
everruns knowledge sync --collection-id kn_01950000000070008000000000000001
```

## knowledge update

Configure knowledge indexing and exclusion rules.

```bash
everruns knowledge update [OPTIONS] --collection-id <collection_id> --name <name>
```

| Flag | Description |
|---|---|
| `--collection-id <COLLECTION_ID>` | Required. |
| `--description <DESCRIPTION>` | Human-readable purpose of the collection, available during progressive disclosure. |
| `--embedding-model-id <EMBEDDING_MODEL_ID>` | Organization embedding model; required when hybrid indexing is enabled. |
| `--indexer-id <INDEXER_ID>` | Optional organization search account. |
| `--indexing <INDEXING>` | Optional collection indexing: no index, keyword retrieval, or keyword plus embeddings. One of `off`, `lexical`, `hybrid`. |
| `--name <NAME>` | Required. Collection name; `everruns-docs` is reserved for the predefined documentation snapshot. |
| `--rules <RULES>` | Path-glob arrays named `ingest_exclude`, `index_exclude` and `read_exclude`. |

Example:

```bash
# Enable keyword indexing on retained files
everruns knowledge update --collection-id kn_01950000000070008000000000000001 --name 'Support handbook' --indexing lexical
```

## knowledge files delete

Delete a managed knowledge file and stop serving its search results.

```bash
everruns knowledge files delete [OPTIONS] --collection-id <collection_id> --path <path>
```

| Flag | Description |
|---|---|
| `--collection-id <COLLECTION_ID>` | Required. |
| `--path <PATH>` | Required. |

Example:

```bash
# Delete a managed file
everruns knowledge files delete --collection-id kn_01950000000070008000000000000001 --path /policies/obsolete.md
```

## knowledge files list

List knowledge files with bounded cursor pagination.

```bash
everruns knowledge files list [OPTIONS] --collection-id <collection_id>
```

| Flag | Description |
|---|---|
| `--after <AFTER>` |  |
| `--collection-id <COLLECTION_ID>` | Required. |
| `--extracted` |  |
| `--length <LENGTH>` |  |
| `--limit <LIMIT>` |  |
| `--offset <OFFSET>` |  |
| `--path <PATH>` |  |
| `--tree` | List only the immediate children of `path`: its files plus one entry per subfolder (`kind: "f... |

Example:

```bash
# Page through a collection folder
everruns knowledge files list --collection-id kn_01950000000070008000000000000001 --path /policies/ --limit 20
```

## knowledge files update

Update annotations or file read/index policy without extracting content again.

```bash
everruns knowledge files update [OPTIONS] --collection-id <collection_id> --path <path>
```

| Flag | Description |
|---|---|
| `--collection-id <COLLECTION_ID>` | Required. |
| `--indexable` | Optional indexing override. |
| `--metadata <METADATA>` | Replace user annotations while preserving metadata parsed from the original file. |
| `--path <PATH>` | Required. Existing collection-relative file path. |
| `--readable` | Optional agent-read override. |

Example:

```bash
# Exclude a file from indexing while retaining it
everruns knowledge files update --collection-id kn_01950000000070008000000000000001 --path /policies/draft.md --indexable false
```

## knowledge files write

Add or update an original file in managed knowledge.

```bash
everruns knowledge files write [OPTIONS] --collection-id <collection_id> --path <path>
```

| Flag | Description |
|---|---|
| `--collection-id <COLLECTION_ID>` | Required. |
| `--content <CONTENT>` | UTF-8 original content. |
| `--content-base64 <CONTENT_BASE64>` | Base64 original bytes for binary files; mutually exclusive with `content`. |
| `--indexable` | Optional per-file indexing policy; false retains the original without indexing it. |
| `--metadata <METADATA>` | User annotations retained separately from parsed source/frontmatter metadata. |
| `--mime-type <MIME_TYPE>` | Optional media type used for representation extraction; inferred when omitted. |
| `--path <PATH>` | Required. Normalized absolute path within the collection, preserving its directory hierarchy. |
| `--readable` | Optional agent-read policy. |

Example:

```bash
# Write a managed note preserving its path
everruns knowledge files write --collection-id kn_01950000000070008000000000000001 --path /policies/refunds.md --content 'Refunds are available for thirty days.'
```

## knowledge sources add

Connect an existing GitHub identity or public Git source to knowledge.

```bash
everruns knowledge sources add [OPTIONS] --collection-id <collection_id> --kind <kind>
```

| Flag | Description |
|---|---|
| `--collection-id <COLLECTION_ID>` | Required. |
| `--config <CONFIG>` | Adapter-specific repository, revision and root-folder configuration, without credentials. |
| `--connection-owner <CONNECTION_OWNER>` | Optional GitHub connection owner; existing connection authorization still applies. |
| `--kind <KIND>` | Required. Source adapter: `github` reuses a connection; `git` reads a public HTTPS repository. |
| `--root-path <ROOT_PATH>` | Collection-relative root under which the source hierarchy is preserved. |

Example:

```bash
# Attach an existing GitHub connection to a source root
everruns knowledge sources add --collection-id kn_01950000000070008000000000000001 --kind github --root-path /product --config '{"repository":"everruns/everruns","root_folder":"docs"}'
```

## knowledge indexers create

Configure an organization Turbopuffer account using an existing write-only credential binding.

```bash
everruns knowledge indexers create [OPTIONS] --credential-binding-id <credential_binding_id> --endpoint <endpoint> --name <name>
```

| Flag | Description |
|---|---|
| `--credential-binding-id <CREDENTIAL_BINDING_ID>` | Required. Active organization-service credential binding; never a plaintext API key. |
| `--endpoint <ENDPOINT>` | Required. HTTPS regional Turbopuffer API endpoint; embedded credentials are forbidden. |
| `--name <NAME>` | Required. Display name for the organization search account. |

Example:

```bash
# Use an organization service credential for Turbopuffer
everruns knowledge indexers create --name 'Search account' --endpoint https://gcp-us-central1.turbopuffer.com --credential-binding-id 01950000-0000-7000-8000-000000000002
```

## knowledge indexers list

List organization search-provider accounts without credential values.

```bash
everruns knowledge indexers list [OPTIONS]
```

Example:

```bash
# List organization search accounts without exposing credentials
everruns knowledge indexers list
```

## knowledge okf validate

Validate a page of OKF v0.2 metadata and reserved navigation files without converting originals.

```bash
everruns knowledge okf validate [OPTIONS] --collection-id <collection_id>
```

| Flag | Description |
|---|---|
| `--after <AFTER>` |  |
| `--collection-id <COLLECTION_ID>` | Required. |
| `--extracted` |  |
| `--length <LENGTH>` |  |
| `--limit <LIMIT>` |  |
| `--offset <OFFSET>` |  |
| `--path <PATH>` |  |
| `--tree` | List only the immediate children of `path`: its files plus one entry per subfolder (`kind: "f... |

Example:

```bash
# Validate a page of OKF metadata
everruns knowledge okf validate --collection-id kn_01950000000070008000000000000001 --limit 100
```

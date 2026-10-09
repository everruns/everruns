---
title: everruns knowledge-bases
description: "Curated collections of knowledge entries. CLI reference for everruns knowledge-bases."
sidebar:
  label: knowledge-bases
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Curated collections of knowledge entries.

| Command | What it does |
|---|---|
| [`knowledge-bases create`](#knowledge-bases-create) | Create a knowledge base in the current organization. |
| [`knowledge-bases delete`](#knowledge-bases-delete) | Archive a knowledge base. |
| [`knowledge-bases get`](#knowledge-bases-get) | Get a knowledge base by ID. |
| [`knowledge-bases list`](#knowledge-bases-list) | List knowledge bases in the current organization. |
| [`knowledge-bases update`](#knowledge-bases-update) | Update a knowledge base. |
| [`knowledge-bases entries create`](#knowledge-bases-entries-create) | Create an entry inside a knowledge base. |
| [`knowledge-bases entries delete`](#knowledge-bases-entries-delete) | Delete a knowledge entry. |
| [`knowledge-bases entries get`](#knowledge-bases-entries-get) | Get a knowledge entry by ID. |
| [`knowledge-bases entries list`](#knowledge-bases-entries-list) | List entries inside a knowledge base. |
| [`knowledge-bases entries update`](#knowledge-bases-entries-update) | Update a knowledge entry. |
| [`knowledge-bases okf-import import`](#knowledge-bases-okf-import-import) | Import an Open Knowledge Format (OKF) bundle into a knowledge base. |

## knowledge-bases create

Create a knowledge base in the current organization.

```bash
everruns knowledge-bases create [OPTIONS] --name <name>
```

| Flag | Description |
|---|---|
| `--description <DESCRIPTION>` | Human-readable description. |
| `--embedding-model-id <EMBEDDING_MODEL_ID>` | Optional embedding model for hybrid retrieval. |
| `--name <NAME>` | Required. Human-readable name. |

Example:

```bash
# Start a knowledge base for a team's runbooks
everruns knowledge-bases create --name support-runbooks --description 'How we handle common support cases' --reason 'Give agents a shared source of answers'
```

## knowledge-bases delete

Archive a knowledge base.

```bash
everruns knowledge-bases delete [OPTIONS] --kb-id <kb_id>
```

| Flag | Description |
|---|---|
| `--kb-id <KB_ID>` | Required. Knowledge base's prefixed public identifier. |

Example:

```bash
# Archive a knowledge base nobody uses any more
everruns knowledge-bases delete --kb-id kb_01h9 --reason 'Merged into support-runbooks'
```

## knowledge-bases get

Get a knowledge base by ID.

```bash
everruns knowledge-bases get [OPTIONS] [KB_ID]
```

| Flag | Description |
|---|---|
| `--kb-id <KB_ID>` | Knowledge base's prefixed public identifier. |

Example:

```bash
# Show a knowledge base's settings
everruns knowledge-bases get kb_01h9
```

## knowledge-bases list

List knowledge bases in the current organization.

```bash
everruns knowledge-bases list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--include-archived` | Include archived knowledge bases. |
| `--search <SEARCH>` | Case-insensitive name filter. |

Example:

```bash
# Find a knowledge base by name when you do not know the id
everruns knowledge-bases list --search runbooks
```

## knowledge-bases update

Update a knowledge base.

```bash
everruns knowledge-bases update [OPTIONS] --kb-id <kb_id>
```

| Flag | Description |
|---|---|
| `--description <DESCRIPTION>` | Human-readable description. |
| `--embedding-model-id <EMBEDDING_MODEL_ID>` | Optional embedding model for hybrid retrieval. |
| `--kb-id <KB_ID>` | Required. Knowledge base's prefixed public identifier. |
| `--name <NAME>` | Human-readable name. |

Example:

```bash
# Rename a knowledge base
everruns knowledge-bases update --kb-id kb_01h9 --name support-playbooks --reason 'Broader scope than runbooks'
```

## knowledge-bases entries create

Create an entry inside a knowledge base.

```bash
everruns knowledge-bases entries create [OPTIONS] --body <body> --kb-id <kb_id> --title <title>
```

| Flag | Description |
|---|---|
| `--body <BODY>` | Required. |
| `--kb-id <KB_ID>` | Required. Knowledge base's prefixed public identifier. |
| `--kind <KIND>` | Discriminator selecting the variant of this resource. |
| `--resource <RESOURCE>` | Optional OKF resource URI identifying the underlying asset. |
| `--tags <TAGS>` | Free-form tags attached to this resource. Repeatable. |
| `--title <TITLE>` | Required. Human-readable title. |

Example:

```bash
# Add a runbook entry to a knowledge base
everruns knowledge-bases entries create --kb-id kb_01h9 --title 'Refund a payment past 30 days' --body 'Escalate to billing with the order id.' --kind runbook --tags billing,refunds --reason 'Document the refund exception'
```

## knowledge-bases entries delete

Delete a knowledge entry.

```bash
everruns knowledge-bases entries delete [OPTIONS] --entry-id <entry_id> --kb-id <kb_id>
```

| Flag | Description |
|---|---|
| `--entry-id <ENTRY_ID>` | Required. Knowledge base entry's prefixed public identifier. |
| `--kb-id <KB_ID>` | Required. Knowledge base's prefixed public identifier. |

Example:

```bash
# Remove an outdated entry
everruns knowledge-bases entries delete --kb-id kb_01h9 --entry-id kbe_01h9 --reason 'Policy changed'
```

## knowledge-bases entries get

Get a knowledge entry by ID.

```bash
everruns knowledge-bases entries get [OPTIONS] --entry-id <entry_id> --kb-id <kb_id>
```

| Flag | Description |
|---|---|
| `--entry-id <ENTRY_ID>` | Required. Knowledge base entry's prefixed public identifier. |
| `--kb-id <KB_ID>` | Required. Knowledge base's prefixed public identifier. |

Example:

```bash
# Read one entry's full body
everruns knowledge-bases entries get --kb-id kb_01h9 --entry-id kbe_01h9
```

## knowledge-bases entries list

List entries inside a knowledge base.

```bash
everruns knowledge-bases entries list [OPTIONS] --kb-id <kb_id>
```

| Flag | Description |
|---|---|
| `--kb-id <KB_ID>` | Required. Knowledge base's prefixed public identifier. |
| `--kind <KIND>` | Discriminator selecting the variant of this resource. |
| `--search <SEARCH>` | Case-insensitive title or body filter. |

Example:

```bash
# Find entries about a topic inside one knowledge base
everruns knowledge-bases entries list --kb-id kb_01h9 --search refund
```

## knowledge-bases entries update

Update a knowledge entry.

```bash
everruns knowledge-bases entries update [OPTIONS] --entry-id <entry_id> --kb-id <kb_id>
```

| Flag | Description |
|---|---|
| `--body <BODY>` | Updated entry body. |
| `--entry-id <ENTRY_ID>` | Required. Knowledge base entry's prefixed public identifier. |
| `--kb-id <KB_ID>` | Required. Knowledge base's prefixed public identifier. |
| `--kind <KIND>` | Discriminator selecting the variant of this resource. |
| `--resource <RESOURCE>` | Optional OKF resource URI. |
| `--tags <TAGS>` | Free-form tags attached to this resource. Repeatable. |
| `--title <TITLE>` | Human-readable title. |

Example:

```bash
# Correct an entry's body after a policy change
everruns knowledge-bases entries update --kb-id kb_01h9 --entry-id kbe_01h9 --body 'Escalate to billing within 24 hours.' --reason 'Updated refund policy'
```

## knowledge-bases okf-import import

Import an Open Knowledge Format (OKF) bundle into a knowledge base.

```bash
everruns knowledge-bases okf-import import [OPTIONS] --kb-id <kb_id>
```

| Flag | Description |
|---|---|
| `--bundle-base64 <BUNDLE_BASE64>` | A base64-encoded `.tar.gz` OKF bundle. |
| `--files <FILES>` | Inline bundle files. |
| `--kb-id <KB_ID>` | Required. Knowledge base's prefixed public identifier. |
| `--prune` | When true, delete previously-imported entries absent from this bundle. |

Example:

```bash
# Load an OKF bundle (a .tar.gz of markdown concepts) into a knowledge base
everruns knowledge-bases okf-import import --kb-id kb_01h9 --bundle-base64 "$(base64 < bundle.tar.gz)" --prune true --reason 'Sync the docs repo'
```

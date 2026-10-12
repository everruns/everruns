---
title: everruns plugin-marketplaces
description: "Sources of installable plugins. CLI reference for everruns plugin-marketplaces."
sidebar:
  label: plugin-marketplaces
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Sources of installable plugins.

| Command | What it does |
|---|---|
| [`plugin-marketplaces create`](#plugin-marketplaces-create) | Register a new plugin marketplace for this organization. |
| [`plugin-marketplaces delete`](#plugin-marketplaces-delete) | Delete a plugin marketplace. |
| [`plugin-marketplaces get`](#plugin-marketplaces-get) | Get a plugin marketplace by ID. |
| [`plugin-marketplaces list`](#plugin-marketplaces-list) | List plugin marketplaces registered for this organization. |
| [`plugin-marketplaces sync`](#plugin-marketplaces-sync) | Re-sync the plugin marketplace catalog from its source. |
| [`plugin-marketplaces update`](#plugin-marketplaces-update) | Update a plugin marketplace (name or status). |
| [`plugin-marketplaces plugins get`](#plugin-marketplaces-plugins-get) | List plugin catalog entries for a marketplace (with installed flag per entry). |

## plugin-marketplaces create

Register a new plugin marketplace for this organization.

```bash
everruns plugin-marketplaces create [OPTIONS] --name <name> --source <source> --source-type <source_type>
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Required. Unique name within the org (kebab-case, e.g. |
| `--source <SOURCE>` | Required. Source value. |
| `--source-type <SOURCE_TYPE>` | Required. Source type: `github`, `url`, or `local_path` (dev/test only). |

Example:

```bash
# Register a GitHub repository as a source of plugins
everruns plugin-marketplaces create --name acme-plugins --source-type github --source acme/plugins --reason 'Share internal plugins'
```

## plugin-marketplaces delete

Delete a plugin marketplace. Installed plugins become unattached.

```bash
everruns plugin-marketplaces delete [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Public marketplace ID. |

Example:

```bash
# Remove a marketplace; installed plugins stay but become unattached
everruns plugin-marketplaces delete plgmkt_01h9 --reason 'Source repository archived'
```

## plugin-marketplaces get

Get a plugin marketplace by ID.

```bash
everruns plugin-marketplaces get [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Public marketplace ID (`plgmkt_<32-hex>`). |

Example:

```bash
# Check a marketplace's source and sync state
everruns plugin-marketplaces get plgmkt_01h9
```

## plugin-marketplaces list

List plugin marketplaces registered for this organization.

```bash
everruns plugin-marketplaces list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--search <SEARCH>` | Only marketplaces whose name matches this text. |

Example:

```bash
# Find marketplaces by name when you do not know the id
everruns plugin-marketplaces list --search acme
```

## plugin-marketplaces sync

Re-sync the plugin marketplace catalog from its source.

```bash
everruns plugin-marketplaces sync [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Public marketplace ID. |

Example:

```bash
# Refresh a marketplace catalog after its source changed
everruns plugin-marketplaces sync plgmkt_01h9 --reason 'New plugins published'
```

## plugin-marketplaces update

Update a plugin marketplace (name or status).

```bash
everruns plugin-marketplaces update [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Public marketplace ID. |
| `--name <NAME>` | New marketplace name. |
| `--status <STATUS>` | New lifecycle status, e.g. |

Example:

```bash
# Rename a marketplace or disable it
everruns plugin-marketplaces update plgmkt_01h9 --status disabled --reason 'Pause installs while the catalog is audited'
```

## plugin-marketplaces plugins get

List plugin catalog entries for a marketplace (with installed flag per entry).

```bash
everruns plugin-marketplaces plugins get [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Public marketplace ID. |

Example:

```bash
# Browse what a marketplace offers and what is already installed
everruns plugin-marketplaces plugins get plgmkt_01h9
```

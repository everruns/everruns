---
title: everruns plugins
description: "Plugins installed in this organization. CLI reference for everruns plugins."
sidebar:
  label: plugins
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Plugins installed in this organization.

| Command | What it does |
|---|---|
| [`plugins get`](#plugins-get) | Get an installed plugin by ID. |
| [`plugins install`](#plugins-install) | Install a plugin from a marketplace catalog entry. |
| [`plugins list`](#plugins-list) | List installed plugins for this organization. |
| [`plugins patch`](#plugins-patch) | Update an installed plugin's status or MCP acting identities. |
| [`plugins uninstall`](#plugins-uninstall) | Uninstall a plugin. |
| [`plugins update`](#plugins-update) | Re-install an installed plugin at the marketplace's current catalog version. |

## plugins get

Get an installed plugin by ID.

```bash
everruns plugins get [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Public plugin ID (`plugin_<32-hex>`). |

Example:

```bash
# Inspect an installed plugin's status and definition
everruns plugins get plugin_01h9
```

## plugins install

Install a plugin from a marketplace catalog entry.

```bash
everruns plugins install [OPTIONS] --marketplace-id <marketplace_id> --plugin-name <plugin_name>
```

| Flag | Description |
|---|---|
| `--marketplace-id <MARKETPLACE_ID>` | Required. Public ID of the marketplace to install from. |
| `--plugin-name <PLUGIN_NAME>` | Required. Name of the plugin entry in the marketplace catalog. |

Example:

```bash
# Install a plugin listed in a marketplace catalog
everruns plugins install --marketplace-id plgmkt_01h9 --plugin-name microsoft-docs --reason 'Give agents the Microsoft docs tools'
```

## plugins list

List installed plugins for this organization.

```bash
everruns plugins list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--search <SEARCH>` | Only plugins whose name matches this text. |

Example:

```bash
# Find installed plugins by name
everruns plugins list --search docs
```

## plugins patch

Update an installed plugin's status or MCP acting identities.

```bash
everruns plugins patch [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Public plugin ID. |
| `--mcp-server-identities <MCP_SERVER_IDENTITIES>` | Explicit acting-identity choices for legacy authenticated MCP servers. Only `user` and `servi... |
| `--status <STATUS>` | New lifecycle status: `active` or `disabled`. |

Example:

```bash
# Disable a plugin, or choose which identity its MCP servers act as
everruns plugins patch plugin_01h9 --status disabled --reason 'Pause during the incident'
```

## plugins uninstall

Uninstall a plugin. Capability ref becomes dangling for assigned agents.

```bash
everruns plugins uninstall [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Public plugin ID. |

Example:

```bash
# Remove a plugin; agents that reference it will report a dangling capability
everruns plugins uninstall plugin_01h9 --reason 'Replaced by a native capability'
```

## plugins update

Re-install an installed plugin at the marketplace's current catalog version.

```bash
everruns plugins update [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Public plugin ID. |

Example:

```bash
# Move an installed plugin to the marketplace's current version
everruns plugins update plugin_01h9 --reason 'Pick up the security fix'
```

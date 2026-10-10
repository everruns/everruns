---
title: everruns mcp-servers
description: "Registered MCP servers available to agents and sessions. CLI reference for everruns mcp-servers."
sidebar:
  label: mcp-servers
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Registered MCP servers available to agents and sessions.

| Command | What it does |
|---|---|
| [`mcp-servers check-connection`](#mcp-servers-check-connection) | Check again whether an OAuth MCP server's sign-in service is reachable, and record the result on the server. |
| [`mcp-servers create`](#mcp-servers-create) | Create a new MCP server with a name, URL, and optional authentication. |
| [`mcp-servers delete`](#mcp-servers-delete) | Archive an MCP server (soft delete). |
| [`mcp-servers destroy`](#mcp-servers-destroy) | Permanently delete an archived MCP server. |
| [`mcp-servers get`](#mcp-servers-get) | Get a single MCP server by ID. |
| [`mcp-servers tools`](#mcp-servers-tools) | List the tools an MCP server offers, with each tool's annotations and saved risk label. |
| [`mcp-servers list`](#mcp-servers-list) | List all active MCP servers. |
| [`mcp-servers label-tool`](#mcp-servers-label-tool) | Set or clear a person's risk label for one MCP server tool. |
| [`mcp-servers suggest-labels`](#mcp-servers-suggest-labels) | Ask the deployment's decision service to suggest read_only or changes for every tool of an MCP server that has no label yet. |
| [`mcp-servers update`](#mcp-servers-update) | Update an MCP server. |

## mcp-servers check-connection

Check again whether an OAuth MCP server's sign-in service is reachable, and record the result on the server.

```bash
everruns mcp-servers check-connection [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# See whether people can sign in to an OAuth MCP server from here
everruns mcp-servers check-connection mcp_01h9
```

## mcp-servers create

Create a new MCP server with a name, URL, and optional authentication.

```bash
everruns mcp-servers create [OPTIONS] --name <name> --url <url>
```

| Flag | Description |
|---|---|
| `--api-key <API_KEY>` | API key for authentication (optional). |
| `--auth-mode <AUTH_MODE>` |  |
| `--description <DESCRIPTION>` | A human-readable description of what the MCP server provides. |
| `--elicitation-policy <ELICITATION_POLICY>` |  |
| `--headers <HEADERS>` | Additional HTTP headers for authentication. |
| `--name <NAME>` | Required. The name of the MCP server. |
| `--protocol-mode <PROTOCOL_MODE>` |  |
| `--service-connection-provider <SERVICE_CONNECTION_PROVIDER>` | Connection provider whose connection on an agent's service virtual user supplies the service ... |
| `--transport-type <TRANSPORT_TYPE>` | MCP Server transport type. One of `http`, `stdio`. |
| `--url <URL>` | Required. The URL of the MCP server endpoint. |

Example:

```bash
# Register an MCP server so agents can use its tools
everruns mcp-servers create --name github --url https://api.example.com/mcp --reason 'Give agents GitHub tools'
```

## mcp-servers delete

Archive an MCP server (soft delete). Can be restored.

```bash
everruns mcp-servers delete [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Archive an MCP server, keeping it restorable
everruns mcp-servers delete mcp_01h9 --reason 'Moved to the hosted GitHub server'
```

## mcp-servers destroy

Permanently delete an archived MCP server.

```bash
everruns mcp-servers destroy [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Permanently remove an already-archived MCP server
everruns mcp-servers destroy mcp_01h9 --reason 'Retired after the archive window'
```

## mcp-servers get

Get a single MCP server by ID.

```bash
everruns mcp-servers get [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Show one MCP server's URL and configuration
everruns mcp-servers get mcp_01h9
```

## mcp-servers tools

List the tools an MCP server offers, with each tool's annotations and saved risk label.

```bash
everruns mcp-servers tools [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# See which tools of an MCP server ask for approval
everruns mcp-servers tools mcp_01h9
```

## mcp-servers list

List all active MCP servers. Use search for name/description search, include_archived=true to include archived.

```bash
everruns mcp-servers list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--include-archived` |  |
| `--search <SEARCH>` |  |

Example:

```bash
# Find a registered MCP server by name
everruns mcp-servers list --search github
```

## mcp-servers label-tool

Set or clear a person's risk label for one MCP server tool. read_only never asks for approval in normal mode, changes always asks, null clears.

```bash
everruns mcp-servers label-tool [OPTIONS] [ID] [TOOL_NAME]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |
| `--tool-name <TOOL_NAME>` | The tool's own name on the MCP server. |
| `--label <LABEL>` |  |

Example:

```bash
# Stop a read-only MCP tool from asking for approval
everruns mcp-servers label-tool mcp_01h9 search_docs --label read_only --reason 'Only reads documentation'
```

## mcp-servers suggest-labels

Ask the deployment's decision service to suggest read_only or changes for every tool of an MCP server that has no label yet. Suggestions are never applied: a person confirms one by setting the label.

```bash
everruns mcp-servers suggest-labels [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Get suggested risk labels for the unlabeled tools of an MCP server
everruns mcp-servers suggest-labels mcp_01h9
```

## mcp-servers update

Update an MCP server. Only provided fields are changed.

```bash
everruns mcp-servers update [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |
| `--api-key <API_KEY>` | API key for authentication. |
| `--auth-mode <AUTH_MODE>` |  |
| `--description <DESCRIPTION>` | A human-readable description of what the MCP server provides. |
| `--elicitation-policy <ELICITATION_POLICY>` |  |
| `--headers <HEADERS>` | Additional HTTP headers for authentication. |
| `--name <NAME>` | The name of the MCP server. |
| `--protocol-mode <PROTOCOL_MODE>` |  |
| `--service-connection-provider <SERVICE_CONNECTION_PROVIDER>` | Connection provider whose connection on an agent's service virtual user supplies the service ... |
| `--status <STATUS>` |  |
| `--transport-type <TRANSPORT_TYPE>` |  |
| `--url <URL>` | The URL of the MCP server endpoint. |

Example:

```bash
# Repoint an MCP server at a new URL
everruns mcp-servers update mcp_01h9 --name github-prod --reason 'Rename for the production rollout'
```

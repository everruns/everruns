---
title: Deno Sandboxes
description: Run agent code in isolated Deno cloud microVMs with command execution, file access, and session-scoped lifecycle management.
sidebar:
  label: Deno Sandboxes
---

| | |
|---|---|
| **ID** | `deno` |
| **Category** | Sandboxes |
| **Features** | None |
| **Dependencies** | [`session_storage`](/capabilities/session-storage/) |

Run code in cloud-based Deno sandboxes. Create isolated Linux microVMs, execute commands, and manage files. Each sandbox is scoped to the session and supports configurable memory and network policies.

> **Experimental:** This capability may change in future releases.

## Tools

### `deno_create_sandbox`

Create a new Deno sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `title` | string | no | Sandbox label |
| `region` | string | no | Region (e.g. `ord` or `ams`) |
| `timeout` | string | no | Sandbox lifetime (e.g. `20m`). Default: `10m` |
| `memory_mb` | integer | no | Memory in MiB (1–16384). Default: 512 |
| `allow_net` | array | no | Outbound network allowlist hosts/IPs |

Returns `sandbox_id`, `region`, and `workspace_path`.

### `deno_exec`

Execute a shell command in a sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `command` | string | yes | Shell command to run |
| `cwd` | string | no | Working directory |

### `deno_read_file`

Read a text file from a sandbox filesystem.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `path` | string | yes | File path |
| `offset` | integer | no | Line offset to start reading from |
| `limit` | integer | no | Maximum number of lines to return |

### `deno_write_file`

Write a text file into a sandbox filesystem.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `path` | string | yes | File path |
| `content` | string | yes | File content |

### `deno_list_sandboxes`

List all Deno sandboxes created in the current session.

### `deno_manage_sandbox`

Delete a Deno sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `action` | string | yes | `delete` |

## Authentication

Deno API token is resolved automatically from **Settings > Connections > Deno**.

## Notes

- Sandboxes are isolated Linux microVMs with network access (subject to `allow_net`)
- Sandboxes are automatically tracked for the session; use `deno_manage_sandbox` to delete when done
- File operations target the sandbox filesystem, not the session workspace

## See Also

- [Storage](/capabilities/session-storage/), token and state persistence
- [Deno integration guide](/integrations/deno/), setup and configuration
- [Capabilities Overview](/capabilities/)

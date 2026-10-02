---
title: E2B Sandboxes
description: Run agent code in isolated E2B cloud sandboxes with command execution, file access, and session-scoped lifecycle management.
sidebar:
  label: E2B
---

| | |
|---|---|
| **ID** | `e2b` |
| **Category** | Sandboxes |
| **Features** | None |
| **Dependencies** | [`session_storage`](/capabilities/session-storage/) |

Run code in cloud sandboxes powered by E2B. Create isolated Linux environments, execute commands, and manage sandbox files. Sandboxes are tracked per session; pause, resume, or delete them with `e2b_manage_sandbox`.

## Tools

### `e2b_create_sandbox`

Create a new E2B sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `template` | string | no | Sandbox template name or ID |
| `timeout_seconds` | integer | no | Sandbox TTL in seconds |

Returns `sandbox_id` and connection details.

### `e2b_exec`

Execute a shell command in a sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `command` | string | yes | Shell command to run |
| `cwd` | string | no | Working directory |
| `timeout_ms` | integer | no | Command timeout in milliseconds |

### `e2b_read_file`

Read a text file from a sandbox filesystem.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `path` | string | yes | File path |
| `offset` | integer | no | Line offset to start reading from |
| `limit` | integer | no | Maximum number of lines to return |

### `e2b_write_file`

Write a text file into a sandbox filesystem.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `path` | string | yes | File path |
| `content` | string | yes | File content |

### `e2b_list_sandboxes`

List all E2B sandboxes created in the current session.

### `e2b_manage_sandbox`

Pause, resume, or delete an E2B sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `action` | string | yes | `pause`, `resume`, or `delete` |
| `timeout_seconds` | integer | no | Timeout to apply when resuming |

## Authentication

E2B API key is resolved automatically from **Settings > Connections > E2B**.

## Notes

- Each sandbox is an isolated Linux environment with internet access
- Sandboxes are tracked per session; use `e2b_manage_sandbox` to delete when done
- File operations target the sandbox filesystem, not the session workspace

## See Also

- [Storage](/capabilities/session-storage/), API key and state persistence
- [Capabilities Overview](/capabilities/)

---
title: Daytona
description: Run agent code in Daytona cloud sandboxes with command execution, file access, workspace downloads, and session-scoped lifecycle controls.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `daytona` |
| **Category** | Sandboxes |
| **Features** | None |
| **Dependencies** | [`session_storage`](/capabilities/session-storage/) |

Run code in cloud-based sandboxes powered by Daytona. Create multiple isolated Linux environments per session, execute commands, manage files, and download results.

## Tools

### `daytona_create_sandbox`

Create and start a new sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `title` | string | no | Sandbox name |
| `image` | string | no | Container image |
| `upload_files` | array | no | Files to upload after creation |

### `daytona_exec`

Run a shell command in a sandbox (synchronous).

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `command` | string | yes | Shell command to execute |
| `cwd` | string | no | Working directory |
| `timeout_ms` | integer | no | Timeout in milliseconds |

### `daytona_read_file`

Read a file from a sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `path` | string | yes | File path |

### `daytona_write_file`

Write a file to a sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `path` | string | yes | File path |
| `content` | string | yes | File content |

### `daytona_download_workspace`

Download sandbox workspace to session storage.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |

### `daytona_list_snapshots`

List available Daytona snapshots (names, CPU/memory/disk specs, state). Use a snapshot name as the `snapshot` parameter of `daytona_create_sandbox`. Takes no parameters.

### `daytona_list_sandboxes`

List all sandboxes for the current session.

### `daytona_manage_sandbox`

Stop or delete a sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `action` | string | yes | `stop` or `delete` |

### `daytona_git_clone`

Clone a git repository into a sandbox. Automatically uses connected GitHub credentials for private repos.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `url` | string | yes | Repository URL or `user/repo` shorthand |
| `branch` | string | no | Branch to clone |

### `daytona_git_credentials`

Configure git credentials for push/pull/fetch.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |

### `daytona_api_call` (opt-in)

Call any Daytona REST endpoint directly, for operations the dedicated tools do not cover. Only available when the capability config sets `enable_api_calling` to `true`. Authentication headers are injected automatically, and the Daytona OpenAPI spec is mounted at `/daytona/openapi.yaml`.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `method` | string | yes | `GET`, `POST`, `PUT`, `PATCH`, or `DELETE` |
| `path` | string | yes | Management API path (`/sandbox/...`) or Toolbox API path (`/toolbox/{sandbox_id}/...`) |
| `body` | object or array | no | JSON request body |

## Authentication

Daytona API key is resolved automatically from **Settings > Connections > Daytona**.

## Notes

- Each sandbox is a full isolated Linux environment with network access
- Sandboxes auto-stop after 5 minutes of inactivity
- Always delete sandboxes when done to free resources
- All sandbox-scoped tools (not `daytona_create_sandbox`, `daytona_list_sandboxes`, or `daytona_list_snapshots`) require a `sandbox_id`

## See Also

- [Storage](/capabilities/session-storage/), API key and state persistence
- [Daytona integration guide](/integrations/daytona/), setup and configuration
- [Capabilities Overview](/capabilities/)

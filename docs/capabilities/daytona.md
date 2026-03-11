---
title: Daytona
description: Run code in isolated cloud sandboxes with Daytona, including command execution, file access, and session-scoped environments.
---

| | |
|---|---|
| **ID** | `daytona` |
| **Category** | Execution |
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

## Authentication

Daytona API key is resolved automatically:

1. Session secret `DAYTONA_API_KEY` (highest priority)
2. User connection (Settings > Connections > Daytona)

## Use Cases

- **Code execution** — run and test code in isolated Linux environments
- **Multi-language projects** — use custom container images for any stack
- **Repository analysis** — clone and explore repos in a sandbox
- **Build and test** — compile, test, and package in clean environments

## Notes

- Each sandbox is a full isolated Linux environment with network access
- Sandboxes auto-stop after 5 minutes of inactivity
- Always delete sandboxes when done to free resources
- All tools except `daytona_create_sandbox` and `daytona_list_sandboxes` require a `sandbox_id`

## See Also

- [Storage](/capabilities/session-storage/) — API key and state persistence
- [Daytona integration guide](/integrations/daytona/) — setup and configuration
- [Capabilities Overview](/capabilities/)

---
title: Daytona
description: Run agent code in Daytona cloud sandboxes with command execution, file access, git, workspace downloads, and session-scoped lifecycle controls.
appliesTo: [platform, cloud]
---

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 275 287" width="49.8" height="52.0" fill="currentColor" aria-hidden="true" style="float: right; margin-left: 16px;"><path d="M14.5584 193.736H114.275V227.925H14.5584V193.736Z"/><path d="M148.464 74.076H262.426V108.265H148.464V74.076Z"/><path d="M88.6338 84.6127L173.246 0L197.422 24.175L112.809 108.788L88.6338 84.6127Z"/><path d="M89.157 170.084L24.175 105.102L0 129.277L64.9819 194.259L89.157 170.084Z"/><path d="M174.629 217.911L106.133 286.407L81.9577 262.232L150.454 193.736L174.629 217.911Z"/><path d="M174.106 132.44L250.66 208.994L274.835 184.819L198.281 108.265L174.106 132.44Z"/><path d="M88.6338 48.434V131.057H54.4451L54.4451 48.434H88.6338Z"/><path d="M208.294 168.094V270.66H174.106V168.094H208.294Z"/></svg>

| | |
|---|---|
| **ID** | `daytona` |
| **Category** | Sandboxes |
| **Features** | None |
| **Dependencies** | [`session_storage`](/capabilities/session-storage/) |

Run code in cloud sandboxes powered by [Daytona](https://www.daytona.io/). An agent can create several isolated Linux environments per session, each with network access, then execute commands, manage files, clone repositories, and download results. The capability is marked experimental and may change.

## Set up

1. In the [Daytona Dashboard](https://app.daytona.io), open **API Keys** in your account settings, select **Create New API Key**, and copy the key.
2. In Everruns, open **Settings** > **Connections**, find **Daytona**, select **Connect**, and paste the key.

Once connected, agents with the Daytona capability can use the tools below.

## Tools

### `daytona_create_sandbox`

Create and start a new sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `title` | string | no | Sandbox name |
| `size` | string | no | `small` (1 vCPU, 1 GiB RAM, 3 GiB disk, default), `medium` (2 vCPU, 4 GiB, 8 GiB), or `large` (4 vCPU, 8 GiB, 10 GiB) |
| `snapshot` | string | no | Daytona snapshot name; overrides `size` |
| `auto_stop_minutes` | integer | no | Minutes of inactivity before auto-stop, 1 to 60 (default 5) |
| `upload_files` | array | no | `{session_path, sandbox_path}` pairs to copy from session storage |

### `daytona_exec`

Run a shell command in a sandbox and wait for it.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `command` | string | yes | Shell command to execute |
| `cwd` | string | no | Working directory (default: sandbox workspace) |
| `timeout` | integer | no | Timeout in milliseconds (default 300000) |

### `daytona_read_file`

Read a file from a sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `path` | string | yes | File path |
| `offset` | integer | no | Zero-based line offset to start reading from |
| `limit` | integer | no | Maximum number of lines to return |

### `daytona_write_file`

Write a file to a sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `path` | string | yes | File path |
| `content` | string | yes | File content |

### `daytona_download_workspace`

Download a sandbox directory to session storage.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `sandbox_path` | string | no | Root path in the sandbox (default: workspace path) |
| `session_path` | string | no | Destination in session storage (default: `/workspace`) |

### `daytona_list_snapshots`

List available Daytona snapshots (names, CPU, memory, and disk specs, state). Use a snapshot name as the `snapshot` parameter of `daytona_create_sandbox`. Takes no parameters.

### `daytona_list_sandboxes`

List all sandboxes for the current session. Takes no parameters.

### `daytona_manage_sandbox`

Stop or delete a sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `action` | string | yes | `stop` or `delete` |

### `daytona_git_clone`

Clone a git repository into a sandbox. Uses your connected GitHub credentials for private repositories.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `repo_url` | string | yes | Repository URL or `user/repo` shorthand |
| `branch` | string | no | Branch to clone (default: the default branch) |
| `path` | string | no | Clone destination (default: `/home/daytona/<owner>/<repo>`) |

### `daytona_git_credentials`

Configure git credentials in a sandbox so later `git push`, `pull`, and `fetch` commands run through `daytona_exec` authenticate.

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

## Config

```json
{
  "enable_api_calling": true
}
```

`enable_api_calling` is the only setting, and it defaults to `false`.

## Git

- `daytona_git_clone` uses your connected GitHub account, so private repositories clone without extra setup.
- For push, pull, or fetch, call `daytona_git_credentials` once after creating the sandbox, then run git commands with `daytona_exec`.
- Repository arguments accept `user/repo` shorthand instead of full URLs.

## Sandbox lifecycle

Sandboxes auto-stop after 5 minutes of inactivity by default as a safety net. Delete sandboxes when you are done: stopping only pauses a sandbox, and it stays visible in your Daytona dashboard.

All sandbox-scoped tools require a `sandbox_id`. The exceptions are `daytona_create_sandbox`, `daytona_list_sandboxes`, and `daytona_list_snapshots`.

## Security

- API keys are encrypted at rest (AES-256-GCM envelope encryption).
- Each sandbox is isolated from other sandboxes and the host.
- Git credentials are short-lived and scoped to the sandbox.
- Sandbox state is stored in encrypted session secrets.

## See also

- [Storage](/capabilities/session-storage/): API key and state persistence.
- [Daytona documentation](https://www.daytona.io/docs)
- [Integrations](/integrations/): every vendor integration.
- [Capabilities Overview](/capabilities/)

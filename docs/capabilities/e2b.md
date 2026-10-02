---
title: E2B Sandboxes
description: Run agent code in isolated E2B cloud sandboxes with your own API key, command execution, file access, and session-scoped lifecycle management.
sidebar:
  label: E2B
appliesTo: [platform, cloud]
---

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 65 18" width="52.0" height="14.4" fill="currentColor" aria-hidden="true" style="float: right; margin-left: 16px;"><path d="M20.2235 0V4.67645H5.49328C5.04263 4.67661 4.67645 5.0426 4.67645 5.49328V5.84494C4.67645 6.29563 5.04263 6.66161 5.49328 6.66178H20.2235V11.3382H5.49328C5.04263 11.3384 4.67645 11.7044 4.67645 12.1551V12.5067C4.67657 12.9573 5.04271 13.3222 5.49328 13.3223H20.2235V18H3.12668C1.39998 17.9996 1.98414e-05 16.5989 0 14.8721V3.12668C0.000280465 1.40008 1.40013 0.000432767 3.12668 0H20.2235Z"/><path d="M39.2723 0C40.9992 0.000155056 42.399 1.40092 42.399 3.12791V8.36701C42.3989 10.0101 41.0672 11.3417 39.424 11.3419H36.9587C36.9413 11.3408 36.9232 11.3382 36.9057 11.3382H27.6379C27.1873 11.3384 26.8211 11.7044 26.8211 12.1551V12.5067C26.8213 12.9572 27.1874 13.3221 27.6379 13.3223H42.3903V18H22.1446V9.63299C22.1446 7.98998 23.4767 6.65732 25.1195 6.65684H27.5762C27.5967 6.65838 27.6174 6.66174 27.6379 6.66178H36.9057C37.3563 6.66171 37.7225 6.29578 37.7225 5.84494V5.49328C37.7224 5.04255 37.3563 4.67775 36.9057 4.67768H22.1755V0H39.2723Z"/><path fill-rule="evenodd" clip-rule="evenodd" d="M61.4379 0C63.1648 3.6786e-05 64.5655 1.39985 64.5658 3.12668V14.8721C64.5658 16.5992 63.1649 18 61.4379 18H44.3386V0H61.4379ZM49.8319 11.3382C49.3813 11.3384 49.0151 11.7044 49.0151 12.1551V12.5067C49.0152 12.9573 49.3813 13.3222 49.8319 13.3223H59.0725C59.523 13.3222 59.888 12.9574 59.8881 12.5067V12.1551C59.8881 11.7043 59.5231 11.3384 59.0725 11.3382H49.8319ZM49.8319 4.67645C49.3813 4.67661 49.0151 5.0426 49.0151 5.49328V5.84494C49.0151 6.29562 49.3813 6.66161 49.8319 6.66178H59.0725C59.5231 6.66162 59.8881 6.29571 59.8881 5.84494V5.49328C59.8881 5.04252 59.5231 4.67661 59.0725 4.67645H49.8319Z"/></svg>

| | |
|---|---|
| **ID** | `e2b` |
| **Category** | Sandboxes |
| **Features** | None |
| **Dependencies** | [`session_storage`](/capabilities/session-storage/) |

Run code in cloud sandboxes powered by [E2B](https://e2b.dev/docs). An agent can create, pause, resume, delete, and work in several isolated Linux sandboxes per session. Sandboxes are tracked per session.

You bring your own E2B API key. There is no platform-owned or environment-variable fallback, so sandbox costs and quotas stay in your own E2B account.

## Set up

1. In the [E2B Dashboard](https://e2b.dev/dashboard), create an API key and copy it.
2. In Everruns, open **Settings** > **Connections**, find **E2B**, select **Connect**, and paste the key.

Every E2B operation requires a user-provided key. If none is configured, the agent shows an inline connection prompt.

## Tools

### `e2b_create_sandbox`

Create a new E2B sandbox. Returns `sandbox_id` and connection details.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `title` | string | no | Human-readable sandbox name |
| `template` | string | no | E2B template ID (default `base`) |
| `timeout_seconds` | integer | no | Sandbox TTL in seconds (default `3600`) |
| `env_vars` | object | no | Environment variables injected into the sandbox |
| `upload_files` | array | no | `{session_path, sandbox_path}` pairs to copy from session storage |

### `e2b_exec`

Execute a shell command in a sandbox, capturing stdout, stderr, and the exit code.

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

List all E2B sandboxes created in the current session. Takes no parameters.

### `e2b_manage_sandbox`

Pause, resume, or delete an E2B sandbox.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `sandbox_id` | string | yes | Target sandbox |
| `action` | string | yes | `pause`, `resume`, or `delete` |
| `timeout_seconds` | integer | no | Timeout to apply when resuming |

## How it works

E2B exposes two surfaces, and the capability uses both:

- **Management API** (`api.e2b.app`): sandbox lifecycle, metadata, and timeout control.
- **envd sandbox endpoint**: in-sandbox file access and command execution.

Per-sandbox state (sandbox ID, domain, access token, workspace path, timeout) is stored in encrypted session secrets and registered as a leased resource, so orphaned sandboxes are cleaned up on the worker side. Every sandbox is tagged with Everruns ownership metadata (session, harness, org, and agent IDs) for dashboard traceability and audit review.

File operations target the sandbox filesystem, not the session workspace. Each sandbox has internet access. Use `e2b_manage_sandbox` to delete sandboxes when you are done.

## Security

- API keys resolve fresh from your user connection on each tool call. They are never stored in sandbox state or env vars, and never appear in tool output.
- envd access tokens are session-scoped and stored only in encrypted session secrets (AES-256-GCM envelope encryption).
- Sandbox isolation depends on E2B's runtime boundaries plus Everruns session-scoped secret lookups.
- E2B timeouts and auto-pause, plus Everruns leased-resource cleanup, limit resource leaks.

## See also

- [Storage](/capabilities/session-storage/): API key and state persistence.
- [E2B documentation](https://e2b.dev/docs)
- [Integrations](/integrations/): every vendor integration.
- [Capabilities Overview](/capabilities/)

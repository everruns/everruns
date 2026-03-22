---
title: Deno
description: Integrate Deno cloud sandbox environments for secure, isolated code execution. Configure access tokens, sandbox lifecycle, and session-scoped state.
---

Everruns integrates with [Deno Sandboxes](https://docs.deno.com/sandbox/) to provide cloud-based sandbox environments for secure, isolated code execution. Agents can create, manage, and interact with multiple sandboxes per session — each an isolated Linux microVM with network access.

## What You Get

- **Isolated Sandboxes**: Each sandbox is a secure, isolated Linux microVM
- **Multi-Sandbox Sessions**: Create and manage multiple sandboxes within a single session
- **File Operations**: Read and write text files in sandbox filesystems
- **Shell Execution**: Run arbitrary commands via `bash -lc` with stdout/stderr capture
- **Fixed Timeout Lifecycle**: Sandboxes are created with a concrete timeout (e.g. 20 minutes) since Everruns uses per-tool websocket connections

## Quick Start

### 1. Get Your Access Token

1. Go to the [Deno Deploy Console](https://console.deno.com)
2. Create an **organization access token** (`ddo_...`)
3. Copy the token

> **Note**: Personal tokens (`ddp_...`) are not supported in the generic connection flow yet. Use an organization token.

### 2. Connect in Everruns

1. Go to **Settings** > **Connections**
2. Find **Deno Deploy** in the available providers
3. Click **Connect** and paste your access token

Once connected, the Deno capability is automatically available in agent sessions.

### 3. Use in Sessions

Agents with the Deno capability can use these tools:

| Tool | Description |
|------|-------------|
| `deno_create_sandbox` | Create and start a new sandbox |
| `deno_exec` | Execute shell commands |
| `deno_read_file` | Read text files from sandbox |
| `deno_write_file` | Write text files to sandbox |
| `deno_list_sandboxes` | List active sandboxes in this session |
| `deno_manage_sandbox` | Delete sandboxes |

## Sandbox Lifecycle

Deno sandboxes are created with a fixed timeout (default: 20 minutes) because Everruns closes the creator websocket after each tool call. The `timeout="session"` mode is not supported.

Best practice is to explicitly delete sandboxes when done using `deno_manage_sandbox` with `action="delete"`.

## Security

- Access tokens are encrypted at rest (AES-256-GCM envelope encryption)
- Each sandbox is a fully isolated Linux microVM
- Tokens are resolved fresh from user connections on each tool call — never stored in sandbox state
- Sandbox state is stored in encrypted session secrets
- Capability is high-risk and requires admin configuration

## Links

- [Deno Sandboxes Documentation](https://docs.deno.com/sandbox/)
- [Deno Deploy Console](https://console.deno.com)

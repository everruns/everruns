---
title: Deno Deploy Sandboxes for Agent Code Tasks
description: Integrate Deno cloud sandbox environments for secure, isolated code execution. Configure access tokens, sandbox lifecycle, and session-scoped state.
sidebar:
  label: Deno
---

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="52.0" height="52.0" fill="currentColor" aria-hidden="true" style="float: right; margin-left: 16px;"><path d="M1.105 18.02A11.9 11.9 0 0 1 0 12.985q0-.698.078-1.376a12 12 0 0 1 .231-1.34A12 12 0 0 1 4.025 4.02a12 12 0 0 1 5.46-2.771 12 12 0 0 1 3.428-.23c1.452.112 2.825.477 4.077 1.05a12 12 0 0 1 2.78 1.774 12.02 12.02 0 0 1 4.053 7.078A12 12 0 0 1 24 12.985q0 .454-.036.914a12 12 0 0 1-.728 3.305 12 12 0 0 1-2.38 3.875c-1.33 1.357-3.02 1.962-4.43 1.936a4.4 4.4 0 0 1-2.724-1.024c-.99-.853-1.391-1.83-1.53-2.919a5 5 0 0 1 .128-1.518c.105-.38.37-1.116.76-1.437-.455-.197-1.04-.624-1.226-.829-.045-.05-.04-.13 0-.183a.155.155 0 0 1 .177-.053c.392.134.869.267 1.372.35.66.111 1.484.25 2.317.292 2.03.1 4.153-.813 4.812-2.627s.403-3.609-1.96-4.685-3.454-2.356-5.363-3.128c-1.247-.505-2.636-.205-4.06.582-3.838 2.121-7.277 8.822-5.69 15.032a.191.191 0 0 1-.315.19 12 12 0 0 1-1.25-1.634 12 12 0 0 1-.769-1.404M11.57 6.087c.649-.051 1.214.501 1.31 1.236.13.979-.228 1.99-1.41 2.013-1.01.02-1.315-.997-1.248-1.614.066-.616.574-1.575 1.35-1.635"/></svg>

Everruns integrates with [Deno Sandboxes](https://docs.deno.com/sandbox/) to provide cloud-based sandbox environments for secure, isolated code execution. Agents can create, manage, and interact with multiple sandboxes per session, each an isolated Linux microVM with network access.

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
- Tokens are resolved fresh from user connections on each tool call, never stored in sandbox state
- Sandbox state is stored in encrypted session secrets
- Capability is high-risk and requires admin configuration

## Links

- [Deno Sandboxes Documentation](https://docs.deno.com/sandbox/)
- [Deno Deploy Console](https://console.deno.com)

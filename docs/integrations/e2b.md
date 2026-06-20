---
title: E2B Cloud Sandboxes for Agent Code Execution
description: Integrate E2B cloud sandboxes for secure, isolated code execution. Bring your own API key, create multiple sandboxes per session, and manage their lifecycle.
sidebar:
  label: E2B
---

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="56" height="56" aria-hidden="true" style="float: right; margin-left: 16px;"><rect x="2.5" y="4" width="19" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="1.6"/><path d="M6.5 9.5l3 2.5-3 2.5M12.5 15h5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>

Everruns integrates with [E2B](https://e2b.dev/docs) to provide cloud sandbox environments for secure, isolated code execution. Agents can create, pause, resume, delete, and interact with multiple isolated Linux sandboxes per session. You bring your own E2B API key — there is no platform-owned or environment-variable fallback, so sandbox costs and quotas stay scoped to your own E2B account.

## What You Get

- **Isolated Sandboxes**: Each sandbox is a secure, isolated Linux environment
- **Multi-Sandbox Sessions**: Create and manage multiple sandboxes within a single session
- **File Operations**: Read and write files in sandbox filesystems
- **Shell Execution**: Run commands with stdout/stderr/exit-code capture
- **Lifecycle Control**: Pause, resume, and delete sandboxes; auto-timeout limits cost

## Quick Start

### 1. Get Your API Key

1. Go to the [E2B Dashboard](https://e2b.dev/dashboard)
2. Create an API key
3. Copy the key

### 2. Connect in Everruns

1. Go to **Settings** > **Connections**
2. Find **E2B** in the available providers
3. Click **Connect** and paste your API key

Once connected, the E2B capability is automatically available in agent sessions. Every E2B operation requires a user-provided key — if none is configured, the agent surfaces an inline connection prompt.

### 3. Use in Sessions

Agents with the E2B capability can use these tools:

| Tool | Description |
|------|-------------|
| `e2b_create_sandbox` | Create a sandbox from a template, optionally uploading session files |
| `e2b_exec` | Execute a shell command |
| `e2b_read_file` | Read a file from the sandbox filesystem |
| `e2b_write_file` | Write a file into the sandbox filesystem |
| `e2b_list_sandboxes` | List sandboxes created in this session |
| `e2b_manage_sandbox` | Pause, resume, or delete a sandbox |

`e2b_create_sandbox` accepts an optional `template` (default `base`), a `timeout_seconds` (default `3600`), `env_vars`, and `upload_files` mapping session paths into the sandbox.

## How It Works

E2B exposes two surfaces, and the integration uses both:

- **Management API** (`api.e2b.app`) — sandbox lifecycle, metadata, and timeout control.
- **envd sandbox endpoint** — in-sandbox file access and command execution.

Per-sandbox state (sandbox ID, domain, access token, workspace path, timeout) is stored in encrypted session secrets and registered as a leased resource, so orphaned sandboxes are cleaned up on the worker side. Every sandbox is tagged with Everruns ownership metadata (session, harness, org, and agent IDs) for dashboard traceability and audit review.

## Security

- API keys resolve fresh from your user connection on each tool call — never stored in sandbox state, env vars, or emitted in tool output
- envd access tokens are session-scoped and stored only in encrypted session secrets (AES-256-GCM envelope encryption)
- Sandbox isolation depends on E2B's runtime boundaries plus Everruns session-scoped secret lookups
- Resource leaks are mitigated by E2B timeouts and auto-pause plus Everruns leased-resource cleanup

## Links

- [E2B Documentation](https://e2b.dev/docs)
- [E2B Dashboard](https://e2b.dev/dashboard)

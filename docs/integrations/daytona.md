---
title: Daytona Integration for Agent Code Sandboxes
description: Integrate Daytona cloud sandbox environments for secure, isolated code execution. Configure API keys, workspace templates, and session-scoped sandbox lifecycle.
sidebar:
  label: Daytona
---

![Daytona Integration](daytona.png)

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 275 287" width="49.8" height="52.0" fill="currentColor" aria-hidden="true" style="float: right; margin-left: 16px;"><path d="M14.5584 193.736H114.275V227.925H14.5584V193.736Z"/><path d="M148.464 74.076H262.426V108.265H148.464V74.076Z"/><path d="M88.6338 84.6127L173.246 0L197.422 24.175L112.809 108.788L88.6338 84.6127Z"/><path d="M89.157 170.084L24.175 105.102L0 129.277L64.9819 194.259L89.157 170.084Z"/><path d="M174.629 217.911L106.133 286.407L81.9577 262.232L150.454 193.736L174.629 217.911Z"/><path d="M174.106 132.44L250.66 208.994L274.835 184.819L198.281 108.265L174.106 132.44Z"/><path d="M88.6338 48.434V131.057H54.4451L54.4451 48.434H88.6338Z"/><path d="M208.294 168.094V270.66H174.106V168.094H208.294Z"/></svg>

Everruns integrates with [Daytona](https://www.daytona.io/) to provide cloud-based sandbox environments for secure, isolated code execution. Agents can create, manage, and interact with multiple sandboxes per session, each a fully isolated Linux environment with network access.

## What You Get

- **Isolated Sandboxes**: Each sandbox is a secure, isolated Linux environment
- **Multi-Sandbox Sessions**: Create and manage multiple sandboxes within a single session
- **File Operations**: Read, write, and download files from sandbox filesystems
- **Git Integration**: Clone repositories with automatic GitHub credential forwarding
- **Shell Execution**: Run arbitrary commands with configurable timeouts

## Quick Start

### 1. Get Your API Key

1. Go to the [Daytona Dashboard](https://app.daytona.io)
2. Navigate to **API Keys** in your account settings
3. Click **Create New API Key**
4. Copy the key

### 2. Connect in Everruns

1. Go to **Settings** > **Connections**
2. Find **Daytona** in the available providers
3. Click **Connect** and paste your API key

Once connected, the Daytona capability is automatically available in agent sessions.

### 3. Use in Sessions

Agents with the Daytona capability can use these tools:

| Tool | Description |
|------|-------------|
| `daytona_create_sandbox` | Create and start a new sandbox |
| `daytona_exec` | Execute shell commands |
| `daytona_read_file` | Read files from sandbox |
| `daytona_write_file` | Write files to sandbox |
| `daytona_download_workspace` | Download workspace to session storage |
| `daytona_list_sandboxes` | List active sandboxes |
| `daytona_manage_sandbox` | Stop or delete sandboxes |
| `daytona_git_clone` | Clone repositories (auto-authenticates with GitHub) |
| `daytona_git_credentials` | Configure git push/pull credentials |

## Git Integration

Daytona sandboxes integrate with your connected GitHub account:

- **Clone private repos**: `daytona_git_clone` automatically uses your GitHub credentials
- **Push/pull/fetch**: Call `daytona_git_credentials` once after creating a sandbox, then use `daytona_exec` for any git command
- **Shorthand syntax**: Use `user/repo` format instead of full URLs

## Sandbox Lifecycle

Sandboxes auto-stop after 5 minutes of inactivity as a safety net. Best practice is to explicitly delete sandboxes when done, stopping only pauses them (they remain visible on your Daytona dashboard).

## Security

- API keys are encrypted at rest (AES-256-GCM envelope encryption)
- Each sandbox is fully isolated from other sandboxes and the host
- Git credentials are short-lived and scoped to the sandbox
- Sandbox state is stored in encrypted session secrets

## Links

- [Daytona Website](https://www.daytona.io/)
- [Daytona Dashboard](https://app.daytona.io)
- [Daytona Documentation](https://www.daytona.io/docs)

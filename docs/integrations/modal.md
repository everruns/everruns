---
title: Modal
description: Modal sandboxes for code execution, full Linux VMs with their own kernel or lighter gVisor containers, with filesystem snapshots and public tunnels.
appliesTo: [platform]
---

> **Status:** Experimental. No feature flag: adding the capability and a Modal connection is the opt-in. Modal is not part of the supported sandbox set and may change or be removed.

Everruns integrates with [Modal](https://modal.com/) sandboxes. By default an agent gets a [VM sandbox](https://modal.com/blog/vm-sandboxes-agent-computers): a full Linux virtual machine with its own kernel, so workloads that need real kernel features (Docker, FUSE, databases, system services) run as they would on a server. The lighter gVisor container runtime is available too.

## What You Get

- **Full VMs or containers**: `runtime: "vm"` (default) boots a real kernel; `runtime: "gvisor"` starts a lighter container
- **Any registry image**: start from a public image (default `python:3.13-slim` with git and curl added), optionally with Dockerfile setup commands
- **Filesystem snapshots**: save a sandbox's filesystem as an image and boot copies of it later
- **Public tunnels**: expose ports and get HTTPS URLs for them
- **Multiple sandboxes per session**, each cleaned up automatically when the session lease expires

## Quick Start

### 1. Create a Modal token

1. Sign in at [modal.com](https://modal.com/)
2. Open [Settings > API Tokens](https://modal.com/settings/tokens) and create a token
3. Copy the token ID (`ak-...`) and token secret (`as-...`)

### 2. Connect in Everruns

1. Go to **Settings** > **Connections**
2. Find **Modal** in the available providers
3. Click **Connect** and paste the token as `ak-...:as-...`

The form also accepts the `modal token set --token-id ak-... --token-secret as-...` command Modal shows when the token is created. Everruns checks the token with Modal before saving it.

### 3. Use in Sessions

Add the **Modal** capability to an agent. Agents with it can use these tools:

| Tool | Description |
|------|-------------|
| `modal_create_sandbox` | Create a VM or gVisor sandbox from an image, setup commands, or a snapshot |
| `modal_exec` | Run a shell command (`sh -c`) and return stdout, stderr and the exit code |
| `modal_read_file` | Read a text file from the sandbox |
| `modal_write_file` | Write a text file into the sandbox, creating parent directories |
| `modal_list_sandboxes` | List sandboxes created in this session |
| `modal_manage_sandbox` | Check a sandbox's status or terminate it |
| `modal_snapshot_sandbox` | Snapshot the filesystem into an image (`im-...`) |
| `modal_tunnel_urls` | Get the public HTTPS URLs for exposed ports |

Commands and relative file paths start in `/workspace` inside the sandbox.

## Creating Sandboxes

`modal_create_sandbox` accepts:

| Parameter | Meaning |
|---|---|
| `runtime` | `vm` (default) or `gvisor` |
| `image` | Registry image, for example `node:22` |
| `setup_commands` | Up to 20 Dockerfile lines (`RUN ...`) applied on top of the image |
| `snapshot_image_id` | Boot from a snapshot instead of an image |
| `timeout_seconds` | Maximum lifetime, 60 to 86400 (default 3600) |
| `idle_timeout_seconds` | Terminate after this long without activity |
| `cpu`, `memory_mb` | Resources to request |
| `expose_ports` | Up to 10 ports to publish through tunnels |

Modal caches built images per workspace, so the first sandbox from a new image takes longer than the ones after it.

## Network and GitHub access

Pass `network` to `modal_create_sandbox` to control outbound traffic. Modal enforces it outside the sandbox:

- `{"mode": "blocked"}`: no outbound network
- `{"mode": "allowlist", "domains": ["pypi.org", "*.pythonhosted.org"], "cidrs": ["10.0.0.0/8"]}`: only those destinations
- `{"mode": "open"}`: anything (the default)

Pass `"inject_connections": ["github"]` to let the sandbox use your GitHub connection without exposing the token. Modal adds it to requests for `api.github.com` and to git over `https://github.com`, so `curl https://api.github.com/user` and `git clone` of private repositories work, but the token never appears in the sandbox environment or files. Injection needs open network or a CIDR-only allowlist.

In Sandbox Templates, the template's network setting (Open, Only listed domains, Blocked) is enforced the same way, and the **Use my GitHub connection** switch turns on injection.

## Snapshots

1. Call `modal_snapshot_sandbox` to save the current filesystem. The sandbox keeps running.
2. Pass the returned `im-...` ID as `snapshot_image_id` to `modal_create_sandbox` to boot a copy with that state.

## Tunnels

1. Create the sandbox with `expose_ports`, for example `[8080]`
2. Start a server listening on `0.0.0.0` at that port
3. Call `modal_tunnel_urls` to get the public HTTPS URL

## Sandbox Templates

On deployments that offer it, Modal is also a **managed** target in Sandbox Templates. Pick **Modal** under **Runs in** and choose the runtime (VM or gVisor), an image, CPU cores, memory and a workspace path (default `/workspace`). The agent then uses the standard sandbox tools instead of the `modal_*` tools, and Everruns manages the sandbox for the session.

When the session goes idle, Everruns snapshots the sandbox filesystem and terminates the sandbox. The next command boots a new sandbox from that snapshot, so files survive but running processes do not. Recovery for Modal templates is `provider_snapshot`.

## Lifecycle

Sandboxes run until the agent terminates them, their `timeout_seconds` or idle timeout passes, or the session's lease on them expires. Everruns terminates leased sandboxes when the lease expires, so an abandoned session does not keep one running. Modal bills running sandboxes per second; see [Modal pricing](https://modal.com/pricing).

## Security

- **Isolation**: VM sandboxes run their own kernel; gVisor sandboxes run behind a user-space kernel
- **Encrypted credentials**: the token is stored in user connections, encrypted at rest
- **Session ownership**: tools only act on sandboxes this session created
- **Network access**: sandboxes have open outbound access unless you block or allowlist it, and exposed ports are public. Treat anything served on a tunnel as public
- **Injected credentials**: injected tokens go only to the service's own domains, and the sandbox never holds them

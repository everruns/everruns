---
title: Container Sandbox
description: Self-hosted container sandboxes for secure, isolated code execution via Docker Engine. No external SaaS dependency.
---

<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="52.0" height="52.0" fill="currentColor" aria-hidden="true" style="float: right; margin-left: 16px;"><path d="M13.983 11.078h2.119a.186.186 0 00.186-.185V9.006a.186.186 0 00-.186-.186h-2.119a.185.185 0 00-.185.185v1.888c0 .102.083.185.185.185m-2.954-5.43h2.118a.186.186 0 00.186-.186V3.574a.186.186 0 00-.186-.185h-2.118a.185.185 0 00-.185.185v1.888c0 .102.082.185.185.185m0 2.716h2.118a.187.187 0 00.186-.186V6.29a.186.186 0 00-.186-.185h-2.118a.185.185 0 00-.185.185v1.887c0 .102.082.185.185.186m-2.93 0h2.12a.186.186 0 00.184-.186V6.29a.185.185 0 00-.185-.185H8.1a.185.185 0 00-.185.185v1.887c0 .102.083.185.185.186m-2.964 0h2.119a.186.186 0 00.185-.186V6.29a.185.185 0 00-.185-.185H5.136a.186.186 0 00-.186.185v1.887c0 .102.084.185.186.186m5.893 2.715h2.118a.186.186 0 00.186-.185V9.006a.186.186 0 00-.186-.186h-2.118a.185.185 0 00-.185.185v1.888c0 .102.082.185.185.185m-2.93 0h2.12a.185.185 0 00.184-.185V9.006a.185.185 0 00-.184-.186h-2.12a.185.185 0 00-.184.185v1.888c0 .102.083.185.185.185m-2.964 0h2.119a.185.185 0 00.185-.185V9.006a.185.185 0 00-.184-.186h-2.12a.186.186 0 00-.186.186v1.887c0 .102.084.185.186.185m-2.92 0h2.12a.185.185 0 00.184-.185V9.006a.185.185 0 00-.184-.186h-2.12a.185.185 0 00-.184.185v1.888c0 .102.082.185.185.185M23.763 9.89c-.065-.051-.672-.51-1.954-.51-.338.001-.676.03-1.01.087-.248-1.7-1.653-2.53-1.716-2.566l-.344-.199-.226.327c-.284.438-.49.922-.612 1.43-.23.97-.09 1.882.403 2.661-.595.332-1.55.413-1.744.42H.751a.751.751 0 00-.75.748 11.376 11.376 0 00.692 4.062c.545 1.428 1.355 2.48 2.41 3.124 1.18.723 3.1 1.137 5.275 1.137.983.003 1.963-.086 2.93-.266a12.248 12.248 0 003.823-1.389c.98-.567 1.86-1.288 2.61-2.136 1.252-1.418 1.998-2.997 2.553-4.4h.221c1.372 0 2.215-.549 2.68-1.009.309-.293.55-.65.707-1.046l.098-.288Z"/></svg>

Everruns provides self-hosted container sandboxes via Docker Engine for secure, isolated code execution. Agents can create, manage, and interact with multiple containers per session, each an isolated Linux environment with real filesystem, process execution, and network access.

## What You Get

- **Self-Hosted**: Runs on your own infrastructure, no SaaS dependency
- **Isolated Containers**: Each sandbox is an isolated Linux container with cgroup resource limits
- **Multi-Sandbox Sessions**: Create and manage multiple containers within a single session
- **File Operations**: Read, write, upload, and download files between session storage and containers
- **Shell Execution**: Run arbitrary commands with stdout/stderr/exit_code capture

## Quick Start

### 1. Docker Engine Access

Ensure Docker Engine is accessible from the server/worker. The capability communicates via Docker Engine REST API (not the CLI).

- **Local**: Docker Desktop or `dockerd` on the host (default socket: `/var/run/docker.sock`)
- **Remote**: TCP or TCP+TLS endpoint (e.g., `http://10.0.0.3:2375`)

Set `CONTAINER_SANDBOX_DOCKER_HOST` to override the default Docker host.

### 2. Enable the Feature and Assign the Capability

Set `FEATURE_CONTAINER_SANDBOX=true` anywhere capabilities are registered or executed to enable the capability and built-in **Coding (Container)** harness. In most deployments, that means both the server and any workers.

For legacy deployments, `FEATURE_DOCKER_CAPABILITY=true` still enables the same feature until operators switch to the new flag name, and it must be enabled in the same places.

Once the flag is enabled in the relevant processes, add the `container_sandbox` capability to a custom harness or use the built-in **Coding (Container)** harness.

### 3. Use in Sessions

Agents with the Container Sandbox capability can use these tools:

| Tool | Description |
|------|-------------|
| `sandbox_create` | Create and start a new container with resource limits |
| `sandbox_exec` | Execute shell commands in a container |
| `sandbox_read_file` | Read files from container filesystem |
| `sandbox_write_file` | Write files to container filesystem |
| `sandbox_upload` | Copy files from session storage into container |
| `sandbox_download` | Copy files from container to session storage |
| `sandbox_list` | List active containers in the session |
| `sandbox_manage` | Stop, start, or remove containers |

## Container Lifecycle

1. **Create**: `sandbox_create` pulls the image, creates an isolated network, and starts the container
2. **Use**: `sandbox_exec`, `sandbox_read_file`, `sandbox_write_file` for coding work
3. **Transfer**: `sandbox_upload`/`sandbox_download` to move files between session storage and container
4. **Remove**: `sandbox_manage` with action "remove" deletes the container and network

Containers auto-stop after 10 minutes of inactivity (configurable). Leased resource cleanup handles abandoned containers after 20 minutes.

## Configuration

| Parameter | Default | Description |
|-----------|---------|-------------|
| `docker_host` | Auto-detect | Docker Engine endpoint |
| `runtime` | `runc` | Container runtime (`runc`, `sysbox-runc`, `kata`, `gvisor`) |
| `image` | `ubuntu:24.04` | Default container image |
| `memory_limit` | 2 GiB | Memory limit per container |
| `cpu_limit` | 1 core | CPU limit per container |
| `pids_limit` | 256 | Max processes per container |

## Security

- Each container runs in its own isolated Docker network
- Resource limits (memory, CPU, PIDs) enforced via cgroups
- Container names derived from session ID, no cross-session access
- Docker socket never mounted into containers
- Configurable runtime: use `sysbox-runc` for user-namespace isolation in production

## Links

- [Docker Engine API Reference](https://docs.docker.com/engine/api/)
- [Sysbox Runtime](https://github.com/nestybox/sysbox)

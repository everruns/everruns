---
title: Container Sandbox
description: Self-hosted container sandboxes for secure, isolated code execution via Docker Engine. No external SaaS dependency.
---

Everruns provides self-hosted container sandboxes via Docker Engine for secure, isolated code execution. Agents can create, manage, and interact with multiple containers per session — each an isolated Linux environment with real filesystem, process execution, and network access.

## What You Get

- **Self-Hosted**: Runs on your own infrastructure — no SaaS dependency
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

### 2. Assign the Capability

Add the `container_sandbox` capability to a harness or use the built-in **Coding (Container)** harness which includes it by default.

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
- Container names derived from session ID — no cross-session access
- Docker socket never mounted into containers
- Configurable runtime: use `sysbox-runc` for user-namespace isolation in production

## Links

- [Docker Engine API Reference](https://docs.docker.com/engine/api/)
- [Sysbox Runtime](https://github.com/nestybox/sysbox)

---
title: Container Sandbox
description: Self-hosted Linux container per session through the Docker Engine API, with shell execution, file I/O, and file transfer to session storage.
appliesTo: [platform]
---

| | |
|---|---|
| **ID** | `container_sandbox` |
| **Category** | Execution |
| **Features** | `leased_resources` |
| **Dependencies** | [Storage](/capabilities/session-storage/) |
| **Risk** | High |

Container Sandbox gives an agent a Linux container to run code in. The platform
talks to a Docker Engine you operate over its REST API, so no sandbox vendor is
involved. Each session gets at most one container, on its own Docker network.

**Off by default.** Set `FEATURE_CONTAINER_SANDBOX=prod` on the server and on
every worker to register the capability and enable it by default for organisations.
Use `dev` for local development or `preview`/`adoption` for explicit enrolment.
Each capability has its own grade; the Docker flag does not enable the sandbox.

## Setup

1. Run a Docker Engine the server and workers can reach.
2. Set `CONTAINER_SANDBOX_DOCKER_HOST` to its `http://` or `https://` URL, on
   the server and every worker.
3. Set `FEATURE_CONTAINER_SANDBOX=prod` in the same places.
4. Add `container_sandbox` to a custom agent or harness. For coding work,
   inherit [Generic](/built-ins/harnesses/generic/) and optionally add
   [GitHub Scout](/capabilities/github-scout/).

Without `CONTAINER_SANDBOX_DOCKER_HOST` the client defaults to
`unix:///var/run/docker.sock`. The client cannot use a Unix socket yet, so every
tool call then fails with an error asking for an `http(s)://` host. This is
deliberate: it never falls back to an unauthenticated TCP endpoint. Use
`https://` with mutual TLS for any Docker daemon that is not on loopback, and
never expose a plaintext Docker port on a network.

## Tools

| Tool | Parameters | What it does |
|---|---|---|
| `sandbox_create` | `image` (optional) | Creates the session's network and container and starts it |
| `sandbox_exec` | `command`, `working_dir`, `output` | Runs a shell command and returns stdout, stderr, and the exit code |
| `sandbox_read_file` | `path`, `offset`, `limit` | Reads a text file as a line window (`limit` defaults to 2000 lines). For a binary file it returns only the size |
| `sandbox_write_file` | `path`, `content` | Writes text to a file, replacing it in full |
| `sandbox_upload` | `session_path`, `container_path` | Copies a file from session storage into the container, byte for byte |
| `sandbox_download` | `container_path`, `session_path` | Copies a file out of the container into session storage, where it outlives the container |
| `sandbox_list` | none | Lists the session's sandbox with its state |
| `sandbox_manage` | `action`: `stop`, `start`, or `remove` | `stop` keeps the filesystem, `start` runs it again, `remove` deletes the container, its network, and every file that was not downloaded |

The capability's system prompt describes the lifecycle
(`sandbox_create`, then exec and file tools, then `sandbox_manage` with
`remove`) and tells the agent to remove the sandbox when it is done.

## Container defaults

| Setting | Value |
|---|---|
| Image | `ubuntu:24.04`, unless `sandbox_create` passes `image` |
| Working directory | `/workspace` |
| Memory limit | 2 GiB |
| CPU limit | 1 CPU |
| Process limit | 256 |
| Runtime | The Docker daemon's default runtime (normally `runc`) |
| Network | A bridge network per session, named after the session |

These values are fixed in the platform. The capability takes no configuration,
and the image is the only setting a tool call can change. Exec output and file
transfers are capped at 8 MiB per response, and a single file transfer at
4 MiB.

Each create, exec, and start refreshes a 20-minute lease on the container in
the session's leased resources.

## Security

- Each session's container runs on its own Docker network, and container and
  network names are derived from the session ID.
- Memory, CPU, and process limits are applied through cgroups.
- The Docker socket is never mounted into a container.
- No egress filtering is applied. A container can reach whatever its bridge
  network routes to, including private address ranges and cloud metadata
  endpoints. Restrict egress at the network or firewall layer.
- `runc` shares the host kernel. For untrusted or multi-tenant workloads,
  configure a hardened default runtime on the Docker daemon, such as
  `sysbox-runc`, gVisor, or Kata.

## See also

- [Daytona](/capabilities/daytona/) and [E2B](/capabilities/e2b/), hosted sandbox providers
- [Storage](/capabilities/session-storage/), where uploaded and downloaded files live
- [Docker Container](/capabilities/docker/), an older experimental Docker capability

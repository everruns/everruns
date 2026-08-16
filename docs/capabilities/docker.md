---
title: Docker Container Sandbox
description: Run agent commands and manage files in a Docker container tied to the session. Self-hosted alternative to cloud sandbox providers.
sidebar:
  label: Docker Container
---

| | |
|---|---|
| **ID** | `docker_container` |
| **Category** | Sandboxes |
| **Features** | None |
| **Dependencies** | None |

Run commands and manage files in a Docker container tied to the session. The container is lazily started on first use and persists for the session duration. A self-hosted alternative to cloud sandbox providers like Daytona or E2B.

> **Experimental:** This capability may change significantly in future releases.

## Tools

### `docker_exec`

Execute a command inside the Docker container.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `command` | string | yes | Shell command to run |
| `cwd` | string | no | Working directory |
| `timeout_ms` | integer | no | Timeout in milliseconds |

Returns stdout, stderr, and exit code. Container is started automatically if not already running.

### `docker_read_file`

Read a text file from the container filesystem.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `path` | string | yes | File path |
| `offset` | integer | no | Line offset to start reading from |
| `limit` | integer | no | Maximum number of lines to return |

### `docker_write_file`

Write a text file into the container filesystem.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `path` | string | yes | File path |
| `content` | string | yes | File content |

### `docker_logs`

Retrieve recent logs from the Docker container.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `tail` | integer | no | Number of log lines to return |

### `docker_stop`

Stop the Docker container for this session.

## Configuration

Configure the Docker container via the capability settings:

| Setting | Description |
|---|---|
| `image` | Docker image to use (e.g. `ubuntu:24.04`) |
| `env` | Environment variables to inject |
| `binds` | Host path mounts |

## Notes

- Only one container runs per session
- The container is stopped when the session ends or `docker_stop` is called
- Docker Engine must be accessible from the server running Everruns

## See Also

- [Container Sandbox integration guide](/integrations/container-sandbox/), setup and configuration
- [Capabilities Overview](/capabilities/)

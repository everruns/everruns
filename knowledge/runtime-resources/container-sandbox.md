---
type: Specification
title: "Container Sandbox Capability Specification"
description: "Self-hosted Docker execution for agent sessions."
tags:
  - everruns
  - runtime-resources
  - sandbox
---
# Container Sandbox Capability Specification

## Abstract

Self-hosted container-based agent execution via Docker Engine REST API. Provides real filesystem, full process execution, and network access in isolated Docker containers. Fills the gap between the bashkit shell (safe but limited) and cloud sandboxes like Daytona/E2B (powerful but SaaS dependency).

**Status**: Available when the internal `container_sandbox` feature flag is enabled (`FEATURE_CONTAINER_SANDBOX=true`, with legacy fallback from `FEATURE_DOCKER_CAPABILITY=true`) and Docker Engine access is configured

## Architecture

### Docker Engine REST API

The capability communicates with Docker Engine via its REST API (v1.47) using `reqwest`. Supports Unix socket, TCP, and TCP+TLS transports. No dependency on the `docker` CLI binary.

See `crates/platform/src/container_sandbox/client.rs` for the full API client implementation.

### State Management

Per-sandbox state (container ID, network ID, image, working directory) is stored in session-scoped secrets under `container_sandbox:{name}`. This ensures cross-session isolation, sessions cannot access each other's containers.

Leased resources track container lifecycle with 20-minute lease duration. The leased resource scheduler handles cleanup of abandoned containers.

See `crates/platform/src/container_sandbox/state.rs` for state serialization and lease management.

### Configuration

```json
{
  "docker_host": "http://10.0.0.3:2375",
  "runtime": "sysbox-runc",
  "image": "ubuntu:24.04",
  "memory_limit": "2g",
  "cpu_limit": "1",
  "pids_limit": 256,
  "network_mode": "bridge"
}
```

Environment variables `CONTAINER_SANDBOX_DOCKER_HOST` and `CONTAINER_SANDBOX_RUNTIME` provide deployment-level defaults. OSS works with plain Docker (runc) out of the box.

See `crates/platform/src/container_sandbox/config.rs` for full configuration fields and defaults.

## Tools

| Tool | Description |
|------|-------------|
| `sandbox_create` | Create container from image with resource limits and isolated network |
| `sandbox_exec` | Execute command in container, return stdout/stderr/exit_code |
| `sandbox_read_file` | Read file from container filesystem via Docker archive API |
| `sandbox_write_file` | Write content to file in container via Docker archive API |
| `sandbox_upload` | Copy file from session VFS into container |
| `sandbox_download` | Copy file from container into session VFS |
| `sandbox_list` | List active containers in current session |
| `sandbox_manage` | Stop/start/remove container |

See `crates/platform/src/container_sandbox/tools.rs` for full tool implementations including parameter schemas, return types, and error handling.

## Multi-Tenant Isolation (6 layers)

1. **Tool scoping**: container name derived from `ToolContext.session_id`, never user input
2. **Per-sandbox Docker network**: `sandbox-{org}-{session}`, sole member = the sandbox
3. **Label-filtered API calls**: all queries include `session` + `managed-by` labels
4. **Per-org limits**: max concurrent sandboxes checked at create time via leased resources
5. **Egress filtering**: block private IPs + cloud metadata from sandbox bridges
6. **Runtime isolation**: configurable (sysbox adds user-ns + procfs virtualization)

## Security

See `knowledge/security/threat-model.md#20-container-sandbox-tm-sandbox` for full threat analysis (TM-SANDBOX-001 through TM-SANDBOX-010).

## Design Decisions

### Platform-owned module, not integration

Located at `crates/platform/src/container_sandbox/`, not `integrations/`. This
is opt-in deployment infrastructure (self-hosted Docker), not an external SaaS
service or a neutral execution-kernel concern.

### REST API over CLI

Uses Docker Engine REST API via `reqwest` instead of shelling out to the `docker` CLI. Avoids binary dependency, enables typed error handling, and matches the Daytona client pattern.

### Runtime-agnostic

The container runtime (`runc`, `sysbox-runc`, `kata`, `gvisor`) is a deployment-time config field. Code doesn't assume a specific runtime, isolation guarantees scale with runtime choice.

## Capability Registration

- **ID**: `container_sandbox`
- **Name**: Container Sandbox
- **Category**: Execution
- **Feature flag**: `container_sandbox`
- **Dependencies**: None (Docker Engine is infrastructure, not a user connection)

## Module Structure

```
crates/platform/
├── src/container_sandbox/
│   ├── mod.rs           # ContainerSandboxCapability + tool registration
│   ├── client.rs        # Docker Engine REST API client
│   ├── config.rs        # Configuration with env var defaults
│   ├── state.rs         # Sandbox state + leased resources
│   └── tools.rs         # All 8 tool implementations
└── tests/
    └── container_sandbox_live_api_test.rs # Feature-gated Docker API tests
```

## Live API tests

`crates/platform/tests/container_sandbox_live_api_test.rs` is the canonical live-test entrypoint. It is gated behind the `container-sandbox-live-tests` feature and expects an unauthenticated Docker daemon reachable through `CONTAINER_SANDBOX_DOCKER_HOST`. The dedicated workflow runs it against `docker:dind` when the platform module changes; the live sweep reruns it weekly and on demand.

# Container Sandbox Capability Specification

## Abstract

The `container_sandbox` capability provides self-hosted container-based agent execution via the Docker Engine REST API. Agents create, exec, and manage isolated Linux containers per session. The container runtime is configurable at deployment time — works with standard `runc` (Docker default) or stronger isolation runtimes like `sysbox-runc`, `kata-runtime`, or `gvisor`.

**Status**: Planned

## Motivation

Everruns has a gap between virtual bash (safe but no real processes) and cloud sandboxes like Daytona/E2B (full power but SaaS dependency). The existing Docker integration is dev-only (host networking, no user-namespace). This capability fills the gap: self-hosted containers with configurable isolation, bridge networking, and cgroup resource limits.

| Tier | Isolation | Real Processes | Self-Hosted | Production-Ready |
|------|-----------|---------------|-------------|-----------------|
| Virtual Bash | WASM-like | No | N/A | Yes |
| **Container Sandbox** | **Configurable (runc → sysbox)** | **Yes** | **Yes** | **Yes** |
| Docker (experimental) | Weak (host net) | Yes | Yes | No |
| Daytona / E2B | Full VM | Yes | No (SaaS) | Yes |

## Architecture

### Docker Engine REST API (not CLI)

The capability communicates with Docker via HTTP REST API using `reqwest`. No `docker` CLI binary needed. This follows the Daytona pattern (HTTP client → REST API → sandbox) rather than the existing Docker integration pattern (`Command::new("docker")`).

```
Worker container                    Docker Host
┌──────────────────┐     ┌──────────────────────────────┐
│ Agent Loop       │     │ Docker Engine REST API        │
│  → sandbox_exec  │     │  (socket proxy or TCP:2376)   │
│    → reqwest ────────► │                              │
│      HTTP        │     │  Container Runtime            │
└──────────────────┘     │  (runc / sysbox / kata / ...)│
                         │                              │
                         │ ┌──────────────────────────┐ │
                         │ │ Sandbox Container         │ │
                         │ │  bridge network           │ │
                         │ │  cgroup-limited           │ │
                         │ └──────────────────────────┘ │
                         └──────────────────────────────┘
```

Workers reach the Docker daemon via:
- **Socket proxy** (e.g., Tecnativa/docker-socket-proxy) — restricts API surface, workers stay containerized
- **TCP with TLS** (`daemon.json` `hosts: ["tcp://0.0.0.0:2376"]`) — no socket mount, cert-based auth
- **Local socket** (`unix:///var/run/docker.sock`) — for local dev / OSS single-host deployments

### Runtime Agnosticism

The `runtime` field in config is passed to Docker's `HostConfig.Runtime` on container create. The capability code is identical regardless of runtime. Isolation level is a deployment decision:

| Runtime | Isolation | Performance | Install |
|---------|-----------|-------------|---------|
| `""` (default / runc) | Standard container | 100% native | Docker default |
| `sysbox-runc` | VM-like (user-ns, procfs virtualized) | ~95-98% | `apt install sysbox-ce` |
| `kata-runtime` | Full VM per container | ~90% | Kata Containers |
| `runsc` (gVisor) | User-space kernel | ~70-90% (I/O bound) | gVisor install |

### State Management

Per-sandbox state stored in session **secrets** (encrypted at rest), same pattern as Daytona:

- Secret key: `container_sandbox:{container_id}`
- Value: JSON `SandboxState` (container_id, image, created_at, docker_host)

### Docker Host Resolution

Priority: capability config → env var → default.

| Source | Variable | Example |
|--------|----------|---------|
| Env var | `CONTAINER_SANDBOX_DOCKER_HOST` | `http://10.0.0.3:2375` |
| Capability config | `docker_host` | `http://sandbox-host:2375` |
| Default | — | `unix:///var/run/docker.sock` |

### Container Runtime Resolution

| Source | Variable | Example |
|--------|----------|---------|
| Env var | `CONTAINER_SANDBOX_RUNTIME` | `sysbox-runc` |
| Capability config | `runtime` | `sysbox-runc` |
| Default | — | `""` (Docker default = runc) |

## Data Model

### SandboxState (persisted per sandbox)

| Field | Type | Description |
|-------|------|-------------|
| `container_id` | String | Docker container ID (short hash) |
| `container_name` | String | `evr-{session_uuid}-sysbox` |
| `image` | String | Image used to create sandbox |
| `network_id` | String | Docker network ID for this sandbox |
| `docker_host` | String | Docker API endpoint used |
| `created_at` | String (ISO 8601) | Creation time |

### ContainerSandboxConfig

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `docker_host` | String | env / socket | Docker Engine API endpoint |
| `runtime` | String | `""` | OCI runtime (e.g., `sysbox-runc`) |
| `image` | String | `ubuntu:24.04` | Container base image |
| `memory_limit` | String | `2g` | Hard memory limit |
| `cpu_limit` | String | `1` | CPU quota |
| `pids_limit` | u64 | `256` | Max process count |
| `working_dir` | String | `/workspace` | Default working directory |
| `network_mode` | String | `bridge` | Docker network mode |
| `auto_stop_minutes` | u64 | `10` | Inactivity auto-stop |
| `allowed_ports` | Vec<u16> | `[]` | Ports to forward |

## Docker Engine API Integration

### Container Lifecycle

| Operation | HTTP Method | Docker API Endpoint | Purpose |
|-----------|-------------|-------------------|---------|
| Create network | POST | `/networks/create` | Per-sandbox isolated bridge network |
| Create container | POST | `/containers/create` | Create sandbox with runtime + limits |
| Start container | POST | `/containers/{id}/start` | Start after creation |
| Exec create | POST | `/containers/{id}/exec` | Create exec instance for command |
| Exec start | POST | `/exec/{id}/start` | Run command, stream output |
| Exec inspect | GET | `/exec/{id}/json` | Get exit code |
| Get archive | GET | `/containers/{id}/archive?path=...` | Read file (tar stream) |
| Put archive | PUT | `/containers/{id}/archive?path=...` | Write file (tar stream) |
| Stop container | POST | `/containers/{id}/stop` | Graceful stop |
| Remove container | DELETE | `/containers/{id}` | Remove container |
| Remove network | DELETE | `/networks/{id}` | Remove sandbox network |

### Container Create Request

```json
{
  "Image": "ubuntu:24.04",
  "Cmd": ["tail", "-f", "/dev/null"],
  "WorkingDir": "/workspace",
  "Labels": {
    "managed-by": "everruns",
    "org": "{org_public_id}",
    "session": "{session_id}"
  },
  "HostConfig": {
    "Runtime": "sysbox-runc",
    "NetworkMode": "sandbox-{org}-{session}",
    "Memory": 2147483648,
    "NanoCpus": 1000000000,
    "PidsLimit": 256,
    "Init": true
  }
}
```

## Tool Surface

| Tool | Description | Hints |
|------|-------------|-------|
| `sandbox_create` | Create sandbox container from image | `destructive: false` |
| `sandbox_exec` | Execute command, return stdout/stderr/exit_code | `long_running: true`, `persist_output: true`, `open_world: true` |
| `sandbox_read_file` | Read file from sandbox filesystem | `readonly: true` |
| `sandbox_write_file` | Write content to file in sandbox | `destructive: false` |
| `sandbox_upload` | Copy file from session VFS into sandbox | `destructive: false` |
| `sandbox_download` | Copy file from sandbox into session VFS | `readonly: true` |
| `sandbox_list` | List active sandboxes in session | `readonly: true` |
| `sandbox_manage` | Stop/start/remove sandbox | `destructive: true` |

## Multi-Tenant Isolation

### Container Naming

`evr-{session_uuid}-sysbox` — derived from `ToolContext.session_id`, never from user input.

### Per-Sandbox Docker Network

Each sandbox gets its own Docker bridge network: `sandbox-{org_public_id}-{session_uuid}`. The container is the sole member. Cross-tenant L3 traffic is impossible.

### Label-Filtered API Calls

All Docker API queries include label filters: `{"label":["session={session_id}","managed-by=everruns"]}`. Defense-in-depth against tool code bugs.

### Per-Org Limits

Enforced at `sandbox_create` time:
- Max concurrent sandboxes per org (configurable, e.g., 3 free / 10 paid)
- Checked via leased resource count query

## Leased Resources

Follow Daytona pattern. Lease registered on create, refreshed on each tool call, cleaned up by durable scheduler.

```rust
LeasedResource {
    provider: "container_sandbox",
    resource_type: "container",
    external_id: container_name,
    lease_expires_at: now + Duration::from_secs(auto_stop_minutes * 60),
    metadata: json!({
        "image": image,
        "docker_host": docker_host,
        "network_id": network_id
    }),
}
```

Cleanup handler: stop container → remove container → remove network.

## Crate Location

`crates/container-sandbox/` — core execution crate, same pattern as `crates/session-sqldb/`. Not in `integrations/` because this is core infrastructure (Docker Engine on your own host), not an external service integration.

## Capability Registration

```rust
inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,
        factory: || Box::new(ContainerSandboxCapability),
    }
}

impl Capability for ContainerSandboxCapability {
    fn id(&self) -> &str { "container_sandbox" }
    fn name(&self) -> &str { "Container Sandbox" }
    fn risk_level(&self) -> RiskLevel { RiskLevel::High }
    fn icon(&self) -> Option<&str> { Some("container") }
    fn category(&self) -> Option<&str> { Some("Execution") }
    fn dependencies(&self) -> Vec<&'static str> { vec!["session_storage"] }
    fn features(&self) -> Vec<&'static str> { vec![LEASED_RESOURCES_FEATURE] }
}
```

## Harness

Built-in `coding-container` harness:

| Property | Value |
|----------|-------|
| Name | `coding-container` |
| Display Name | Coding (Container) |
| Parent | `generic` |
| Additional capability | `container_sandbox` |

Same two-level execution as `coding-daytona`: workspace VFS for lightweight ops, container sandbox for real builds/tests/services.

## Threat Model

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-SANDBOX-001 | Container escape via kernel vulnerability | High | Configurable runtime (sysbox adds user-ns); kernel patching | ACCEPTED |
| TM-SANDBOX-002 | Resource exhaustion (CPU/memory/disk) | High | cgroup limits via Docker flags; `--pids-limit` | MITIGATED |
| TM-SANDBOX-003 | Network attacks from sandbox | High | Per-sandbox bridge network; egress filtering; `none` mode | MITIGATED |
| TM-SANDBOX-004 | Cross-session container access | High | Session-scoped naming; label-filtered API queries | MITIGATED |
| TM-SANDBOX-005 | Image supply chain (malicious images) | Medium | Curated allowlist in config; image pull policy | MITIGATED |
| TM-SANDBOX-006 | Docker socket exposure inside sandbox | High | No socket mount; DinD uses inner daemon (sysbox) or disabled | MITIGATED |
| TM-SANDBOX-007 | Stale container not cleaned up | Medium | Leased resource scheduler; auto_stop_minutes; Docker `--rm` | MITIGATED |
| TM-SANDBOX-008 | Cross-tenant sandbox access via Docker API | Critical | Tool scoping by session_id; per-sandbox network; label filters | MITIGATED |
| TM-SANDBOX-009 | Cross-tenant network reachability | High | Per-sandbox isolated bridge; no shared network | MITIGATED |
| TM-SANDBOX-010 | Tenant resource starvation | High | Per-sandbox cgroups + per-org max sandbox count | MITIGATED |

## Output Sanitization

Same pipeline as existing exec tools: strip ANSI → collapse CR lines → middle-truncate at 16 KiB. Full output persisted to `/.outputs/` via `tool_output_persistence` capability.

## Testing

### Unit Tests

- Plugin registration, capability metadata
- Container name derivation from session_id
- Config parsing (defaults, env override, capability config override)
- Label construction
- Docker API request body serialization

### Integration Tests (`tests/tool_integration.rs`)

- Tool `execute_with_context` flows against wiremock (mock Docker API)
- Create sandbox → exec → read file → write file → stop → cleanup
- Per-org limit enforcement
- Network creation and cleanup
- Leased resource registration and cleanup

### Live Tests (`tests/live_api_test.rs`)

- Feature-gated (`container-sandbox-live-tests`)
- Requires Docker Engine accessible at `CONTAINER_SANDBOX_DOCKER_HOST`
- Full lifecycle: create → exec → file I/O → stop → remove

# Sysbox Sandbox Support Analysis

## Abstract

Analysis of integrating [Sysbox](https://github.com/nestybox/sysbox) as a self-hosted sandbox runtime for Everruns agent execution. Sysbox is an OCI container runtime (runc fork) that provides VM-like isolation without hardware virtualization — user-namespace remapping, procfs/sysfs virtualization, and syscall interception. Originally by Nestybox (acquired by Docker, 2022), now community-maintained.

This document evaluates the fit against Everruns' existing execution model, proposes an integration architecture, and identifies risks and trade-offs versus the current sandbox options.

## Motivation

Everruns currently offers three execution tiers:

| Tier | Isolation | Network | Where | Trade-off |
|------|-----------|---------|-------|-----------|
| Virtual Bash (bashkit) | WASM-like, in-process | None | Worker process | Safe but limited — no real processes, no packages, no services |
| Cloud sandboxes (Daytona/E2B) | Full Linux VM | Full | Remote SaaS | Full power but external dependency, latency, cost per sandbox |
| Docker (experimental) | Standard container, host networking | Full | Worker host | Weak isolation — `--network host`, no user-namespace, dev-only |

**Gap**: No self-hosted, strongly-isolated execution environment that supports real Linux processes (apt, pip, git, build tools, dev servers) without depending on a cloud sandbox provider.

Sysbox fills this gap: containers with VM-like isolation, running on infrastructure you control, at 2-3x the density of actual VMs.

## What Sysbox Provides

### Core Isolation Mechanisms

1. **Mandatory user-namespace**: Root inside container maps to unprivileged UID on host (e.g., 165536). Container escape to host root is eliminated by design.
2. **procfs/sysfs virtualization**: `sysbox-fs` FUSE daemon intercepts `/proc` and `/sys` reads/writes, emulating per-container kernel interfaces. Prevents host kernel parameter manipulation.
3. **Syscall interception**: Selective trapping of control-path syscalls (mount, umount2, setxattr). Minimal overhead — unlike gVisor, only ~20 syscalls are trapped, not all of them.
4. **Mount immutability**: Mounts created at container init cannot be remounted read-write from inside, even by root.
5. **Seccomp profile**: Fixed allowlist of ~300 syscalls. Blocks bpf, perf_event_open, ptrace, etc.
6. **ID-mapped mounts** (kernel 5.12+): Host files mounted into container show correct ownership without shiftfs.

### What This Enables

- **Real package managers**: `apt install`, `pip install`, `npm install` work natively
- **Docker-in-Docker**: Agents can build and run Docker containers inside their sandbox (secure, unprivileged)
- **systemd**: Full init system works — agents can manage services
- **Dev servers**: Start and expose HTTP servers for testing
- **Build tools**: cargo, make, gcc, javac — all work without privilege

### What It Does NOT Provide

- No separate kernel (shared host kernel — kernel 0-day can still escape)
- No GPU/device passthrough (no `--device` support)
- No nested Sysbox (Sysbox-in-Sysbox)
- No custom seccomp profiles (fixed allowlist)
- Requires root on host for daemon installation

## Proposed Integration Architecture

### Capability: `sysbox`

New integration crate at `integrations/sysbox/`, following the Daytona reference implementation pattern.

```
integrations/sysbox/
├── Cargo.toml
├── SPEC.md
├── src/
│   ├── lib.rs          # SysboxCapability + IntegrationPlugin registration
│   ├── runtime.rs      # Container lifecycle (create, start, stop, remove)
│   ├── tools.rs        # Tool implementations
│   └── config.rs       # SysboxContainerConfig
└── tests/
    ├── tool_integration.rs
    └── live_api_test.rs
```

### Execution Model

```
┌─────────────────────────────────────────────────────────┐
│ Everruns Worker Host                                    │
│                                                         │
│ ┌─────────────────────────────────────────────────────┐ │
│ │ Worker Process                                      │ │
│ │                                                     │ │
│ │  Agent Loop → Tool Call → SysboxExecTool            │ │
│ │                   │                                 │ │
│ │                   ▼                                 │ │
│ │  docker run --runtime=sysbox-runc                   │ │
│ │  docker exec <container> <command>                  │ │
│ └──────────────────┬──────────────────────────────────┘ │
│                    │                                    │
│ ┌──────────────────▼──────────────────────────────────┐ │
│ │ Sysbox Runtime Layer                                │ │
│ │  sysbox-runc  ·  sysbox-fs  ·  sysbox-mgr          │ │
│ └──────────────────┬──────────────────────────────────┘ │
│                    │                                    │
│ ┌──────────────────▼──────────────────────────────────┐ │
│ │ Per-Session Sysbox Container                        │ │
│ │ ┌────────────────────────────────────────────────┐  │ │
│ │ │ UID 0 → host UID 165536  (user-namespace)     │  │ │
│ │ │ /proc, /sys virtualized  (sysbox-fs)          │  │ │
│ │ │ Mounts immutable         (sysbox-runc)        │  │ │
│ │ │                                                │  │ │
│ │ │ Agent workspace at /workspace                  │  │ │
│ │ │ Real bash, apt, pip, git, docker, systemd      │  │ │
│ │ │ Network: bridge (isolated) or none             │  │ │
│ │ │ Resources: cgroup-limited (CPU, memory, I/O)   │  │ │
│ │ └────────────────────────────────────────────────┘  │ │
│ └─────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

**Key difference from current Docker integration**: Sysbox containers use bridge networking (not host), mandatory user-namespace isolation, and resource limits. This makes them production-safe, not dev-only.

### Container Lifecycle

| Phase | Trigger | Action |
|-------|---------|--------|
| Create | First tool call in session | `docker run --runtime=sysbox-runc -d --name everruns-sysbox-{session_id} ...` |
| Execute | `sysbox_exec` tool | `docker exec` with timeout enforcement |
| File I/O | `sysbox_read_file` / `sysbox_write_file` | `docker exec cat` / `docker cp` |
| Heartbeat | Leased resource renewal | Refresh `lease_expires_at` while session active |
| Stop | Session end or inactivity | `docker stop` + `docker rm` via leased resource cleanup |

### Tool Surface

| Tool | Description | Hints |
|------|-------------|-------|
| `sysbox_create` | Create sandbox from image, optional resource limits | `destructive: false` |
| `sysbox_exec` | Execute command in sandbox, returns stdout/stderr/exit_code | `long_running: true`, `persist_output: true` |
| `sysbox_read_file` | Read file from sandbox filesystem | `readonly: true` |
| `sysbox_write_file` | Write content to file in sandbox filesystem | `destructive: false` |
| `sysbox_upload` | Copy file from session VFS into sandbox | `destructive: false` |
| `sysbox_download` | Copy file from sandbox into session VFS | `readonly: true` |
| `sysbox_list` | List active sandboxes in session | `readonly: true` |
| `sysbox_manage` | Stop/start/remove sandbox | `destructive: true` |

### Configuration

```json
{
  "image": "ubuntu:24.04",
  "memory_limit": "2g",
  "cpu_limit": "2",
  "working_dir": "/workspace",
  "network_mode": "bridge",
  "auto_stop_minutes": 10,
  "enable_docker_in_docker": false,
  "allowed_ports": [3000, 8080]
}
```

Capability config attached to agent definition, same pattern as Daytona/Docker.

### Network Isolation

Unlike the current Docker integration (host networking), Sysbox containers should use **bridge networking** by default:

- Agent sandbox gets isolated network namespace
- Outbound access controlled via iptables/nftables rules on the bridge
- Port forwarding only for explicitly allowed ports (`allowed_ports` config)
- Integration with Everruns' `network_access` layer: bridge firewall rules derived from the agent/session network policy
- `network_mode: "none"` option for fully airgapped execution

### Resource Limits

Enforced via Docker cgroup flags, which Sysbox respects:

```
--memory 2g                    # Hard memory limit
--cpus 2                       # CPU quota
--pids-limit 256               # Process count limit (fork bomb prevention)
--storage-opt size=10G         # Filesystem size (requires overlay2 + xfs)
--ulimit nofile=1024:4096      # File descriptor limits
```

These map to capability config fields and provide defense-in-depth against resource exhaustion (threat TM-DOS category).

## Integration with Existing Systems

### Capability Registration

```rust
// integrations/sysbox/src/lib.rs
inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,  // Production-ready from day one
        factory: || Box::new(SysboxCapability),
    }
}
```

### Leased Resources

Follow the Daytona pattern — register a lease on container creation, refresh on each tool call, cleanup via the durable scheduler:

```rust
// On sysbox_create:
ctx.leased_resource_store().upsert_resource(LeasedResource {
    provider: "sysbox",
    resource_type: "container",
    external_id: container_name,
    lease_expires_at: now + Duration::from_secs(auto_stop_minutes * 60),
    metadata: json!({ "image": config.image, "session_id": session_id }),
    ..
}).await?;
```

Cleanup handler: `docker stop <name> && docker rm <name>`.

### Harness: `coding-sysbox`

New built-in harness, parallel to `coding-daytona`:

| Property | Value |
|----------|-------|
| Name | `coding-sysbox` |
| Display Name | Coding (Self-Hosted) |
| Parent | `generic` |
| Additional capability | `sysbox` |

Same two-level architecture as `coding-daytona`: workspace VFS for lightweight ops, Sysbox sandbox for real builds/tests/services.

### Threat Model Additions

| ID | Threat | Severity | Mitigation | Status |
|----|--------|----------|------------|--------|
| TM-SYSBOX-001 | Container escape via kernel vulnerability | High | Sysbox user-namespace remapping ensures root = unprivileged on host; kernel patching cadence | ACCEPTED |
| TM-SYSBOX-002 | Resource exhaustion (CPU/memory/disk) | High | cgroup limits enforced via Docker flags; `--pids-limit` for fork bombs | MITIGATED |
| TM-SYSBOX-003 | Network-based attacks from sandbox | High | Bridge networking with iptables rules; `network_access` policy integration; `none` mode available | MITIGATED |
| TM-SYSBOX-004 | Cross-session container access | High | Container name includes session_id; Docker API access restricted to worker process | MITIGATED |
| TM-SYSBOX-005 | Sysbox daemon compromise | Critical | Runs as root; host kernel attack surface; monitor daemon health; restrict host access | ACCEPTED |
| TM-SYSBOX-006 | Image supply chain (malicious base images) | Medium | Curated image allowlist in config; image pull policy; registry restrictions | MITIGATED |
| TM-SYSBOX-007 | Docker socket exposure inside sandbox | High | Docker-in-Docker uses Sysbox inner Docker (no socket mount); `enable_docker_in_docker` opt-in | MITIGATED |
| TM-SYSBOX-008 | Stale container not cleaned up | Medium | Leased resource scheduler; `auto_stop_minutes` config; Docker `--rm` flag | MITIGATED |

### Output Sanitization

Same pipeline as existing exec tools: strip ANSI → collapse CR lines → middle-truncate at 16 KiB. Full output persisted to `/.outputs/` via `tool_output_persistence` capability.

## Comparison Matrix

| Dimension | Virtual Bash | Sysbox (proposed) | Docker (current) | Daytona | E2B |
|-----------|-------------|-------------------|-------------------|---------|-----|
| Isolation level | WASM-like | VM-like (user-ns) | Weak (host network) | Full VM | Full VM |
| Real processes | No | Yes | Yes | Yes | Yes |
| Network access | None | Configurable | Host (unsafe) | Full | Full |
| Package install | No | Yes | Yes | Yes | Yes |
| Docker-in-Docker | No | Yes (secure) | No | Yes | No |
| Self-hosted | N/A (in-process) | Yes | Yes | No (SaaS) | No (SaaS) |
| Startup time | Instant | ~1s | ~1s | ~5-10s | ~3-5s |
| Per-sandbox cost | Zero | Host resources only | Host resources only | API pricing | API pricing |
| Production-ready | Yes | Yes (with limits) | No (dev-only) | Yes | Yes |
| Density (per host) | N/A | ~50-100 agents/host | ~50-100 agents/host | N/A | N/A |
| Resource isolation | Execution limits | cgroups (CPU/mem/PID) | None enforced | Provider-managed | Provider-managed |
| Kernel isolation | N/A | Shared (user-ns) | Shared (no user-ns) | Separate | Separate |
| Risk level | Low | High (admin-gated) | High (admin-gated) | High (admin-gated) | High (admin-gated) |

## Infrastructure Requirements

### Host Requirements

| Requirement | Minimum | Recommended |
|-------------|---------|-------------|
| Linux kernel | 5.12 | 6.1+ (LTS) |
| Distribution | Ubuntu 22.04 / Debian 11 | Ubuntu 24.04 |
| CPU | 4 cores | 8+ cores |
| RAM | 8 GB | 32+ GB |
| Disk | 50 GB | 200+ GB (SSD) |
| Docker Engine | 20.10+ | 27+ |
| Sysbox | 0.7.0+ | Latest release |

### Installation (on worker hosts)

```bash
# 1. Install Sysbox
wget https://downloads.nestybox.com/sysbox/releases/v0.7.0/sysbox-ce_0.7.0-0.linux_amd64.deb
sudo apt-get install ./sysbox-ce_0.7.0-0.linux_amd64.deb

# 2. Verify
systemctl status sysbox
docker info | grep -i runtime  # should list sysbox-runc

# 3. Test
docker run --runtime=sysbox-runc -it --rm ubuntu:24.04 bash -c "whoami && cat /proc/self/uid_map"
# Should show: root   0  165536  65536  (UID remapped)
```

### Kubernetes Deployment

For K8s-managed workers, Sysbox runs as a DaemonSet:

```yaml
apiVersion: node.k8s.io/v1
kind: RuntimeClass
metadata:
  name: sysbox-runc
handler: sysbox-runc

---
apiVersion: v1
kind: Pod
metadata:
  name: everruns-worker
spec:
  runtimeClassName: sysbox-runc
  hostUsers: false
  containers:
  - name: worker
    image: everruns-worker:latest
    # Worker creates inner containers via Docker CLI
```

## SaaS Infrastructure: How Sysbox Fits

### Current Production Topology (from `everruns/saas`)

```
          Cloudflare DNS (dns-only)
                  │
          dev.everruns.com
                  │
       ┌──────────┴──────────┐
       │ Hetzner VPS (cpx21) │  Ubuntu 24.04, 3 vCPU, 4 GB RAM, 80 GB disk
       │                     │
       │  Caddy (:80/:443)   │  auto-TLS, reverse proxy
       │    ├─ /api/* → server:9000
       │    └─ /*     → ui:3000
       │                     │
       │  saas-server        │  control plane (HTTP + gRPC)
       │  saas-worker ×3     │  stateless executors (gRPC to server)
       │  saas-ui            │  Next.js frontend
       │  NATS               │  JetStream (ephemeral events)
       └──────────┬──────────┘
                  │
          Neon Postgres
          (managed, aws-us-east-1)
```

**IaC**: Terraform Cloud → Hetzner + Cloudflare + Neon providers.
**Deploy**: GitHub Actions → build Docker images → push GHCR → SSH to VPS → `docker compose pull && up`.
**Secrets**: Doppler (CI project `everruns-dev`, runtime project `everruns`). Only `DOPPLER_TOKEN` on disk.
**Systemd**: `everruns.service` runs `doppler run -- docker compose up` for reboot persistence.

### Problem: Sysbox on Current Setup

The current `cpx21` VPS (3 vCPU, 4 GB RAM) is a shared-CPU Hetzner instance running **all services on one host**. Sysbox adds several constraints:

1. **Sysbox daemons need root on the host** — `sysbox-runc`, `sysbox-fs`, `sysbox-mgr` run as systemd services alongside Docker.
2. **Kernel 5.12+ required** — Ubuntu 24.04 ships kernel 6.8+, so this is already satisfied.
3. **Workers need Docker CLI access** — workers must `docker run --runtime=sysbox-runc`, meaning they either run on the host (not in Docker) or need Docker-in-Docker (which needs Sysbox itself, circular).
4. **Resource headroom** — each Sysbox sandbox uses 150-500 MB RAM + CPU share. On a 4 GB host already running server + 3 workers + NATS + Caddy, there's limited room.

### Deployment Options

#### Option A: Workers on Host (Recommended for Dev)

Workers run as native processes on the VPS (not containerized), with direct access to Docker + Sysbox:

```
┌─────────────────────────────────────────────┐
│ Hetzner VPS (Ubuntu 24.04)                  │
│                                             │
│  systemd services:                          │
│    sysbox.service      (sysbox daemons)     │
│    everruns.service     (docker compose)     │
│                                             │
│  Docker Compose (server + ui + nats):       │
│    ┌────────┐ ┌────┐ ┌──────┐ ┌──────┐     │
│    │ Caddy  │ │NATS│ │Server│ │  UI  │     │
│    └────────┘ └────┘ └──────┘ └──────┘     │
│                                             │
│  Native worker processes:                   │
│    everruns-worker ×N  (direct Docker CLI)  │
│         │                                   │
│         ▼                                   │
│    docker run --runtime=sysbox-runc ...     │
│    ┌──────────┐ ┌──────────┐               │
│    │ Sandbox1 │ │ Sandbox2 │  ...          │
│    │(session) │ │(session) │               │
│    └──────────┘ └──────────┘               │
└─────────────────────────────────────────────┘
```

**Pros**: Simple, no Docker-in-Docker. Workers call `docker run` directly.
**Cons**: Workers not containerized (less uniform). Needs worker binary installed on host.

**Terraform changes**: Add Sysbox install to `cloud-init.yaml`, download worker binary, create worker systemd units.

#### Option B: Privileged Worker Container with Docker Socket (Not Recommended)

Mount `/var/run/docker.sock` into worker containers. Workers create Sysbox containers on the host via the socket.

**Pros**: Workers stay in Docker Compose.
**Cons**: Docker socket mount = root-equivalent on host. Defeats the purpose of Sysbox isolation. If an agent escapes the worker container via the socket, they own the host.

#### Option C: Dedicated Sandbox Host (Recommended for Production)

Separate the sandbox execution onto a different machine. Workers still run in Docker Compose, but sandbox operations go over SSH/API to a Sysbox-enabled host:

```
┌──────────────────────┐      ┌──────────────────────────┐
│ Control Plane VPS    │      │ Sandbox Host(s)          │
│                      │      │                          │
│ Caddy + Server + UI  │ gRPC │ Worker(s) (native)       │
│ NATS                 │◄────►│ Docker + Sysbox          │
│                      │      │ Agent sandboxes           │
└──────────┬───────────┘      └──────────────────────────┘
           │
     Neon Postgres
```

**Pros**: Control plane isolated from sandbox blast radius. Scale sandbox hosts independently. Sandbox host can be a bigger machine (dedicated CPU, more RAM).
**Cons**: Two machines, more Terraform, cross-host gRPC networking.

**Terraform changes**: New `hcloud_server` resource for sandbox host(s). Workers deployed on sandbox host. Firewall: allow gRPC (9001) from sandbox host to control plane.

#### Option D: Sysbox Worker Container (Future / K8s)

Run the worker itself in a Sysbox container. The worker can then create inner containers (Docker-in-Docker) for agent sandboxes:

```
Sysbox container (worker):
  └─ Docker daemon (inner)
       └─ Agent sandbox container (inner, runc)
```

**Pros**: Cleanest architecture. Worker + sandboxes fully isolated.
**Cons**: Requires K8s with Sysbox RuntimeClass, or a host running Sysbox where the worker image starts its own Docker daemon. More complex orchestration. Sysbox nesting is not supported, so inner containers use standard runc (still isolated by outer Sysbox user-namespace).

### Recommended Path

| Phase | Option | Environment | VPS Size |
|-------|--------|-------------|----------|
| Dev/MVP | **A** (workers on host) | `cpx31` (4 vCPU, 8 GB) | Single VPS |
| Staging | **C** (dedicated sandbox host) | `cpx31` control + `cpx41` sandbox | Two VPS |
| Production | **C** or **D** | Dedicated pool or K8s | Scale-out |

### Concrete Terraform Changes for Option A (Dev MVP)

```hcl
# Upgrade VPS type for sandbox headroom
variable "server_type" {
  default = "cpx31"  # was cpx21 (3 vCPU, 4 GB → 4 vCPU, 8 GB)
}
```

**cloud-init.yaml additions:**

```yaml
runcmd:
  # ... existing Docker install ...

  # Install Sysbox
  - wget -q https://downloads.nestybox.com/sysbox/releases/v0.7.0/sysbox-ce_0.7.0-0.linux_amd64.deb
  - apt-get install -y ./sysbox-ce_0.7.0-0.linux_amd64.deb
  - systemctl enable --now sysbox
  - rm sysbox-ce_0.7.0-0.linux_amd64.deb

  # Verify Sysbox runtime is registered with Docker
  - docker info --format '{{.Runtimes}}' | grep -q sysbox-runc

  # Download worker binary (same version as server image)
  - |
    docker create --name tmp-worker ghcr.io/everruns/saas-server:development
    docker cp tmp-worker:/usr/local/bin/everruns-worker /usr/local/bin/
    docker rm tmp-worker
```

**New systemd unit** `everruns-worker@.service` (template for N workers):

```ini
[Unit]
Description=Everruns Worker %i
After=everruns.service docker.service sysbox.service
Requires=docker.service sysbox.service

[Service]
Type=simple
EnvironmentFile=/opt/everruns/doppler.env
ExecStart=/usr/local/bin/doppler run -- /usr/local/bin/everruns-worker
Environment=WORKER_GRPC_ADDRESS=127.0.0.1:9001
Environment=RUST_LOG=info
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Enable with `systemctl enable everruns-worker@{1..3}`.

**docker-compose.dev.yml changes**: Remove the `worker` service (workers now run natively). Server gRPC port exposed on localhost:

```yaml
  server:
    ports:
      - "127.0.0.1:9000:9000"
      - "127.0.0.1:9001:9001"  # gRPC for native workers
```

### Resource Budget (Option A, cpx31: 4 vCPU, 8 GB RAM)

| Component | RAM | CPU |
|-----------|-----|-----|
| Server | ~300 MB | 0.5 |
| Workers ×3 | ~150 MB each = 450 MB | 0.3 each |
| NATS | ~50 MB | 0.1 |
| Caddy | ~30 MB | 0.05 |
| UI | ~200 MB | 0.2 |
| Sysbox daemons | ~100 MB | 0.1 |
| **Subtotal** | **~1.1 GB** | **~1.9** |
| **Available for sandboxes** | **~6.5 GB** | **~2 vCPU** |
| Sandboxes (2 GB each) | 3 concurrent | 1 CPU each |

With a `cpx31`, you can comfortably run ~3 concurrent agent sandboxes. For more, upgrade to `cpx41` (8 vCPU, 16 GB) or split to Option C.

## Implementation Plan

### Phase 1: Core Integration (MVP)

1. Create `integrations/sysbox/` crate with `SysboxCapability`
2. Implement container lifecycle: create, exec, read_file, write_file, stop
3. Bridge networking with default-deny outbound (allow DNS + HTTPS)
4. cgroup resource limits from capability config
5. Leased resource integration for cleanup
6. Unit tests + integration tests with mock Docker commands
7. SPEC.md co-located with crate

### Phase 2: Harness & Polish

8. `coding-sysbox` built-in harness (parent: `generic`)
9. System prompt tuned for two-level execution (VFS + Sysbox)
10. Connection provider for runtime detection (is Sysbox installed on this host?)
11. Threat model section in `specs/threat-model.md`
12. User docs at `docs/integrations/sysbox.md`

### Phase 3: Advanced Features

13. Docker-in-Docker support (opt-in via config)
14. Snapshot/checkpoint support (save container state, restore later)
15. Image allowlist enforcement
16. Network policy integration with `network_access` layer
17. Port forwarding for dev server access
18. Multi-container sessions (e.g., app + database)

### Phase 4: Kubernetes Native

19. RuntimeClass-based pod scheduling for K8s deployments
20. Pod-level Sysbox containers (worker itself runs in Sysbox)
21. Storage class integration for persistent workspace volumes

## Risks & Open Questions

### Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| Kernel vulnerability = escape (shared kernel) | High | Keep host kernel patched; user-namespace limits blast radius; defense-in-depth with seccomp + cgroups |
| Sysbox project maintenance (post-Docker acquisition, community) | Medium | Sysbox is open-source; fork-ready; core mechanism is user-namespaces which is kernel-native |
| Ubuntu/Debian only for package install | Medium | Build from source on other distros; or standardize worker hosts on Ubuntu |
| Sysbox daemons require root on host | Low | Standard for container runtimes; same as Docker daemon itself |
| Worker must have Docker CLI + Sysbox installed | Medium | Infrastructure requirement; document in operator guide; health check at worker startup |

### Open Questions

1. **Single vs. multiple containers per session?** Daytona supports multiple sandboxes. For Sysbox, start with single container per session (simpler lifecycle), add multi-container in Phase 3.

2. **Image registry policy**: Should we maintain a curated set of base images, or let operators configure any image? Recommend: curated defaults + operator override.

3. **Workspace sync strategy**: How to move files between session VFS and Sysbox container? Options:
   - `docker cp` (simplest, current Docker integration approach)
   - Volume mount from host (requires host filesystem, breaks VFS isolation)
   - Agent-driven upload/download tools (proposed — `sysbox_upload` / `sysbox_download`)
   - Recommend: agent-driven tools for explicit control, `docker cp` as implementation.

4. **Worker topology**: Should workers that support Sysbox be a separate pool, or should all workers have Sysbox? Recommend: capability-based worker routing — task metadata includes required capabilities, scheduler routes to capable workers.

5. **Sysbox vs. gVisor vs. Kata**: Why Sysbox specifically?
   - gVisor: Intercepts ALL syscalls (not just ~20) — 10-30% overhead on I/O-heavy workloads. Better isolation but worse performance.
   - Kata: Full VM per container — strongest isolation but VM startup time and density overhead.
   - Sysbox: Best balance of isolation, performance, and density for agent workloads. ~95-98% native performance.

6. **Firecracker/Sprites overlap**: The Sprites integration (Fly.io Firecracker microVMs) is already listed in integrations. Sysbox is complementary — Sprites for cloud-managed microVMs, Sysbox for self-hosted container isolation. Different deployment models, same agent tool interface.

## Decision

Proceed with Sysbox integration as a new `integrations/sysbox/` crate. It fills the self-hosted strong-isolation gap between virtual bash (safe but limited) and cloud sandboxes (powerful but external). The integration follows established patterns (Daytona reference, inventory plugin, leased resources, capability trait) and adds a production-ready self-hosted execution option.

Key architectural choices:
- **Bridge networking** (not host) — isolated by default
- **cgroup limits** — mandatory resource boundaries
- **Leased resources** — durable cleanup
- **Admin-gated** (`RiskLevel::High`) — same as Daytona/E2B
- **Phase 1 = single container per session** — match Docker integration simplicity, expand later

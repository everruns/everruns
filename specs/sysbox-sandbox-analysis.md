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

Worker tools communicate with the Docker Engine REST API (via socket proxy or TCP) using `reqwest` — same HTTP client pattern as Daytona. No `docker` CLI binary needed inside the worker container.

```
Worker container                    Host
┌──────────────────┐     ┌──────────────────────────────┐
│ Agent Loop       │     │ docker-socket-proxy :2375    │
│  → SysboxExec    │     │  or Docker TCP API :2376     │
│    → reqwest ────────►│                              │
│      POST /exec  │     │  → Docker Engine daemon      │
└──────────────────┘     │    → sysbox-runc runtime     │
                         │                              │
                         │ ┌──────────────────────────┐ │
                         │ │ Sysbox Container         │ │
                         │ │  UID 0 → host UID 165536 │ │
                         │ │  /proc,/sys virtualized  │ │
                         │ │  cgroup-limited           │ │
                         │ │  bridge network           │ │
                         │ └──────────────────────────┘ │
                         └──────────────────────────────┘
```

**Key differences from current Docker integration**:
- Uses Docker Engine REST API (`reqwest`) instead of Docker CLI (`Command::new("docker")`)
- Sysbox runtime provides mandatory user-namespace isolation
- Bridge networking (not host)
- cgroup resource limits enforced
- Production-safe, not dev-only

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

### How Tools Reach Sandboxes

Two existing patterns in the codebase:

| Integration | How tools talk to sandboxes | Worker needs |
|-------------|---------------------------|--------------|
| **Daytona** | HTTP REST (`reqwest`) → Daytona cloud API | Nothing special — just network access |
| **Docker** (experimental) | `tokio::process::Command::new("docker")` → local Docker CLI | `docker` binary + Docker daemon access |

For Sysbox, the tool needs to call `docker run --runtime=sysbox-runc` and `docker exec`. Same as Docker integration — **process-level access to the Docker CLI and daemon**.

The worker doesn't "become" the sandbox. It creates and manages sandbox containers as external resources, same as Daytona manages remote sandboxes via HTTP.

### Problem: Containerized Workers Can't Reach Host Docker

Workers currently run inside Docker Compose containers. A containerized worker can't call `docker run` on the host without either:

1. **Docker socket mount** (`/var/run/docker.sock`) — grants root-equivalent host access. Defeats Sysbox's isolation purpose.
2. **Docker-in-Docker** — worker runs its own Docker daemon inside its container. But that inner daemon doesn't have Sysbox (Sysbox daemons run on the host, not inside containers).

**This is the core infra constraint**: Sysbox daemons (`sysbox-runc`, `sysbox-fs`, `sysbox-mgr`) run as systemd services on the **host**. Only the host's Docker daemon can use `--runtime=sysbox-runc`. Workers must somehow reach that daemon.

Additional constraints:
- **Kernel**: Ubuntu 24.04 ships kernel 6.8+ — already satisfied.
- **Resource headroom**: Each sandbox uses 150-500 MB RAM. Current `cpx21` (4 GB) is tight.

### Deployment Options

#### Option A: Docker Socket Proxy (Recommended for Dev)

Use an off-the-shelf **docker-socket-proxy** to expose the host's Docker Engine REST API to worker containers over HTTP — no custom code needed. Workers call it with `reqwest` (same pattern as Daytona). The proxy restricts which Docker API endpoints are accessible.

Existing projects:
- [**Tecnativa/docker-socket-proxy**](https://github.com/Tecnativa/docker-socket-proxy) — HAProxy-based, widely used, env-var access control
- [**Wollomatic/socket-proxy**](https://github.com/wollomatic/socket-proxy) — Go, zero dependencies, fine-grained path-based rules

```
Docker Compose (all services stay containerized):
  ┌────────┐ ┌────┐ ┌──────┐ ┌──────┐
  │ Caddy  │ │NATS│ │Server│ │  UI  │
  └────────┘ └────┘ └──────┘ └──────┘
  ┌────────────┐      ┌──────────────────┐
  │ Workers ×3 │─HTTP→│ docker-socket-   │
  └────────────┘      │ proxy :2375      │
                      │ (restricts API)  │
                      └────────┬─────────┘
                               │ /var/run/docker.sock
                     Host Docker daemon
                         + Sysbox
                               │
                  ┌────────────┴────────────┐
                  │ Sysbox containers       │
                  │ (agent sandboxes)       │
                  └─────────────────────────┘
```

**docker-compose.dev.yml** addition:

```yaml
  docker-proxy:
    image: tecnativa/docker-socket-proxy
    restart: unless-stopped
    environment:
      CONTAINERS: 1   # allow create/start/stop/remove/exec
      POST: 1         # allow POST (create, exec)
      IMAGES: 1       # allow pull
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
    expose:
      - "2375"
```

Workers reach the proxy at `http://docker-proxy:2375`. The integration crate uses Docker Engine REST API directly via `reqwest`:

```rust
// No docker CLI needed — pure HTTP to Docker Engine API
let resp = client.post("http://docker-proxy:2375/containers/create")
    .json(&json!({
        "Image": "ubuntu:24.04",
        "HostConfig": {
            "Runtime": "sysbox-runc",
            "Memory": 2_147_483_648_u64,
            "NanoCpus": 2_000_000_000_u64,
            "PidsLimit": 256
        }
    }))
    .send().await?;
```

**Pros**: Zero custom code. Workers stay in Docker Compose. Proxy restricts API surface (e.g., disable image delete, network create). Well-tested OSS projects.
**Cons**: Socket mount on the proxy container (single trust point — proxy is the only container with socket access, and it's read-only with restricted endpoints). Not as locked-down as a custom manager but good enough for dev.

#### Option B: Docker Engine API over TCP (Production Alternative)

Configure Docker daemon to listen on TCP (with TLS) instead of using a socket proxy. Workers call `https://host:2376/` directly. No socket mount anywhere.

```json
// /etc/docker/daemon.json on host
{
  "hosts": ["unix:///var/run/docker.sock", "tcp://0.0.0.0:2376"],
  "tls": true,
  "tlscacert": "/etc/docker/ca.pem",
  "tlscert": "/etc/docker/server-cert.pem",
  "tlskey": "/etc/docker/server-key.pem",
  "tlsverify": true,
  "runtimes": { "sysbox-runc": { "path": "/usr/bin/sysbox-runc" } }
}
```

**Pros**: No socket mount. TLS client cert auth. Docker-native, no proxy.
**Cons**: Exposes full Docker API (no endpoint filtering). Requires TLS cert management.

#### Option C: Dedicated Sandbox Host (Recommended for Production)

Separate machine for sandboxes. Workers on control plane VPS call Docker Engine API on sandbox host over TLS (Option B pattern, cross-host).

**Pros**: Blast radius isolation. Scale sandbox hosts independently.
**Cons**: Two machines, TLS cert management, more Terraform.

#### Option D: Sysbox Worker Container (Future / K8s)

Worker itself runs in a Sysbox container with inner Docker daemon. K8s-native via RuntimeClass.

**Pros**: Cleanest K8s-native architecture.
**Cons**: Complex. Sysbox nesting unsupported — inner containers use runc.

### Recommended Path

| Phase | Option | Why | VPS |
|-------|--------|-----|-----|
| Dev/MVP | **A** (socket proxy) | Zero custom code, workers unchanged | `cpx31` (4 vCPU, 8 GB) |
| Staging | **B+C** (TLS Docker API, dedicated host) | Blast radius isolation | `cpx31` + `cpx41` |
| Production | **C** or **D** | Dedicated pool or K8s | Scale-out |

### Concrete Changes for Option A (Dev MVP)

**Terraform** — upgrade VPS for sandbox headroom:

```hcl
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

  # Verify Sysbox runtime registered with Docker
  - docker info --format '{{.Runtimes}}' | grep -q sysbox-runc
```

**docker-compose.dev.yml additions:**

```yaml
  docker-proxy:
    image: tecnativa/docker-socket-proxy
    restart: unless-stopped
    environment:
      CONTAINERS: 1
      POST: 1
      IMAGES: 1
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
    expose:
      - "2375"

  worker:
    environment:
      - DOCKER_HOST=tcp://docker-proxy:2375
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
| Docker socket proxy | ~10 MB | 0.02 |
| **Subtotal** | **~1.1 GB** | **~1.9** |
| **Available for sandboxes** | **~6.5 GB** | **~2 vCPU** |
| Sandboxes (2 GB each) | 3 concurrent | 1 CPU each |

With a `cpx31`, ~3 concurrent agent sandboxes. Upgrade to `cpx41` (8 vCPU, 16 GB) or split to Option C for more.

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

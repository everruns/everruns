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
| TM-SYSBOX-004 | Cross-session container access | High | Container name includes session_id; Docker API queries label-filtered; per-sandbox isolated network | MITIGATED |
| TM-SYSBOX-009 | Cross-tenant sandbox access via Docker API | Critical | Tool code scopes by session_id from ToolContext; Docker API label filters; per-sandbox network prevents L3 reach | MITIGATED |
| TM-SYSBOX-010 | Cross-tenant sandbox network reachability | High | Each sandbox on its own Docker bridge network; no shared bridge; private IP ranges blocked | MITIGATED |
| TM-SYSBOX-011 | Tenant resource starvation | High | Per-sandbox cgroup limits + per-org max sandbox count enforced at create time | MITIGATED |
| TM-SYSBOX-012 | Stale sandbox data leakage across tenants | High | Container + network removed on cleanup; no volume reuse; leased resource scheduler ensures cleanup | MITIGATED |
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

## SaaS Multi-Tenant Architecture

### Constraint: Multi-Tenant from Day One

Even dev runs multiple tenants (PropelAuth, org-based isolation). This is the primary architectural constraint — not "how do workers reach Docker" but **"how do we prevent Tenant A's sandbox from seeing Tenant B's sandbox"**.

### Current SaaS Topology (from `everruns/saas`)

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

### Multi-Tenancy Trust Model for Sandboxes

**Existing isolation** (TM-TENANT-001 through TM-TENANT-008):
- All DB queries include `WHERE org_id = $org_id`
- Workers are cross-org by design — org-scoping enforced at API/service layer
- Sessions scoped via agent FK → agent scoped to org
- 404 (not 403) for cross-org access — no existence leakage

**New isolation needed for Sysbox containers**:

| Threat | Description | Severity |
|--------|-------------|----------|
| TM-SYSBOX-009 | Tenant A's sandbox lists/execs into Tenant B's sandbox | Critical |
| TM-SYSBOX-010 | Tenant A's sandbox reaches Tenant B's sandbox over network | High |
| TM-SYSBOX-011 | Tenant A exhausts host resources, starving Tenant B | High |
| TM-SYSBOX-012 | Stale sandbox from Tenant A leaks data to next Tenant B session | High |

### How Existing Integrations Handle This

**Daytona**: Each sandbox created with a per-user API key. Daytona SaaS enforces tenant isolation externally. Everruns tools scope operations by session_id stored in session secrets. Worker can't cross-session — `ToolContext` provides the session_id, and the Daytona client is constructed per-call from session-scoped state.

**Docker (experimental)**: Container named `everruns-{session_id}`. Worker code only operates on that container. No API-level enforcement — trust is in the tool implementation. Dev-only, so acceptable.

**For Sysbox in SaaS, we need defense-in-depth**: tool-level scoping (like Daytona) PLUS infrastructure-level isolation (network, resources).

### Proposed Architecture

```
┌─────────────────────────────────────────────────────┐
│ Hetzner VPS (cpx41: 8 vCPU, 16 GB)                 │
│ Ubuntu 24.04 + Docker + Sysbox                      │
│                                                     │
│ Docker Compose:                                     │
│  ┌───────┐ ┌────┐ ┌──────┐ ┌──────┐ ┌────┐         │
│  │ Caddy │ │NATS│ │Server│ │  UI  │ │ VK │         │
│  └───────┘ └────┘ └──────┘ └──────┘ └────┘         │
│  ┌──────────┐                                       │
│  │Workers ×3│                                       │
│  └────┬─────┘                                       │
│       │ reqwest (Docker Engine REST API)             │
│  ┌────▼─────────────────────┐                       │
│  │ docker-socket-proxy:2375 │ (endpoint filtering)  │
│  └────┬─────────────────────┘                       │
│       │ /var/run/docker.sock                        │
│  ┌────▼──────────────────────────────────────┐      │
│  │ Docker Engine + sysbox-runc               │      │
│  │                                           │      │
│  │ ┌───────────────────────────────────────┐ │      │
│  │ │ net: sandbox-{org_a}-{session_1}      │ │      │
│  │ │ ┌─────────────────────────────┐       │ │      │
│  │ │ │ evr-{session_1}-sysbox     │       │ │      │
│  │ │ │ --runtime=sysbox-runc      │       │ │      │
│  │ │ │ --memory 2g --cpus 1       │       │ │      │
│  │ │ │ --pids-limit 256           │       │ │      │
│  │ │ │ label: org={org_a}         │       │ │      │
│  │ │ │ label: session={session_1} │       │ │      │
│  │ │ └─────────────────────────────┘       │ │      │
│  │ └───────────────────────────────────────┘ │      │
│  │                                           │      │
│  │ ┌───────────────────────────────────────┐ │      │
│  │ │ net: sandbox-{org_b}-{session_2}      │ │      │
│  │ │ ┌─────────────────────────────────┐   │ │      │
│  │ │ │ evr-{session_2}-sysbox         │   │ │      │
│  │ │ │ (different org, isolated net)   │   │ │      │
│  │ │ └─────────────────────────────────┘   │ │      │
│  │ └───────────────────────────────────────┘ │      │
│  └───────────────────────────────────────────┘      │
└──────────────────────┬──────────────────────────────┘
                       │
                 Neon Postgres
```

### Isolation Layers (Defense-in-Depth)

#### Layer 1: Tool-Level Scoping (Application)

Same as Daytona. `ToolContext` provides `session_id` and `org_id`. Tool implementation only operates on containers matching `evr-{session_id}-sysbox`. Container name is derived from session_id, never from user input.

```rust
fn container_name(session_id: &SessionId) -> String {
    format!("evr-{}-sysbox", session_id.uuid())
}
```

Worker code never lists all containers or operates on foreign session_ids. The `sysbox_list` tool filters by session — `GET /containers/json?filters={"label":["session={session_id}"]}`.

#### Layer 2: Per-Sandbox Docker Network (Network Isolation)

Each sandbox gets its own Docker bridge network named `sandbox-{org_id}-{session_id}`. The sandbox container is the only member.

```rust
// On sysbox_create:
// 1. Create isolated network
POST /networks/create { "Name": "sandbox-{org}-{session}", "Driver": "bridge" }

// 2. Create container on that network
POST /containers/create {
    "HostConfig": {
        "Runtime": "sysbox-runc",
        "NetworkMode": "sandbox-{org}-{session}",
        ...
    },
    "Labels": {
        "org": "{org_id}",
        "session": "{session_id}",
        "managed-by": "everruns"
    }
}
```

**Effect**: Sandbox A cannot reach Sandbox B at L3. No shared bridge. Even if an agent discovers another container's IP, packets are dropped because they're on different networks.

**Cleanup**: Network removed with container via leased resource handler.

#### Layer 3: Container Labels + Docker API Filtering

All Sysbox-managed containers get labels: `org`, `session`, `managed-by=everruns`. Tool operations use label filters in Docker API calls:

```
GET /containers/json?filters={"label":["session={session_id}","managed-by=everruns"]}
```

This is defense-in-depth — even if tool code had a bug, the Docker API queries are label-scoped.

#### Layer 4: Resource Limits (Per-Sandbox + Per-Org)

**Per-sandbox** (cgroup enforcement via Docker):
```json
{
  "Memory": 2147483648,
  "NanoCpus": 2000000000,
  "PidsLimit": 256
}
```

**Per-org limits** (application-level, enforced in tool code):
- Max concurrent sandboxes per org (e.g., 3 for free tier, 10 for paid)
- Max total memory per org across all sandboxes
- Checked at `sysbox_create` time via leased resource count query

```rust
// Before creating sandbox:
let active = ctx.leased_resource_store()
    .list_resources_by_org(org_id, "sysbox")
    .await?;
if active.len() >= org_sandbox_limit {
    return ToolExecutionResult::tool_error(
        "Sandbox limit reached for your organization"
    );
}
```

#### Layer 5: Outbound Network Policy (Egress Filtering)

Per-sandbox iptables rules on the bridge, blocking:
- Private IP ranges (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16) — prevents internal network probing
- Cloud metadata endpoints (169.254.169.254)
- Allow: DNS (53/udp), HTTPS (443/tcp), HTTP (80/tcp)

Applied as Docker network driver options or post-creation iptables rules. Mirrors fetchkit's private-IP blocking (TM-API-008) at the container level.

#### Layer 6: Sysbox Kernel Isolation

Provided by Sysbox itself — user-namespace, procfs/sysfs virtualization, mount immutability, seccomp. This is the last line of defense if everything above fails.

### How Tools Reach Docker

Workers run inside Docker Compose containers. They reach the host Docker daemon via a **docker-socket-proxy** (Tecnativa or Wollomatic):

```yaml
  docker-proxy:
    image: tecnativa/docker-socket-proxy
    restart: unless-stopped
    environment:
      CONTAINERS: 1   # create/start/stop/remove/exec
      POST: 1         # allow POST requests
      IMAGES: 1       # allow image pull
      NETWORKS: 1     # create/remove sandbox networks
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
    expose:
      - "2375"
```

Integration crate uses `reqwest` to call Docker Engine REST API at `http://docker-proxy:2375` — same pattern as Daytona's HTTP client. No `docker` CLI needed.

### Concrete Infra Changes

**Terraform** — upgrade VPS:

```hcl
variable "server_type" {
  default = "cpx41"  # 8 vCPU, 16 GB (was cpx21: 3 vCPU, 4 GB)
}
```

**cloud-init.yaml** — install Sysbox:

```yaml
runcmd:
  # ... existing Docker install ...
  - wget -q https://downloads.nestybox.com/sysbox/releases/v0.7.0/sysbox-ce_0.7.0-0.linux_amd64.deb
  - apt-get install -y ./sysbox-ce_0.7.0-0.linux_amd64.deb
  - systemctl enable --now sysbox
  - rm sysbox-ce_0.7.0-0.linux_amd64.deb
  - docker info --format '{{.Runtimes}}' | grep -q sysbox-runc
```

**docker-compose.dev.yml** — add proxy + env:

```yaml
  docker-proxy:
    image: tecnativa/docker-socket-proxy
    restart: unless-stopped
    environment:
      CONTAINERS: 1
      POST: 1
      IMAGES: 1
      NETWORKS: 1
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
    expose:
      - "2375"

  worker:
    environment:
      - SYSBOX_DOCKER_HOST=tcp://docker-proxy:2375
```

**Doppler** — add config:

| Variable | Value | Purpose |
|----------|-------|---------|
| `SYSBOX_ENABLED` | `true` | Feature flag for capability availability |
| `SYSBOX_DEFAULT_IMAGE` | `ubuntu:24.04` | Default sandbox image |
| `SYSBOX_MAX_SANDBOXES_PER_ORG` | `3` | Free tier limit |
| `SYSBOX_MEMORY_LIMIT` | `2g` | Per-sandbox default |
| `SYSBOX_CPU_LIMIT` | `1` | Per-sandbox default |

### Resource Budget (cpx41: 8 vCPU, 16 GB RAM)

| Component | RAM | CPU |
|-----------|-----|-----|
| Server + NATS + Caddy + UI | ~600 MB | 0.9 |
| Workers ×3 | ~450 MB | 0.9 |
| Sysbox daemons + proxy | ~120 MB | 0.15 |
| **Subtotal** | **~1.2 GB** | **~2** |
| **Available for sandboxes** | **~14 GB** | **~6 vCPU** |
| Sandboxes (2 GB / 1 CPU each) | **7 concurrent** | across all tenants |

### Production Scale-Out (Option C)

For production multi-tenant with more headroom:

```
┌─────────────────────┐         ┌───────────────────────────┐
│ Control Plane VPS   │         │ Sandbox Host(s) ×N        │
│                     │         │                           │
│ Server + UI + NATS  │  gRPC   │ Workers (native or Sysbox)│
│ Caddy               │◄───────►│ Docker + Sysbox           │
│                     │         │ Docker TCP API + TLS      │
└──────────┬──────────┘         └───────────────────────────┘
           │
     Neon Postgres
```

Workers move to sandbox host(s). Control plane has zero Docker/Sysbox surface. Scale sandbox hosts per demand. Per-org routing possible (premium tenants get dedicated hosts).

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

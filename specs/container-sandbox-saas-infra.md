# Container Sandbox SaaS Infrastructure Specification

## Abstract

Deployment architecture for the `container_sandbox` OSS capability on the Everruns SaaS platform. Dedicated sandbox VPS with Sysbox runtime, connected to the control plane via Hetzner private network. Multi-tenant from day one.

This spec covers the SaaS-specific infrastructure. The OSS capability spec is at `specs/container-sandbox.md`.

## Architecture: Two-VPS Split

Even dev runs multiple tenants. A sandbox escape on a single VPS = control plane compromise (DB creds, Doppler token). The sandbox VPS is dedicated to container execution only.

```
 Hetzner Private Network (10.0.0.0/16)
 ┌──────────────────────────┐    ┌──────────────────────────────────┐
 │ Control Plane VPS        │    │ Sandbox VPS                      │
 │ (cpx21, unchanged)       │    │ (cpx31: 4 vCPU, 8 GB)           │
 │ 10.0.0.2                 │    │ 10.0.0.3                         │
 │                          │    │                                  │
 │ Caddy, Server, Workers   │    │ Docker + Sysbox                  │
 │ NATS, UI                 │    │ docker-socket-proxy (:2375)      │
 │                          │    │                                  │
 │ Workers ── reqwest ──────────► sandbox containers               │
 │            10.0.0.3:2375 │    │ (per-session, per-org network)   │
 └──────────┬───────────────┘    └──────────────────────────────────┘
            │
      Neon Postgres
```

| Control Plane VPS (cpx21) | Sandbox VPS (cpx31+) |
|---------------------------|---------------------|
| Caddy, Server, Workers, NATS, UI | Docker daemon + Sysbox daemons |
| DB credentials, Doppler secrets | docker-socket-proxy only |
| No Docker daemon, no Sysbox | No DB creds, no server, no secrets |
| Public internet (80/443) | Private network only (10.0.0.0/16) |

## Why Sysbox

Sysbox is the container runtime on the sandbox VPS. It provides VM-like isolation without hardware virtualization:

- **Mandatory user-namespace**: root in container = unprivileged UID on host (e.g., 165536)
- **procfs/sysfs virtualization**: sysbox-fs FUSE daemon emulates per-container kernel interfaces
- **Mount immutability**: init-time mounts can't be remounted read-write by container root
- **Fixed seccomp**: ~300 syscalls allowed, blocks bpf/ptrace/perf_event_open
- **~95-98% native performance**: only ~20 control-path syscalls intercepted (unlike gVisor which traps all)
- **Docker-in-Docker**: agents can run Docker inside their sandbox securely

The OSS `container_sandbox` capability sets `"Runtime": "sysbox-runc"` via the `CONTAINER_SANDBOX_RUNTIME` env var. No code change — just a Doppler config.

## Terraform Changes

### Private Network

```hcl
resource "hcloud_network" "internal" {
  name     = "everruns-dev-internal"
  ip_range = "10.0.0.0/16"
}

resource "hcloud_network_subnet" "internal" {
  network_id   = hcloud_network.internal.id
  type         = "cloud"
  network_zone = "us-east"
  ip_range     = "10.0.0.0/24"
}
```

### Control Plane VPS (add private network)

```hcl
resource "hcloud_server" "dev" {
  # ... unchanged ...
  network {
    network_id = hcloud_network.internal.id
    ip         = "10.0.0.2"
  }
}
```

### Sandbox VPS (new)

```hcl
resource "hcloud_firewall" "sandbox" {
  name = "everruns-dev-sandbox"

  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "22"
    source_ips = ["0.0.0.0/0", "::/0"]
  }
  # No 80/443 — no public services on sandbox VPS.
  # Docker API (2375) only via private network.
}

resource "hcloud_server" "sandbox" {
  name         = "everruns-dev-sandbox"
  server_type  = var.sandbox_server_type  # default "cpx31"
  location     = var.server_location      # same DC
  image        = "ubuntu-24.04"
  ssh_keys     = [hcloud_ssh_key.deploy.id]
  firewall_ids = [hcloud_firewall.sandbox.id]

  user_data = templatefile("${path.module}/cloud-init-sandbox.yaml", {
    ghcr_token    = var.ghcr_token
    ghcr_username = var.ghcr_username
  })

  network {
    network_id = hcloud_network.internal.id
    ip         = "10.0.0.3"
  }
}

variable "sandbox_server_type" {
  description = "Hetzner server type for sandbox VPS"
  type        = string
  default     = "cpx31"
}
```

## cloud-init-sandbox.yaml

```yaml
#cloud-config
# Sandbox VPS: Docker + Sysbox + socket proxy. No server, no UI, no secrets.

package_update: true
packages:
  - curl
  - iptables

runcmd:
  # Install Docker
  - curl -fsSL https://get.docker.com | sh
  - systemctl enable --now docker

  # Install Sysbox
  - wget -q https://downloads.nestybox.com/sysbox/releases/v0.7.0/sysbox-ce_0.7.0-0.linux_amd64.deb
  - apt-get install -y ./sysbox-ce_0.7.0-0.linux_amd64.deb
  - systemctl enable --now sysbox
  - rm sysbox-ce_0.7.0-0.linux_amd64.deb

  # Verify
  - docker info --format '{{.Runtimes}}' | grep -q sysbox-runc

  # Login to GHCR (for pulling sandbox base images)
  - echo "${ghcr_token}" | docker login ghcr.io -u "${ghcr_username}" --password-stdin

  # Pre-pull default sandbox image
  - docker pull ubuntu:24.04

  # Start socket proxy (listens on private network only)
  - |
    docker run -d --name docker-proxy --restart unless-stopped \
      -e CONTAINERS=1 -e POST=1 -e IMAGES=1 -e NETWORKS=1 -e EXEC=1 \
      -v /var/run/docker.sock:/var/run/docker.sock:ro \
      -p 10.0.0.3:2375:2375 \
      tecnativa/docker-socket-proxy

  # Block docker-proxy from public interface (defense-in-depth)
  - iptables -A INPUT -p tcp --dport 2375 -s 10.0.0.0/24 -j ACCEPT
  - iptables -A INPUT -p tcp --dport 2375 -j DROP
```

## Doppler Config

Add to `everruns` project, `dev` config:

| Variable | Value | Purpose |
|----------|-------|---------|
| `CONTAINER_SANDBOX_DOCKER_HOST` | `http://10.0.0.3:2375` | Sandbox VPS Docker API |
| `CONTAINER_SANDBOX_RUNTIME` | `sysbox-runc` | Sysbox isolation |
| `CONTAINER_SANDBOX_DEFAULT_IMAGE` | `ubuntu:24.04` | Default sandbox image |
| `CONTAINER_SANDBOX_MAX_PER_ORG` | `3` | Free tier limit |
| `CONTAINER_SANDBOX_MEMORY_LIMIT` | `2g` | Per-sandbox default |
| `CONTAINER_SANDBOX_CPU_LIMIT` | `1` | Per-sandbox default |

## docker-compose.dev.yml Changes

Add env vars to worker service:

```yaml
  worker:
    environment:
      - CONTAINER_SANDBOX_DOCKER_HOST
      - CONTAINER_SANDBOX_RUNTIME
      - CONTAINER_SANDBOX_MAX_PER_ORG
```

## Deploy Workflow Changes

Add sandbox VPS bootstrap to `deploy.yml`:

```yaml
  - name: Bootstrap Sandbox VPS
    run: |
      ssh -o StrictHostKeyChecking=no -i /tmp/deploy_key "root@$SANDBOX_HOST" << ENDSSH
      set -e
      # Verify Sysbox is running
      systemctl is-active sysbox
      # Verify socket proxy is running
      docker inspect docker-proxy --format '{{.State.Running}}'
      # Pull latest sandbox image
      docker pull ubuntu:24.04
      ENDSSH
```

Add `SANDBOX_HOST` env var (`10.0.0.3` or public IP for SSH access).

## Resource Budget

**Control Plane VPS (cpx21: 3 vCPU, 4 GB)** — unchanged.

**Sandbox VPS (cpx31: 4 vCPU, 8 GB)**:

| Component | RAM | CPU |
|-----------|-----|-----|
| Docker + Sysbox + proxy | ~210 MB | 0.2 |
| **Available for sandboxes** | **~7.5 GB** | **~3.8** |
| Sandboxes (2 GB / 1 CPU each) | **3-4 concurrent** | across all tenants |

Upgrade to cpx41 (8 vCPU, 16 GB) for ~7 concurrent sandboxes.

## Monthly Cost

| Component | Current | With Sandboxes |
|-----------|---------|---------------|
| Control plane (cpx21) | ~€8 | ~€8 (unchanged) |
| Sandbox VPS (cpx31) | — | ~€11 |
| **Total** | **~€8** | **~€19** |

## Scale-Out

Add more sandbox VPS instances on the same private network. Each gets its own Terraform resource, same cloud-init, different IP. Workers route via config list or load balancer. Per-org routing possible (premium tenants → dedicated host).

## Smoke Test

After deploy:
1. `curl http://10.0.0.3:2375/version` from control plane VPS → Docker API responds
2. Create session with `coding-container` harness → `sandbox_create` succeeds
3. `sandbox_exec "whoami"` → returns `root`
4. `sandbox_exec "cat /proc/self/uid_map"` → shows UID remapping (sysbox active)
5. Verify sandbox can't reach `10.0.0.2:9000` (control plane blocked)

## Linear Issues (could not create — workspace limit)

The following SaaS issues should be created when workspace capacity is available:

### SaaS-1: Terraform — private network + sandbox VPS
- Add `hcloud_network`, `hcloud_network_subnet`, attach to control plane VPS
- Add `hcloud_server.sandbox` with `cloud-init-sandbox.yaml`
- Add `hcloud_firewall.sandbox` (SSH only, no public services)
- Add `sandbox_server_type` variable
- Test: `terraform plan` shows new resources

### SaaS-2: cloud-init-sandbox.yaml
- Docker install, Sysbox install, socket proxy, iptables, GHCR login, image pre-pull
- Test: fresh VPS boots with Sysbox verified, proxy responding on private network

### SaaS-3: Doppler config + docker-compose.dev.yml
- Add `CONTAINER_SANDBOX_*` vars to `everruns/dev` Doppler project
- Add env pass-through in worker service
- Test: workers can reach `http://10.0.0.3:2375/version`

### SaaS-4: Deploy workflow updates
- Add sandbox VPS health check to deploy.yml
- Add `SANDBOX_HOST` to deploy env
- Test: deploy succeeds with both VPS healthy

### SaaS-5: End-to-end smoke test
- Create session with coding-container harness
- sandbox_create + sandbox_exec + sandbox_read_file + sandbox_manage
- Verify Sysbox isolation (uid_map)
- Verify network isolation (can't reach control plane)

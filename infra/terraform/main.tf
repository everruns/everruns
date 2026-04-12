# Everruns Dev Infrastructure — Hetzner Cloud
#
# Resources:
#   - Control plane VPS (server + workers + UI + DB)
#   - Private network (10.0.0.0/16) for internal communication
#   - Sandbox VPS (Docker + Sysbox for agent container execution)
#
# Usage:
#   terraform init
#   terraform plan
#   terraform apply

terraform {
  required_version = ">= 1.5"

  required_providers {
    hcloud = {
      source  = "hetznercloud/hcloud"
      version = "~> 1.49"
    }
  }
}

provider "hcloud" {
  token = var.hcloud_token
}

# ==========================================================================
# SSH Key
# ==========================================================================

resource "hcloud_ssh_key" "deploy" {
  name       = "everruns-dev-deploy"
  public_key = var.ssh_public_key
}

# ==========================================================================
# Firewall — Control Plane (HTTP, HTTPS, SSH)
# ==========================================================================

resource "hcloud_firewall" "control_plane" {
  name = "everruns-dev-control-plane"

  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "22"
    source_ips = ["0.0.0.0/0", "::/0"]
  }

  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "80"
    source_ips = ["0.0.0.0/0", "::/0"]
  }

  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "443"
    source_ips = ["0.0.0.0/0", "::/0"]
  }
}

# ==========================================================================
# Firewall — Sandbox (SSH only, no public services)
# ==========================================================================

resource "hcloud_firewall" "sandbox" {
  name = "everruns-dev-sandbox"

  rule {
    direction  = "in"
    protocol   = "tcp"
    port       = "22"
    source_ips = ["0.0.0.0/0", "::/0"]
  }
  # No 80/443 — Docker API (2375) only via private network
}

# ==========================================================================
# Private Network (control plane <-> sandbox communication)
# ==========================================================================

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

# ==========================================================================
# Control Plane VPS
# ==========================================================================

resource "hcloud_server" "dev" {
  name         = "everruns-dev"
  server_type  = var.server_type
  location     = var.server_location
  image        = "ubuntu-24.04"
  ssh_keys     = [hcloud_ssh_key.deploy.id]
  firewall_ids = [hcloud_firewall.control_plane.id]

  user_data = var.control_plane_user_data

  network {
    network_id = hcloud_network.internal.id
    ip         = "10.0.0.2"
  }
}

# ==========================================================================
# Sandbox VPS (Docker + Sysbox for agent container execution)
# ==========================================================================

resource "hcloud_server" "sandbox" {
  name         = "everruns-dev-sandbox"
  server_type  = var.sandbox_server_type
  location     = var.server_location
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

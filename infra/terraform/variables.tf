# Everruns Dev Infrastructure — Variables

variable "hcloud_token" {
  description = "Hetzner Cloud API token"
  type        = string
  sensitive   = true
}

variable "ssh_public_key" {
  description = "SSH public key for deploy access"
  type        = string
}

variable "server_type" {
  description = "Hetzner server type for control plane VPS"
  type        = string
  default     = "cpx31" # 4 vCPU, 8 GB
}

variable "sandbox_server_type" {
  description = "Hetzner server type for sandbox VPS"
  type        = string
  default     = "cpx31" # 4 vCPU, 8 GB. Upgrade to cpx41 for more headroom.
}

variable "server_location" {
  description = "Hetzner datacenter location"
  type        = string
  default     = "ash" # Ashburn, US East
}

variable "control_plane_user_data" {
  description = "Cloud-init user data for the control plane VPS"
  type        = string
  default     = ""
}

variable "ghcr_token" {
  description = "GitHub Container Registry token for sandbox image pulls"
  type        = string
  sensitive   = true
}

variable "ghcr_username" {
  description = "GitHub Container Registry username"
  type        = string
}

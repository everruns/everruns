# Everruns Dev Infrastructure — Outputs

output "control_plane_ip" {
  value       = hcloud_server.dev.ipv4_address
  description = "Control plane public IPv4 address"
}

output "control_plane_private_ip" {
  value       = "10.0.0.2"
  description = "Control plane private network IP"
}

output "sandbox_ip" {
  value       = hcloud_server.sandbox.ipv4_address
  description = "Sandbox VPS public IPv4 address"
}

output "sandbox_private_ip" {
  value       = "10.0.0.3"
  description = "Sandbox VPS private network IP"
}

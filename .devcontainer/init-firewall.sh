#!/bin/bash
# Firewall initialization script for Everruns devcontainer
# Restricts outbound connections to whitelisted domains only

set -euo pipefail

echo "Initializing firewall..."

# Skip if not running as root
if [ "$(id -u)" != "0" ]; then
    echo "Warning: Firewall script must run as root. Skipping firewall setup."
    exit 0
fi

# Extract Docker DNS rules before flushing
DOCKER_DNS_RULES=$(iptables-save 2>/dev/null | grep -E "DOCKER|docker" || true)

# Flush existing rules
iptables -F OUTPUT 2>/dev/null || true
iptables -F INPUT 2>/dev/null || true
ipset destroy allowed-domains 2>/dev/null || true

# Create ipset for allowed domains
ipset create allowed-domains hash:net family inet hashsize 4096 maxelem 65536

# Function to resolve domain and add to ipset
add_domain() {
    local domain=$1
    echo "  Adding domain: $domain"
    local ips
    ips=$(dig +short "$domain" A 2>/dev/null | grep -E '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$' || true)
    for ip in $ips; do
        ipset add allowed-domains "$ip/32" 2>/dev/null || true
    done
}

# Function to add CIDR range
add_cidr() {
    local cidr=$1
    if [[ "$cidr" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+/[0-9]+$ ]]; then
        ipset add allowed-domains "$cidr" 2>/dev/null || true
    fi
}

echo "Adding allowed domains..."

# npm registry
add_domain "registry.npmjs.org"
add_domain "registry.yarnpkg.com"

# GitHub (for git operations and API)
echo "  Fetching GitHub IP ranges..."
GITHUB_META=$(curl -s https://api.github.com/meta 2>/dev/null || echo "{}")
for key in git hooks web api; do
    echo "$GITHUB_META" | jq -r ".${key}[]? // empty" 2>/dev/null | while read -r cidr; do
        add_cidr "$cidr"
    done
done
add_domain "github.com"
add_domain "api.github.com"
add_domain "raw.githubusercontent.com"
add_domain "objects.githubusercontent.com"
add_domain "codeload.github.com"

# Anthropic API (for Claude)
add_domain "api.anthropic.com"
add_domain "claude.ai"

# OpenAI API (for LLM operations)
add_domain "api.openai.com"

# Rust/Cargo
add_domain "crates.io"
add_domain "static.crates.io"
add_domain "index.crates.io"
add_domain "static.rust-lang.org"

# VS Code / Development
add_domain "update.code.visualstudio.com"
add_domain "marketplace.visualstudio.com"
add_domain "vscode.blob.core.windows.net"
add_domain "az764295.vo.msecnd.net"

# Sentry (error reporting)
add_domain "sentry.io"
add_domain "o19635.ingest.sentry.io"

# Temporal (if using cloud)
add_domain "temporal.io"

# Docker Hub (for docker-in-docker)
add_domain "registry-1.docker.io"
add_domain "auth.docker.io"
add_domain "production.cloudflare.docker.com"

# Allow localhost ranges
ipset add allowed-domains 127.0.0.0/8 2>/dev/null || true
ipset add allowed-domains 10.0.0.0/8 2>/dev/null || true
ipset add allowed-domains 172.16.0.0/12 2>/dev/null || true
ipset add allowed-domains 192.168.0.0/16 2>/dev/null || true

# Detect host IP and local network
HOST_IP=$(ip route | grep default | awk '{print $3}' || echo "")
if [ -n "$HOST_IP" ]; then
    echo "  Adding host network: $HOST_IP"
    ipset add allowed-domains "$HOST_IP/32" 2>/dev/null || true

    # Calculate network range
    NETWORK=$(ip route | grep -v default | grep -E "^[0-9]" | head -1 | awk '{print $1}' || echo "")
    if [ -n "$NETWORK" ]; then
        echo "  Adding local network: $NETWORK"
        add_cidr "$NETWORK"
    fi
fi

echo "Applying firewall rules..."

# Allow all loopback traffic
iptables -A INPUT -i lo -j ACCEPT
iptables -A OUTPUT -o lo -j ACCEPT

# Allow established connections
iptables -A INPUT -m state --state ESTABLISHED,RELATED -j ACCEPT
iptables -A OUTPUT -m state --state ESTABLISHED,RELATED -j ACCEPT

# Allow DNS (UDP and TCP port 53)
iptables -A OUTPUT -p udp --dport 53 -j ACCEPT
iptables -A OUTPUT -p tcp --dport 53 -j ACCEPT

# Allow SSH (port 22)
iptables -A OUTPUT -p tcp --dport 22 -j ACCEPT

# Allow connections to whitelisted IPs
iptables -A OUTPUT -m set --match-set allowed-domains dst -j ACCEPT

# Restore Docker DNS rules if any
if [ -n "$DOCKER_DNS_RULES" ]; then
    echo "$DOCKER_DNS_RULES" | iptables-restore -n 2>/dev/null || true
fi

# Set default policy to DROP for OUTPUT (but allow established)
# Note: We don't DROP INPUT to allow container communication
iptables -P OUTPUT DROP

echo "Verifying firewall configuration..."

# Test that blocked sites are unreachable
if timeout 3 curl -s --connect-timeout 2 http://example.com >/dev/null 2>&1; then
    echo "Warning: Firewall verification failed - example.com is reachable"
else
    echo "  Blocked sites unreachable (expected)"
fi

# Test that allowed sites are reachable
if timeout 5 curl -s --connect-timeout 3 https://api.github.com >/dev/null 2>&1; then
    echo "  GitHub API reachable (expected)"
else
    echo "Warning: GitHub API unreachable - firewall may be too restrictive"
fi

echo "Firewall initialization complete!"
echo ""
echo "Allowed outbound connections:"
echo "  - DNS (port 53)"
echo "  - SSH (port 22)"
echo "  - npm registry"
echo "  - GitHub"
echo "  - Anthropic API"
echo "  - OpenAI API"
echo "  - Rust crates.io"
echo "  - VS Code services"
echo "  - Docker Hub"
echo "  - Local networks"

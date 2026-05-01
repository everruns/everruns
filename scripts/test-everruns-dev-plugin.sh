#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

python3 - <<'PY' "$PROJECT_ROOT"
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
plugin_dir = root / "plugins" / "everruns-dev"
expected_url = "https://dev.everruns.com/mcp"

mcp = json.loads((plugin_dir / ".mcp.json").read_text())
server = mcp["mcpServers"]["everruns-dev"]

if server["url"] != expected_url:
    raise SystemExit(f"Everruns Dev MCP URL changed unexpectedly: {server['url']}")

if server.get("oauth_resource") != expected_url:
    raise SystemExit(
        "Everruns Dev plugin must declare oauth_resource so Codex MCP login "
        "includes the RFC 8707 resource parameter without user config."
    )

if server.get("scopes"):
    raise SystemExit(
        "Everruns Dev plugin must not request OAuth scopes; "
        "PropelAuth rejects them for this MCP resource."
    )

codex = json.loads((plugin_dir / ".codex-plugin" / "plugin.json").read_text())
claude = json.loads((plugin_dir / ".claude-plugin" / "plugin.json").read_text())
marketplace = json.loads((root / ".claude-plugin" / "marketplace.json").read_text())
marketplace_plugin = marketplace["plugins"][0]

versions = {
    "codex": codex["version"],
    "claude": claude["version"],
    "marketplace": marketplace_plugin["version"],
}

if len(set(versions.values())) != 1:
    raise SystemExit(f"Everruns Dev plugin versions diverged: {versions}")

print("everruns-dev plugin metadata checks passed")
PY

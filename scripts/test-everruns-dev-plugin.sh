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
claude_marketplace = json.loads((root / ".claude-plugin" / "marketplace.json").read_text())
codex_marketplace = json.loads((root / ".agents" / "plugins" / "marketplace.json").read_text())


def marketplace_plugin(label, marketplace, name):
    for plugin in marketplace["plugins"]:
        if plugin["name"] == name:
            return plugin
    raise SystemExit(f"{label} marketplace is missing {name} plugin registration")


claude_marketplace_plugin = marketplace_plugin(
    "Claude Code", claude_marketplace, "everruns-dev"
)
codex_marketplace_plugin = marketplace_plugin(
    "Codex", codex_marketplace, "everruns-dev"
)

for label, plugin in {"codex": codex, "claude": claude}.items():
    if plugin["name"] != "everruns-dev":
        raise SystemExit(f"Everruns Dev {label} plugin name drifted: {plugin['name']}")

versions = {
    "codex": codex["version"],
    "claude": claude["version"],
    "claude_marketplace": claude_marketplace_plugin["version"],
}

if len(set(versions.values())) != 1:
    raise SystemExit(f"Everruns Dev plugin versions diverged: {versions}")

if claude_marketplace_plugin["source"] != "./plugins/everruns-dev":
    raise SystemExit(
        "Claude Code marketplace must point at ./plugins/everruns-dev"
    )

codex_source = codex_marketplace_plugin["source"]
if codex_source.get("source") != "local" or codex_source.get("path") != "./plugins/everruns-dev":
    raise SystemExit(
        "Codex marketplace must point at local ./plugins/everruns-dev"
    )

if "category" in claude:
    raise SystemExit(
        "Claude plugin.json must not declare 'category' — it is not part of "
        "the Claude Code plugin manifest schema. Put it on the marketplace "
        "plugin entry instead."
    )

if not claude_marketplace.get("description"):
    raise SystemExit(
        "marketplace.json must declare a top-level 'description' so the "
        "marketplace renders correctly in Claude Code's plugin browser."
    )

print("everruns-dev plugin metadata checks passed")
PY

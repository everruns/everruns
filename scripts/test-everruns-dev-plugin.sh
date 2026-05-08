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

if "interface" in claude:
    raise SystemExit(
        "Claude plugin.json must not declare 'interface' — it is a Codex-only "
        "field. Adding it to the Claude manifest breaks plugin loading."
    )

if not claude_marketplace.get("description"):
    raise SystemExit(
        "marketplace.json must declare a top-level 'description' so the "
        "marketplace renders correctly in Claude Code's plugin browser."
    )

# Both manifests must declare the shared payload explicitly so each host
# loads the same skills and MCP server. See specs/everruns-dev-plugin.md.
expected_pointers = {"skills": "./skills/", "mcpServers": "./.mcp.json"}
for label, plugin in {"codex": codex, "claude": claude}.items():
    for key, value in expected_pointers.items():
        if plugin.get(key) != value:
            raise SystemExit(
                f"Everruns Dev {label} plugin must declare {key!r}: {value!r} "
                f"to keep Claude Code and Codex pointing at the same shared "
                f"payload (got {plugin.get(key)!r}). "
                f"See specs/everruns-dev-plugin.md."
            )

# Shared metadata that must match verbatim between both host manifests.
shared_keys = ("author", "homepage", "repository", "license", "keywords")
for key in shared_keys:
    if codex.get(key) != claude.get(key):
        raise SystemExit(
            f"Everruns Dev plugin {key!r} drifted between Claude and Codex "
            f"manifests. Keep them identical. "
            f"See specs/everruns-dev-plugin.md."
        )

# Descriptions must match aside from the optional Codex host marker.
# The marker is allowed to appear at most once, anywhere in the Codex
# description; any other deviation (multiple insertions, different wording)
# is treated as drift.
codex_desc = codex.get("description")
claude_desc = claude.get("description")
if not codex_desc or not claude_desc:
    raise SystemExit(
        "Both Everruns Dev plugin.json files must declare a non-empty "
        "'description'. See specs/everruns-dev-plugin.md."
    )

marker = " from Codex"
matches_exact = codex_desc == claude_desc
matches_with_marker = (
    codex_desc.count(marker) == 1
    and codex_desc.replace(marker, "", 1) == claude_desc
)
if not (matches_exact or matches_with_marker):
    raise SystemExit(
        "Everruns Dev description drifted between Claude and Codex manifests. "
        "Codex may insert ' from Codex' exactly once; otherwise the wording "
        "must match. See specs/everruns-dev-plugin.md."
    )

skill = (plugin_dir / "skills" / "everruns-dev" / "SKILL.md").read_text()
if "switch_organization" in skill:
    raise SystemExit(
        "Everruns Dev skill must not mention switch_organization; MCP org "
        "selection is per-call via organization_id."
    )

required_multi_org_phrases = [
    "MCP has no current-org switch",
    '"organization_id": "org_01933b5a00007000800000000000004"',
    "Do not pass `--organization_id` to builtins",
]
missing = [phrase for phrase in required_multi_org_phrases if phrase not in skill]
if missing:
    raise SystemExit(f"Everruns Dev skill is missing multi-org guidance: {missing}")

print("everruns-dev plugin metadata checks passed")
PY

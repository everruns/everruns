#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

python3 - <<'PY' "$PROJECT_ROOT"
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])

PLUGIN_CONFIGS = [
    {
        "name": "everruns-dev",
        "label": "Everruns Dev",
        "url": "https://dev.everruns.com/mcp",
    },
    {
        "name": "everruns",
        "label": "Everruns",
        "url": "https://app.everruns.com/mcp",
    },
]

claude_marketplace = json.loads((root / ".claude-plugin" / "marketplace.json").read_text())
codex_marketplace = json.loads((root / ".agents" / "plugins" / "marketplace.json").read_text())
cursor_marketplace = json.loads((root / ".cursor-plugin" / "marketplace.json").read_text())

if codex_marketplace.get("name") != "everruns":
    raise SystemExit(
        "Codex marketplace top-level name must be 'everruns'. It contains both "
        "the dev and production plugins, and Codex renders this source name in "
        "the plugin directory."
    )

if codex_marketplace.get("interface", {}).get("displayName") != "Everruns":
    raise SystemExit(
        "Codex marketplace interface.displayName must be 'Everruns' because the "
        "marketplace contains both dev and production plugin entries."
    )


def marketplace_plugin(label, marketplace, name):
    for plugin in marketplace["plugins"]:
        if plugin["name"] == name:
            return plugin
    raise SystemExit(f"{label} marketplace is missing {name} plugin registration")


def description_matches(host_desc: str, base_desc: str, marker: str) -> bool:
    if host_desc == base_desc:
        return True
    return (
        host_desc.count(marker) == 1
        and host_desc.replace(marker, "", 1) == base_desc
    )


def validate_plugin(config):
    name = config["name"]
    label = config["label"]
    expected_url = config["url"]
    plugin_dir = root / "plugins" / name
    expected_source = f"./plugins/{name}"

    mcp = json.loads((plugin_dir / ".mcp.json").read_text())
    server = mcp["mcpServers"][name]

    if server["url"] != expected_url:
        raise SystemExit(f"{label} MCP URL changed unexpectedly: {server['url']}")

    if server.get("oauth_resource") != expected_url:
        raise SystemExit(
            f"{label} plugin must declare oauth_resource so Codex MCP login "
            "includes the RFC 8707 resource parameter without user config."
        )

    if server.get("scopes"):
        raise SystemExit(
            f"{label} plugin must not request OAuth scopes; "
            "PropelAuth rejects them for this MCP resource."
        )

    codex = json.loads((plugin_dir / ".codex-plugin" / "plugin.json").read_text())
    claude = json.loads((plugin_dir / ".claude-plugin" / "plugin.json").read_text())
    cursor = json.loads((plugin_dir / ".cursor-plugin" / "plugin.json").read_text())

    codex_interface = codex.get("interface")
    if not isinstance(codex_interface, dict):
        raise SystemExit("Codex plugin.json must declare an 'interface' object")

    codex_default_prompts = codex_interface.get("defaultPrompt", [])
    if not isinstance(codex_default_prompts, list):
        raise SystemExit("Codex interface.defaultPrompt must be a list")

    for index, prompt in enumerate(codex_default_prompts):
        if not isinstance(prompt, str):
            raise SystemExit(f"Codex defaultPrompt[{index}] must be a string")
        if len(prompt) > 128:
            raise SystemExit(
                f"Codex defaultPrompt[{index}] is too long: {len(prompt)} > 128"
            )

    claude_marketplace_plugin = marketplace_plugin("Claude Code", claude_marketplace, name)
    codex_marketplace_plugin = marketplace_plugin("Codex", codex_marketplace, name)
    cursor_marketplace_plugin = marketplace_plugin("Cursor", cursor_marketplace, name)

    for host_label, plugin in {"codex": codex, "claude": claude, "cursor": cursor}.items():
        if plugin["name"] != name:
            raise SystemExit(f"{label} {host_label} plugin name drifted: {plugin['name']}")

    versions = {
        "codex": codex["version"],
        "claude": claude["version"],
        "cursor": cursor["version"],
        "claude_marketplace": claude_marketplace_plugin["version"],
        "cursor_marketplace_plugin": cursor_marketplace_plugin["version"],
        "cursor_marketplace_metadata": cursor_marketplace["metadata"]["version"],
    }

    if len(set(versions.values())) != 1:
        raise SystemExit(f"{label} plugin versions diverged: {versions}")

    if claude_marketplace_plugin["source"] != expected_source:
        raise SystemExit(f"Claude Code marketplace must point at {expected_source}")

    codex_source = codex_marketplace_plugin["source"]
    if codex_source.get("source") != "local" or codex_source.get("path") != expected_source:
        raise SystemExit(f"Codex marketplace must point at local {expected_source}")

    if cursor_marketplace_plugin["source"] != expected_source:
        raise SystemExit(f"Cursor marketplace must point at {expected_source}")

    if cursor.get("mcpServers") != "./.mcp.json":
        raise SystemExit(
            "Cursor plugin manifest must reference the shared ./.mcp.json so all "
            "three hosts read the same MCP server config."
        )

    cursor_name_pattern = r"^[a-z0-9]([a-z0-9.-]*[a-z0-9])?$"
    if not re.match(cursor_name_pattern, cursor["name"]):
        raise SystemExit(
            "Cursor plugin name must match Cursor marketplace pattern "
            f"{cursor_name_pattern}: got {cursor['name']!r}"
        )

    if not cursor.get("displayName"):
        raise SystemExit("Cursor plugin manifest must declare displayName.")

    cursor_logo = cursor.get("logo", "")
    if not cursor_logo or (cursor_logo.startswith(("http://", "https://")) is False
                           and not (plugin_dir / cursor_logo).exists()):
        raise SystemExit(
            f"Cursor plugin logo path is invalid or missing on disk: {cursor_logo!r}"
        )

    commands_dir = plugin_dir / "commands"
    for command_file in sorted(commands_dir.glob("*.md")):
        text = command_file.read_text()
        if not text.startswith("---\n"):
            raise SystemExit(f"Command file missing frontmatter: {command_file}")
        end = text.find("\n---\n", 4)
        if end == -1:
            raise SystemExit(f"Command file frontmatter unterminated: {command_file}")
        block = text[4:end]
        fields = {}
        for line in block.split("\n"):
            if ":" in line and not line.lstrip().startswith("#"):
                key, _, value = line.partition(":")
                fields[key.strip()] = value.strip()
        expected_name = command_file.stem
        if fields.get("name") != expected_name:
            raise SystemExit(
                f"Command {command_file.name} must declare 'name: {expected_name}' "
                f"in frontmatter for Cursor compatibility (got {fields.get('name')!r})."
            )
        if not fields.get("description"):
            raise SystemExit(f"Command {command_file.name} missing 'description' frontmatter.")

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

    expected_pointers = {"skills": "./skills/", "mcpServers": "./.mcp.json"}
    for host_label, plugin in {"codex": codex, "claude": claude, "cursor": cursor}.items():
        for key, value in expected_pointers.items():
            if plugin.get(key) != value:
                raise SystemExit(
                    f"{label} {host_label} plugin must declare {key!r}: {value!r} "
                    f"to keep all hosts pointing at the same shared payload "
                    f"(got {plugin.get(key)!r}). "
                    "See specs/everruns-dev-plugin.md."
                )

    shared_keys = ("author", "homepage", "repository", "license", "keywords")
    for key in shared_keys:
        if codex.get(key) != claude.get(key):
            raise SystemExit(
                f"{label} plugin {key!r} drifted between Claude and Codex "
                "manifests. Keep them identical. "
                "See specs/everruns-dev-plugin.md."
            )

    cursor_shared_keys = ("homepage", "repository", "license", "keywords")
    for key in cursor_shared_keys:
        if cursor.get(key) != claude.get(key):
            raise SystemExit(
                f"{label} cursor plugin {key!r} drifted from the Claude "
                "manifest. Keep them identical. "
                "See specs/everruns-dev-plugin.md."
            )

    if cursor.get("author", {}).get("name") != claude.get("author", {}).get("name"):
        raise SystemExit(
            f"{label} cursor plugin author.name drifted from the Claude "
            "manifest. Cursor manifests cannot share the full author object "
            "(schema rejects `author.url`); keep `author.name` aligned. "
            "See specs/everruns-dev-plugin.md."
        )

    codex_desc = codex.get("description")
    claude_desc = claude.get("description")
    cursor_desc = cursor.get("description")
    if not codex_desc or not claude_desc or not cursor_desc:
        raise SystemExit(
            f"Every {label} plugin.json must declare a non-empty "
            "'description'. See specs/everruns-dev-plugin.md."
        )

    if not description_matches(codex_desc, claude_desc, " from Codex"):
        raise SystemExit(
            f"{label} description drifted between Claude and Codex manifests. "
            "Codex may insert ' from Codex' exactly once; otherwise the wording "
            "must match. See specs/everruns-dev-plugin.md."
        )

    if not description_matches(cursor_desc, claude_desc, " from Cursor"):
        raise SystemExit(
            f"{label} description drifted between Claude and Cursor manifests. "
            "Cursor may insert ' from Cursor' exactly once; otherwise the wording "
            "must match. See specs/everruns-dev-plugin.md."
        )

    skill = (plugin_dir / "skills" / name / "SKILL.md").read_text()
    if "switch_organization" in skill:
        raise SystemExit(
            f"{label} skill must not mention switch_organization; MCP org "
            "selection is per-call via organization_id."
        )

    required_multi_org_phrases = [
        "MCP has no current-org switch",
        '"organization_id": "org_01933b5a00007000800000000000004"',
        "Do not pass `--organization_id` to builtins",
    ]
    missing = [phrase for phrase in required_multi_org_phrases if phrase not in skill]
    if missing:
        raise SystemExit(f"{label} skill is missing multi-org guidance: {missing}")


for config in PLUGIN_CONFIGS:
    validate_plugin(config)

print("Everruns plugin metadata checks passed (dev and production; claude, codex, cursor)")
PY

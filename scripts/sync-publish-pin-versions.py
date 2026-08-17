#!/usr/bin/env python3
"""Sync path-dependency version pins to the workspace package version.

The publish-crates workflow rejects releases when a sub-crate's pinned path
dependency does not match the workspace tag (e.g. `version = "0.8.31"` while
the workspace is at `0.8.32`). This script enforces the invariant.

Two modes:
  --check   exit 1 if any pin drifts from workspace.package.version
  --write   rewrite the pinned versions in place to match (default)

The pin set mirrors `dependency_versions` in
`.github/workflows/publish-crates.yml` and `workspace.dependencies` in
`Cargo.toml`.
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# Path-pin versions kept in sub-crate manifests. Keep in sync with
# .github/workflows/publish-crates.yml `dependency_versions`.
INNER_PINS: dict[str, list[str]] = {
    "crates/engine/Cargo.toml": ["everruns-core"],
    "crates/builtins/Cargo.toml": [
        "everruns-openui",
        "everruns-a2ui",
        "everruns-core",
    ],
    "crates/platform/Cargo.toml": [
        "everruns-builtins",
        "everruns-core",
        "everruns-host",
        "everruns-session-services",
        "everruns-integrations-filesystem",
        "everruns-integrations-bashkit",
        "everruns-integrations-web-fetch",
        "everruns-integrations-lua",
        "everruns-integrations-openrouter-workspace",
    ],
    "crates/http/Cargo.toml": ["everruns-core"],
    "crates/mcp/Cargo.toml": ["everruns-core", "everruns-http"],
    "crates/host/Cargo.toml": [
        "everruns-builtins",
        "everruns-core",
        "everruns-engine",
        "everruns-http",
        "everruns-mcp",
        "everruns-session-services",
        "everruns-integrations-filesystem",
        "everruns-integrations-bashkit",
        "everruns-integrations-web-fetch",
        "everruns-integrations-lua",
    ],
    "crates/everruns/Cargo.toml": [
        "everruns-builtins",
        "everruns-core",
        "everruns-host",
        "everruns-local",
        "everruns-macros",
        "everruns-llmsim",
    ],
    "crates/llmsim/Cargo.toml": ["everruns-provider", "everruns-host"],
    "crates/test-support/Cargo.toml": ["everruns-core", "everruns-llmsim"],
    "crates/local/Cargo.toml": [
        "everruns-core",
        "everruns-host",
        "everruns-platform",
    ],
    "crates/session-services/Cargo.toml": [
        "everruns-capability",
        "everruns-core",
        "everruns-provider",
    ],
    "crates/openai/Cargo.toml": ["everruns-provider"],
    "crates/anthropic/Cargo.toml": ["everruns-provider"],
    "crates/openrouter/Cargo.toml": ["everruns-provider"],
    "crates/bedrock/Cargo.toml": ["everruns-provider"],
    "crates/gemini/Cargo.toml": ["everruns-provider"],
    "crates/meta/Cargo.toml": ["everruns-provider"],
    "integrations/duckduckgo/Cargo.toml": ["everruns-core"],
    "integrations/filesystem/Cargo.toml": ["everruns-core"],
    "integrations/bashkit/Cargo.toml": ["everruns-core"],
    "integrations/web-fetch/Cargo.toml": ["everruns-core"],
    "integrations/lua/Cargo.toml": ["everruns-core"],
    "integrations/openrouter-workspace/Cargo.toml": ["everruns-core"],
    "integrations/github/Cargo.toml": ["everruns-core"],
    "integrations/daytona/Cargo.toml": ["everruns-core"],
    "integrations/e2b/Cargo.toml": ["everruns-core"],
    "integrations/parallel/Cargo.toml": ["everruns-core"],
    "integrations/brave-search/Cargo.toml": ["everruns-core"],
    "integrations/browserless/Cargo.toml": ["everruns-core"],
    "integrations/cursor/Cargo.toml": ["everruns-core"],
    "integrations/deno/Cargo.toml": ["everruns-core"],
    "integrations/docker/Cargo.toml": ["everruns-core"],
    "integrations/sprites/Cargo.toml": ["everruns-core"],
}

# Workspace.dependencies path pins (root Cargo.toml).
WORKSPACE_PIN_DEPS: list[str] = [
    "everruns-builtins",
    "everruns-capability",
    "everruns-core",
    "everruns-engine",
    "everruns-host",
    "everruns-http",
    "everruns-platform",
    "everruns-provider",
    "everruns-mcp",
    "everruns-session-services",
    "everruns-local",
    "everruns-ard",
    "everruns",
    "everruns-macros",
    "everruns-llmsim",
    "everruns-test-support",
    "everruns-integrations-filesystem",
    "everruns-integrations-bashkit",
    "everruns-integrations-web-fetch",
    "everruns-integrations-lua",
    "everruns-integrations-openrouter-workspace",
]


def workspace_version() -> str:
    with (REPO / "Cargo.toml").open("rb") as fh:
        return tomllib.load(fh)["workspace"]["package"]["version"]


def rewrite_pin(path: Path, dep_name: str, target: str) -> bool:
    """Rewrite `dep_name = { ..., version = "X" }` in `path`. Returns True if changed."""
    text = path.read_text()
    pattern = re.compile(
        rf'^({re.escape(dep_name)}\s*=\s*\{{[^}}\n]*?version\s*=\s*")[^"]+(")',
        re.MULTILINE,
    )
    new_text, n = pattern.subn(rf"\g<1>{target}\g<2>", text, count=1)
    if n == 0:
        return False
    if new_text == text:
        return False
    path.write_text(new_text)
    return True


def current_pin(path: Path, dep_name: str) -> str | None:
    """Return the version pin for `dep_name`, or None if it inherits via `workspace = true`."""
    with path.open("rb") as fh:
        manifest = tomllib.load(fh)
    dep = manifest.get("dependencies", {}).get(dep_name)
    if isinstance(dep, dict):
        if dep.get("workspace") is True:
            return None
        return dep.get("version")
    # workspace.dependencies entries
    ws_deps = manifest.get("workspace", {}).get("dependencies", {}).get(dep_name)
    if isinstance(ws_deps, dict):
        return ws_deps.get("version")
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="report drift, do not modify files")
    mode.add_argument("--write", action="store_true", help="rewrite drifted pins (default)")
    args = parser.parse_args()
    check_only = args.check

    target = workspace_version()
    drift: list[tuple[Path, str, str | None]] = []
    changed: list[tuple[Path, str]] = []

    for manifest_rel, deps in INNER_PINS.items():
        path = REPO / manifest_rel
        for dep in deps:
            current = current_pin(path, dep)
            # `None` means the dep uses workspace = true or has no pin — nothing to sync.
            if current is None or current == target:
                continue
            drift.append((path, dep, current))
            if not check_only:
                if rewrite_pin(path, dep, target):
                    changed.append((path, dep))

    root = REPO / "Cargo.toml"
    for dep in WORKSPACE_PIN_DEPS:
        current = current_pin(root, dep)
        if current == target:
            continue
        drift.append((root, dep, current))
        if not check_only:
            if rewrite_pin(root, dep, target):
                changed.append((root, dep))

    if check_only:
        if drift:
            print(f"version pin drift (workspace = {target}):", file=sys.stderr)
            for path, dep, current in drift:
                print(f"  {path.relative_to(REPO)}: {dep} = {current!r}", file=sys.stderr)
            return 1
        print(f"all path-dep pins at {target}")
        return 0

    if changed:
        print(f"synced {len(changed)} pin(s) to {target}:")
        for path, dep in changed:
            print(f"  {path.relative_to(REPO)}: {dep}")
    else:
        print(f"all path-dep pins already at {target}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

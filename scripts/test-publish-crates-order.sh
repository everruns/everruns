#!/usr/bin/env bash
# Guard independent crates.io releases and dependency-version ownership.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_ROOT"

python3 scripts/sync-publish-pin-versions.py --check

python3 - <<'PY'
import json
import pathlib
import subprocess
import sys
import tomllib

repo = pathlib.Path.cwd()
workflow = (repo / ".github/workflows/publish-crates.yml").read_text()
release = (repo / ".github/workflows/release.yml").read_text()
failures: list[str] = []


def require(condition: bool, message: str) -> None:
    if not condition:
        failures.append(message)


require("types: [publish-crate]" in workflow, "crate workflow must use the singular publish-crate dispatch")
require("package:" in workflow, "crate workflow must require a package input")
require("crate/$DISPATCH_PACKAGE/v$VERSION" in workflow, "crate tags must bind package name and version")
require('cargo publish --locked --no-verify --package "$PACKAGE"' in workflow, "workflow must publish only the selected package")
require("CRATES=(" not in workflow, "workflow must not retain a bulk crate publish list")
require("workspace package version" not in workflow.lower(), "workflow must not validate against the product workspace version")
require("publish-crates" not in release and "publish-crate" not in release, "product releases must not dispatch library publishing")

metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--no-deps", "--format-version", "1"], text=True
))
published = [package for package in metadata["packages"] if package.get("publish") != []]
for package in published:
    manifest_path = pathlib.Path(package["manifest_path"])
    with manifest_path.open("rb") as handle:
        manifest = tomllib.load(handle)
    require(
        isinstance(manifest.get("package", {}).get("version"), str),
        f"{manifest_path.relative_to(repo)}: published package must own an explicit version",
    )

legacy_macros = repo / "crates/everruns-macros"
require(not legacy_macros.exists(), "legacy crates/everruns-macros path must not exist")
macros_manifest = repo / "crates/macros/Cargo.toml"
require(macros_manifest.is_file(), "everruns-macros must live at crates/macros/Cargo.toml")

if failures:
    for failure in failures:
        print(f"FAIL: {failure}", file=sys.stderr)
    raise SystemExit(1)

print(f"independent publishing verified for {len(published)} crate(s)")
PY

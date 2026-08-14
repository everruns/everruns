#!/usr/bin/env bash
# Guards the crates.io release contract in .github/workflows/publish-crates.yml:
# a crate can only publish once every workspace dependency it carries is already
# on crates.io, so the workflow's CRATES list must be a topological order of the
# workspace dependency graph, and every such dependency must carry a version.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

python3 - <<'PY'
import pathlib
import re
import sys
import tomllib

REPO = pathlib.Path(".").resolve()
WORKFLOW = REPO / ".github/workflows/publish-crates.yml"
workflow_text = WORKFLOW.read_text()

failures: list[str] = []


def fail(message: str) -> None:
    failures.append(message)


def load(path: pathlib.Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


workspace = load(REPO / "Cargo.toml")
workspace_dependencies = workspace["workspace"].get("dependencies", {})

# The published package remains `everruns-macros`, but its repository path is
# intentionally the concise `crates/macros`. Guard both sides so a future edit
# cannot quietly restore the legacy directory while the dynamic scan still
# finds a valid package.
legacy_macros_path = REPO / "crates/everruns-macros"
expected_macros_manifest = REPO / "crates/macros/Cargo.toml"
if legacy_macros_path.exists():
    fail("legacy crates/everruns-macros path must not exist; use crates/macros")
if not expected_macros_manifest.is_file():
    fail("everruns-macros manifest must live at crates/macros/Cargo.toml")
workspace_macros = workspace_dependencies.get("everruns-macros", {})
if not isinstance(workspace_macros, dict) or workspace_macros.get("path") != "crates/macros":
    fail("workspace dependency everruns-macros must use path crates/macros")

# Every workspace member manifest, keyed by package name.
manifests: dict[str, pathlib.Path] = {}
for member_glob in ("crates/*/Cargo.toml", "integrations/*/Cargo.toml"):
    for manifest_path in REPO.glob(member_glob):
        package = load(manifest_path).get("package")
        if package and "name" in package:
            manifests[package["name"]] = manifest_path

if manifests.get("everruns-macros") != expected_macros_manifest:
    fail("package everruns-macros must resolve to crates/macros/Cargo.toml")

# The bash CRATES=( ... ) array the publish loop iterates.
match = re.search(r"^\s*CRATES=\(\n(.*?)^\s*\)\s*$", workflow_text, re.MULTILINE | re.DOTALL)
if match is None:
    raise SystemExit("could not find the CRATES=( ... ) list in publish-crates.yml")
published = [
    line.strip()
    for line in match.group(1).splitlines()
    if line.strip() and not line.strip().startswith("#")
]

order = {crate: index for index, crate in enumerate(published)}

if len(order) != len(published):
    fail("publish-crates.yml CRATES list contains duplicate entries")

for crate in published:
    if crate not in manifests:
        fail(f"publish-crates.yml publishes unknown package {crate}")

# Ordering + version pins: for each published crate, look at the workspace
# dependencies it carries (including optional ones — they are still recorded in
# the packaged manifest and must resolve on crates.io).
for crate in published:
    if crate not in manifests:
        continue
    manifest_path = manifests[crate]
    manifest = load(manifest_path)
    relative = manifest_path.relative_to(REPO)
    for name, dependency in manifest.get("dependencies", {}).items():
        if not name.startswith("everruns") or name not in manifests:
            continue
        if isinstance(dependency, dict) and dependency.get("workspace"):
            resolved = workspace_dependencies.get(name, {})
        else:
            resolved = dependency
        version = resolved.get("version") if isinstance(resolved, dict) else resolved
        if not version:
            fail(f"{relative}: dependency {name} has no crates.io version, so {crate} cannot publish")
        if name not in order:
            fail(f"{relative}: {crate} depends on {name}, which publish-crates.yml never publishes")
        elif order[name] > order[crate]:
            fail(f"publish-crates.yml publishes {crate} before its dependency {name}")

# Spot-check the edges this contract exists for (EVE-846), so a refactor that
# drops a crate from the list fails loudly rather than silently passing.
for earlier, later in (
    ("everruns-capability", "everruns-core"),
    ("everruns-capability", "everruns"),
    ("everruns-core", "everruns-engine"),
    ("everruns-core", "everruns-host"),
    ("everruns-engine", "everruns-host"),
    ("everruns-mcp", "everruns-host"),
    ("everruns-platform", "everruns-host"),
    ("everruns-host", "everruns"),
    ("everruns-macros", "everruns"),
    ("everruns-durable", "everruns-scale"),
    ("everruns", "everruns-scale"),
):
    if earlier not in order:
        fail(f"publish-crates.yml does not publish {earlier}")
    elif later not in order:
        fail(f"publish-crates.yml does not publish {later}")
    elif order[earlier] > order[later]:
        fail(f"publish-crates.yml must publish {earlier} before {later}")

# The workflow's own version verification must cover the same edges.
for manifest_rel, dependency in (
    ("crates/core/Cargo.toml", "everruns-capability"),
    ("crates/everruns/Cargo.toml", "everruns-capability"),
    ("crates/engine/Cargo.toml", "everruns-core"),
    ("crates/host/Cargo.toml", "everruns-core"),
    ("crates/host/Cargo.toml", "everruns-engine"),
    ("crates/host/Cargo.toml", "everruns-mcp"),
    ("crates/host/Cargo.toml", "everruns-platform"),
    ("crates/everruns/Cargo.toml", "everruns-host"),
    ("crates/everruns/Cargo.toml", "everruns-macros"),
    ("crates/scale/Cargo.toml", "everruns"),
    ("crates/scale/Cargo.toml", "everruns-durable"),
):
    entry = re.search(
        rf'"{re.escape(manifest_rel)}":\s*\[(.*?)\]', workflow_text, re.DOTALL
    )
    if entry is None or f'"{dependency}"' not in entry.group(1):
        fail(
            f"publish-crates.yml version verification does not cover "
            f"{manifest_rel} dependency {dependency}"
        )

# The sync script keeps those pins at the workspace version between releases.
sync_text = (REPO / "scripts/sync-publish-pin-versions.py").read_text()
workspace_pins = re.search(r"WORKSPACE_PIN_DEPS:\s*list\[str\]\s*=\s*\[(.*?)\]", sync_text, re.DOTALL)
if workspace_pins is None:
    fail("could not find WORKSPACE_PIN_DEPS in scripts/sync-publish-pin-versions.py")
else:
    for dependency in ("everruns-capability", "everruns-engine", "everruns-macros"):
        if f'"{dependency}"' not in workspace_pins.group(1):
            fail(f"scripts/sync-publish-pin-versions.py does not sync the {dependency} workspace pin")

if failures:
    for failure in failures:
        print(f"FAIL: {failure}", file=sys.stderr)
    raise SystemExit(1)

print(f"publish order verified for {len(published)} crate(s)")
PY

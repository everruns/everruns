#!/usr/bin/env bash
# EVE-871 removal guard: `everruns-runtime` is deleted in 0.18.
#
# The 0.17.x compatibility crate carried no logic — ordinary applications use
# `everruns` and advanced execution hosts use `everruns-host` plus focused
# sibling crates. This test fails the build if the package, a dependency on it,
# a Rust import, a publish entry, or a current documentation recommendation
# comes back.
#
# Two historical records are allowed to name it: CHANGELOG.md carries the
# breaking change and the 0.17 migration mapping, and knowledge/log.md records
# the dated knowledge update.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

failures=0

fail() {
  echo "$1" >&2
  printf '  %s\n' "${@:2}" >&2
  failures=$((failures + 1))
}

# 1. The crate directory must stay deleted.
if [ -e "crates/runtime" ]; then
  fail "crates/runtime must not exist; everruns-runtime was removed in 0.18." "crates/runtime"
fi

# 2. No manifest may declare the package or depend on it (including renamed
#    dependencies that resolve to the `everruns-runtime` package).
manifest_violations="$(
  python3 - "$PROJECT_ROOT" <<'PY'
import os
import pathlib
import sys
import tomllib

root = pathlib.Path(sys.argv[1]).resolve()
violations = []


def dependency_tables(value, path=()):
    if not isinstance(value, dict):
        return
    for key, child in value.items():
        child_path = path + (key,)
        if key in {"dependencies", "dev-dependencies", "build-dependencies"}:
            yield child_path, child
        yield from dependency_tables(child, child_path)


PRUNE = {"target", "node_modules", ".git"}


def manifests(base):
    """Walk for Cargo.toml files, pruning build and dependency trees."""
    for directory, subdirectories, files in os.walk(base):
        subdirectories[:] = [name for name in subdirectories if name not in PRUNE]
        if "Cargo.toml" in files:
            yield pathlib.Path(directory) / "Cargo.toml"


for manifest in manifests(root):
    relative = manifest.relative_to(root).as_posix()
    document = tomllib.loads(manifest.read_text())

    package = document.get("package")
    if isinstance(package, dict) and package.get("name") == "everruns-runtime":
        violations.append(f"{relative}: package.name")

    members = document.get("workspace", {}).get("members", [])
    for member in members if isinstance(members, list) else []:
        if isinstance(member, str) and member.rstrip("/").endswith("crates/runtime"):
            violations.append(f"{relative}: workspace.members contains {member}")

    for table_path, dependencies in dependency_tables(document):
        if not isinstance(dependencies, dict):
            continue
        for name, declaration in dependencies.items():
            renamed = (
                declaration.get("package") if isinstance(declaration, dict) else None
            )
            if name == "everruns-runtime" or renamed == "everruns-runtime":
                violations.append(f"{relative}: {'.'.join(table_path)}.{name}")

print("\n".join(sorted(violations)))
PY
)"
if [ -n "$manifest_violations" ]; then
  # shellcheck disable=SC2086
  fail "manifests must not declare or depend on everruns-runtime:" $manifest_violations
fi

# 3. The resolved workspace graph must not contain the package. `--no-deps`
#    keeps this offline and fast; a first-party package or dependency edge is
#    the only way everruns-runtime could re-enter the graph.
metadata_violations="$(
  python3 - "$PROJECT_ROOT" <<'PY'
import json
import pathlib
import subprocess
import sys

root = pathlib.Path(sys.argv[1]).resolve()
metadata = json.loads(
    subprocess.check_output(
        [
            "cargo",
            "metadata",
            "--no-deps",
            "--offline",
            "--format-version",
            "1",
            "--manifest-path",
            str(root / "Cargo.toml"),
        ],
        cwd=root,
        text=True,
    )
)
violations = []
for package in metadata["packages"]:
    if package["name"] == "everruns-runtime":
        violations.append(f"package {package['name']} {package['version']}")
    for dependency in package["dependencies"]:
        if dependency["name"] == "everruns-runtime":
            violations.append(f"{package['name']} depends on everruns-runtime")
print("\n".join(sorted(set(violations))))
PY
)"
if [ -n "$metadata_violations" ]; then
  # shellcheck disable=SC2086
  fail "cargo metadata still resolves everruns-runtime:" $metadata_violations
fi

# 3b. The committed lockfile must not carry a resolved package entry either.
if rg -q '^name = "everruns-runtime"$' Cargo.lock; then
  fail "Cargo.lock still resolves everruns-runtime." "Cargo.lock"
fi

# 4. No Rust source may name the crate.
source_violations="$(
  rg -n --glob '*.rs' --glob '!target/**' 'everruns[_-]runtime' . </dev/null || true
)"
if [ -n "$source_violations" ]; then
  echo "Rust sources must not reference everruns-runtime:" >&2
  echo "$source_violations" >&2
  failures=$((failures + 1))
fi

# 5. No current documentation, skill, or workflow may recommend or build it.
#    CHANGELOG.md and knowledge/log.md are excluded as historical records, and
#    naming this guard script itself is always allowed.
doc_violations="$(
  rg -n \
    --glob '!target/**' \
    --glob '!node_modules/**' \
    --glob '!Cargo.lock' \
    --glob '!CHANGELOG.md' \
    --glob '!knowledge/log.md' \
    'everruns[_-]runtime' \
    README.md CONTRIBUTING.md docs knowledge skills crates integrations examples \
    justfile scripts .github </dev/null 2>/dev/null \
    | grep -v 'test-no-everruns-runtime' || true
)"
if [ -n "$doc_violations" ]; then
  echo "docs, skills, and workflows must not reference everruns-runtime:" >&2
  echo "$doc_violations" >&2
  failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
  exit 1
fi

echo "No everruns-runtime package, dependency, import, publish entry, or recommendation remains."

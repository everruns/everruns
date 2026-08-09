#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Compatibility ownership is explicit: the workspace declaration keeps the
# 0.17 adapter publishable, and only the adapter crate may import its own Rust
# name. The coding CLI fixture is a source guard whose string is the assertion
# needle, not a dependency.
SOURCE_PREFIX_ALLOWLIST=(
  "crates/runtime/"
)
SOURCE_FILE_ALLOWLIST=(
  "examples/coding-cli/tests/facade_only.rs"
)

python3 - "$PROJECT_ROOT" <<'PY'
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

for manifest in root.rglob("Cargo.toml"):
    relative = manifest.relative_to(root).as_posix()
    if "target" in manifest.relative_to(root).parts:
        continue
    document = tomllib.loads(manifest.read_text())
    for table_path, dependencies in dependency_tables(document):
        if not isinstance(dependencies, dict):
            continue
        for name, declaration in dependencies.items():
            package = declaration.get("package") if isinstance(declaration, dict) else None
            if name == "everruns-runtime" or package == "everruns-runtime":
                if relative == "Cargo.toml" and table_path == ("workspace", "dependencies"):
                    continue
                violations.append(f"{relative}: {'.'.join(table_path)}.{name}")

if violations:
    print("direct everruns-runtime dependency declarations are forbidden:", file=sys.stderr)
    for violation in violations:
        print(f"  {violation}", file=sys.stderr)
    raise SystemExit(1)
PY

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
    manifest = pathlib.Path(package["manifest_path"]).resolve()
    if root not in manifest.parents:
        continue
    if package["name"] == "everruns-runtime":
        continue
    for dependency in package["dependencies"]:
        if dependency["name"] == "everruns-runtime":
            violations.append(f"{package['name']} ({manifest.relative_to(root)})")

if violations:
    print("workspace packages directly depending on everruns-runtime:", file=sys.stderr)
    for violation in violations:
        print(f"  {violation}", file=sys.stderr)
    raise SystemExit(1)
PY

violations=()
while IFS= read -r match; do
  [ -n "$match" ] || continue
  path="${match%%:*}"
  allowed=false
  for prefix in "${SOURCE_PREFIX_ALLOWLIST[@]}"; do
    if [[ "$path" == "$prefix"* ]]; then
      allowed=true
      break
    fi
  done
  if [ "$allowed" = false ]; then
    for file in "${SOURCE_FILE_ALLOWLIST[@]}"; do
      if [ "$path" = "$file" ]; then
        allowed=true
        break
      fi
    done
  fi
  if [ "$allowed" = false ]; then
    violations+=("$match")
  fi
done < <(
  cd "$PROJECT_ROOT"
  rg -n --glob '*.rs' --glob '!target/**' 'everruns_runtime' || true
)

if [ "${#violations[@]}" -ne 0 ]; then
  echo "everruns_runtime imports/references are forbidden outside the compatibility allowlist:" >&2
  printf '  %s\n' "${violations[@]}" >&2
  exit 1
fi

echo "No first-party consumer directly depends on or imports everruns-runtime."

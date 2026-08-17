#!/usr/bin/env bash
# Validate the structural documentation contract for every crates.io package.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

python3 - <<'PY'
from __future__ import annotations

import pathlib
import json
import re
import subprocess
import sys
import tomllib
from urllib.parse import urlparse

root = pathlib.Path.cwd()
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--no-deps", "--format-version", "1"], text=True
))
published = sorted(
    package["name"] for package in metadata["packages"] if package.get("publish") != []
)

manifests: dict[str, tuple[pathlib.Path, dict]] = {}
for base in (root / "crates", root / "integrations"):
    for manifest in base.glob("*/Cargo.toml"):
        data = tomllib.loads(manifest.read_text())
        name = data.get("package", {}).get("name")
        if name:
            manifests[name] = (manifest, data)

errors: list[str] = []


def require(condition: bool, location: pathlib.Path, message: str) -> None:
    if not condition:
        errors.append(f"{location.relative_to(root)}: {message}")


def route_exists(route: str) -> bool:
    route = route.strip("/")
    if not route:
        return (root / "docs/index.mdx").is_file() or (root / "docs/index.md").is_file()
    if route == "api":
        return True  # Generated from docs/api/openapi.json by Starlight OpenAPI.
    base = root / "docs" / route
    return any(
        candidate.is_file()
        for candidate in (
            base.with_suffix(".md"),
            base.with_suffix(".mdx"),
            base / "index.md",
            base / "index.mdx",
        )
    )


for package in published:
    if package not in manifests:
        errors.append(f"Cargo metadata: no workspace manifest found for {package}")
        continue

    manifest, data = manifests[package]
    crate_dir = manifest.parent
    package_data = data["package"]
    readme = crate_dir / "README.md"
    lib = crate_dir / "src/lib.rs"
    has_library = lib.is_file()

    require(package_data.get("readme") == "README.md", manifest, 'set readme = "README.md"')
    require(readme.is_file(), readme, "published crate README is missing")
    if not readme.is_file():
        continue

    readme_text = readme.read_text()
    rustdoc = lib.read_text() if has_library else ""
    crate_doc_lines = []
    for line in rustdoc.splitlines():
        if line.startswith("//!"):
            crate_doc_lines.append(line)
        elif crate_doc_lines:
            break
    crate_docs = "\n".join(crate_doc_lines)

    require(
        re.search(rf"^# {re.escape(package)}\s*$", readme_text, re.M) is not None,
        readme,
        f"start with '# {package}'",
    )
    require(
        re.search(rf"^# {re.escape(package)}\n\n> \S.+$", readme_text, re.M) is not None,
        readme,
        "place a one-line blockquote tagline directly below the title",
    )
    require("https://everruns.com" in readme_text, readme, "add the Everruns ecosystem link")
    require(
        "```rust" in readme_text if has_library else "```" in readme_text,
        readme,
        "add a quick example",
    )
    require(
        re.search(r"^## (What It Provides|Features)\s*$", readme_text, re.M | re.I) is not None,
        readme,
        "add a What It Provides or Features section",
    )
    require(re.search(r"^## Documentation\s*$", readme_text, re.M | re.I) is not None, readme, "add a Documentation section")
    require("https://docs.everruns.com" in readme_text, readme, "link relevant public documentation")
    if has_library:
        require("https://docs.rs" in readme_text, readme, "link the docs.rs API reference")
    require(re.search(r"^## License\s*$", readme_text, re.M | re.I) is not None, readme, "add a License section")
    require("github.com/everruns/everruns/blob/main/LICENSE" in readme_text, readme, "link the repository MIT license")

    # Crate-level lint attributes may precede the inner rustdoc. Strip only
    # single-line inner attributes; executable items or ordinary comments must
    # still fail the structural contract.
    if has_library:
        rustdoc_start = re.sub(r"\A(?:#!\[[^\n]*\]\s*)*", "", rustdoc)
        require(rustdoc_start.startswith("//!"), lib, "start with crate-level rustdoc")
        require("https://everruns.com" in crate_docs, lib, "mirror the Everruns ecosystem link in rustdoc")
        require(
            re.search(r"^//! ```(?:rust|no_run)?\s*$", crate_docs, re.M) is not None,
            lib,
            "add a compiled crate-level rustdoc example",
        )

    surfaces = [(readme, readme_text)]
    if has_library:
        surfaces.append((lib, crate_docs))
    for location, text in surfaces:
        require("knowledge/" not in text, location, "do not link internal knowledge from published docs")
        require(re.search(r"^//!?\s*Decision:", text, re.M) is None, location, "remove internal Decision: notes from published docs")
        urls = re.findall(r"https://docs\.everruns\.com(?:/[^\s)\]}>`\"]*)?", text)
        for url in urls:
            parsed = urlparse(url.rstrip(".,;:"))
            if not route_exists(parsed.path):
                errors.append(
                    f"{location.relative_to(root)}: docs link has no local route: {url}"
                )

if errors:
    print("Published crate documentation check failed:", file=sys.stderr)
    for error in errors:
        print(f"  - {error}", file=sys.stderr)
    raise SystemExit(1)

print(f"Published crate documentation OK ({len(published)} packages).")
PY

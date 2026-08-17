#!/usr/bin/env python3
"""Validate or update internal dependency pins for independently versioned crates.

Published workspace packages own explicit versions. Every path dependency from
a published package must carry the current version of the package it targets;
workspace-inherited dependencies obtain that pin from ``Cargo.toml``.

The package graph is discovered from Cargo metadata. There are no package or
dependency allowlists to update when crates move or versions diverge.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tomllib
from collections.abc import Iterator
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parent.parent


def load(path: Path) -> dict[str, Any]:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def metadata() -> dict[str, Any]:
    output = subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=REPO,
        text=True,
    )
    return json.loads(output)


def dependency_tables(manifest: dict[str, Any]) -> Iterator[dict[str, Any]]:
    for name in ("dependencies", "build-dependencies", "dev-dependencies"):
        table = manifest.get(name)
        if isinstance(table, dict):
            yield table
    for target in manifest.get("target", {}).values():
        if not isinstance(target, dict):
            continue
        for name in ("dependencies", "build-dependencies", "dev-dependencies"):
            table = target.get(name)
            if isinstance(table, dict):
                yield table


def target_manifest(owner: Path, dependency: dict[str, Any]) -> Path | None:
    path = dependency.get("path")
    if not isinstance(path, str):
        return None
    candidate = (owner.parent / path).resolve()
    return candidate if candidate.name == "Cargo.toml" else candidate / "Cargo.toml"


def rewrite_inline_dependency(path: Path, key: str, version: str) -> bool:
    text = path.read_text()
    pattern = re.compile(
        rf'^(?P<prefix>\s*{re.escape(key)}\s*=\s*\{{)(?P<body>[^}}\n]*)(?P<suffix>\}}.*)$',
        re.MULTILINE,
    )
    match = pattern.search(text)
    if match is None:
        raise ValueError(f"{path.relative_to(REPO)}: cannot rewrite non-inline dependency {key}")
    body = match.group("body")
    if re.search(r'\bversion\s*=\s*"[^"]+"', body):
        new_body = re.sub(
            r'(\bversion\s*=\s*")[^"]+(" )?',
            lambda item: f'{item.group(1)}{version}{item.group(2) or ""}',
            body,
            count=1,
        )
    else:
        new_body = f' version = "{version}", {body.lstrip()}'
    new_text = text[: match.start("body")] + new_body + text[match.end("body") :]
    if new_text == text:
        return False
    path.write_text(new_text)
    return True


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="report drift without modifying files")
    mode.add_argument("--write", action="store_true", help="rewrite drifted pins (default)")
    args = parser.parse_args()
    check_only = args.check

    cargo = metadata()
    packages = {Path(package["manifest_path"]).resolve(): package for package in cargo["packages"]}
    published = {
        manifest: package
        for manifest, package in packages.items()
        if package.get("publish") != []
    }
    root_path = REPO / "Cargo.toml"
    root = load(root_path)
    workspace_dependencies = root["workspace"].get("dependencies", {})
    failures: list[str] = []
    rewrites: dict[tuple[Path, str], str] = {}

    for manifest_path, package in sorted(published.items(), key=lambda item: item[1]["name"]):
        manifest = load(manifest_path)
        if not isinstance(manifest.get("package", {}).get("version"), str):
            failures.append(
                f"{manifest_path.relative_to(REPO)}: published package must declare an explicit version"
            )

        for table in dependency_tables(manifest):
            for key, declaration in table.items():
                if not isinstance(declaration, dict):
                    continue
                resolved = declaration
                declaration_path = manifest_path
                if declaration.get("workspace") is True:
                    resolved = workspace_dependencies.get(key, {})
                    declaration_path = root_path
                if not isinstance(resolved, dict):
                    continue
                dependency_manifest = target_manifest(declaration_path, resolved)
                if dependency_manifest is None or dependency_manifest not in published:
                    continue
                expected = published[dependency_manifest]["version"]
                actual = resolved.get("version")
                if actual != expected:
                    failures.append(
                        f"{declaration_path.relative_to(REPO)}: {key} pins {actual!r}; "
                        f"{published[dependency_manifest]['name']} is {expected}"
                    )
                    rewrites[(declaration_path, key)] = expected

    if check_only:
        if failures:
            print("independent crate version validation failed:", file=sys.stderr)
            for failure in failures:
                print(f"  {failure}", file=sys.stderr)
            return 1
        print(f"independent crate versions and pins verified for {len(published)} package(s)")
        return 0

    changed = 0
    for (path, key), version in sorted(rewrites.items(), key=lambda item: (str(item[0][0]), item[0][1])):
        try:
            changed += rewrite_inline_dependency(path, key, version)
        except ValueError as error:
            print(error, file=sys.stderr)
            return 1
    print(f"updated {changed} internal dependency pin(s)")
    return subprocess.run([sys.executable, __file__, "--check"], cwd=REPO, check=False).returncode


if __name__ == "__main__":
    sys.exit(main())

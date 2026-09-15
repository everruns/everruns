#!/usr/bin/env python3
"""Validate or update internal dependency pins for independently versioned crates.

Published workspace packages own explicit versions. Every path dependency from
a published package must carry the current version of the package it targets;
workspace-inherited dependencies obtain that pin from ``Cargo.toml``. A
dev-dependency counts only once it declares a version of its own (see
``dependency_tables``).

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


# Dev-dependencies are pinned only where the declaration already carries a
# version of its own. A version-less path dev-dependency never reaches
# downstream consumers (`cargo publish` drops it entirely), and crate-release.yml
# ignores dev edges in both its publish ordering and its strand check. Adding a
# version there would only create a publish-order deadlock: a crate that
# dev-depends on a sibling bumped in the same cycle could not package before that
# sibling publishes, while the sibling may in turn depend on it (host <-> llmsim).
# A dev-dependency that spells out a version has opted into that ordering
# deliberately - usually because it also carries features the workspace entry
# does not (ard dev-depends on host with `direct-egress`), so it cannot inherit
# the pin from `[workspace.dependencies]`. Such a pin still has to track the
# target package, or a breaking bump leaves the published crate requesting a
# version that no longer exists. Workspace-inherited dev edges stay excluded:
# they declare no version, so keeping them out preserves the deadlock-free
# default.
DEPENDENCY_KINDS = ("dependencies", "build-dependencies", "dev-dependencies")


def dependency_tables(manifest: dict[str, Any]) -> Iterator[tuple[str, dict[str, Any]]]:
    for kind in DEPENDENCY_KINDS:
        table = manifest.get(kind)
        if isinstance(table, dict):
            yield kind, table
    for target in manifest.get("target", {}).values():
        if not isinstance(target, dict):
            continue
        for kind in DEPENDENCY_KINDS:
            table = target.get(kind)
            if isinstance(table, dict):
                yield kind, table


def target_manifest(owner: Path, dependency: dict[str, Any]) -> Path | None:
    path = dependency.get("path")
    if not isinstance(path, str):
        return None
    candidate = (owner.parent / path).resolve()
    return candidate if candidate.name == "Cargo.toml" else candidate / "Cargo.toml"


TABLE_HEADER = re.compile(r'^\[(?P<name>[^\[\]]+)\]\s*$', re.MULTILINE)


# A manifest can declare the same package in several dependency kinds (a normal
# dependency and a dev-dependency with extra features, say). Rewrites therefore
# search only the tables of the kind the drift was found in - `[dependencies]`,
# `[dev-dependencies]`, `[workspace.dependencies]`, `[target.<cfg>.dependencies]`
# all end in the kind that owns them.
def dependency_table_spans(text: str, kind: str) -> Iterator[tuple[int, int]]:
    headers = list(TABLE_HEADER.finditer(text))
    for index, header in enumerate(headers):
        if header.group("name").strip().rsplit(".", 1)[-1] != kind:
            continue
        end = headers[index + 1].start() if index + 1 < len(headers) else len(text)
        yield header.end(), end


def rewrite_inline_dependency(path: Path, kind: str, key: str, version: str) -> bool:
    text = path.read_text()
    pattern = re.compile(
        rf'^(?P<prefix>\s*{re.escape(key)}\s*=\s*\{{)(?P<body>[^}}\n]*)(?P<suffix>\}}.*)$',
        re.MULTILINE,
    )
    matches = []
    for start, end in dependency_table_spans(text, kind):
        found = pattern.search(text[start:end])
        if found is not None:
            matches.append((start, found))
    if not matches:
        raise ValueError(
            f"{path.relative_to(REPO)}: cannot rewrite non-inline {kind} entry {key}"
        )
    if len(matches) > 1:
        raise ValueError(
            f"{path.relative_to(REPO)}: {key} is declared in several {kind} tables; "
            "pin it by hand"
        )
    offset, match = matches[0]
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
    body_start = offset + match.start("body")
    body_end = offset + match.end("body")
    new_text = text[:body_start] + new_body + text[body_end:]
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
    rewrites: dict[tuple[Path, str, str], str] = {}

    for manifest_path, package in sorted(published.items(), key=lambda item: item[1]["name"]):
        manifest = load(manifest_path)
        if not isinstance(manifest.get("package", {}).get("version"), str):
            failures.append(
                f"{manifest_path.relative_to(REPO)}: published package must declare an explicit version"
            )

        for kind, table in dependency_tables(manifest):
            for key, declaration in table.items():
                if not isinstance(declaration, dict):
                    continue
                # Only a dev-dependency that declares its own version is in
                # scope; workspace-inherited dev edges declare none and stay out.
                if kind == "dev-dependencies" and "version" not in declaration:
                    continue
                resolved = declaration
                declaration_path = manifest_path
                declaration_kind = kind
                if declaration.get("workspace") is True:
                    resolved = workspace_dependencies.get(key, {})
                    declaration_path = root_path
                    declaration_kind = "dependencies"
                if not isinstance(resolved, dict):
                    continue
                dependency_manifest = target_manifest(declaration_path, resolved)
                if dependency_manifest is None or dependency_manifest not in published:
                    continue
                expected = published[dependency_manifest]["version"]
                actual = resolved.get("version")
                if actual != expected:
                    failures.append(
                        f"{declaration_path.relative_to(REPO)} [{declaration_kind}]: "
                        f"{key} pins {actual!r}; "
                        f"{published[dependency_manifest]['name']} is {expected}"
                    )
                    rewrites[(declaration_path, declaration_kind, key)] = expected

    if check_only:
        if failures:
            print("independent crate version validation failed:", file=sys.stderr)
            for failure in failures:
                print(f"  {failure}", file=sys.stderr)
            return 1
        print(f"independent crate versions and pins verified for {len(published)} package(s)")
        return 0

    changed = 0
    for (path, kind, key), version in sorted(rewrites.items(), key=lambda item: tuple(map(str, item[0]))):
        try:
            changed += rewrite_inline_dependency(path, kind, key, version)
        except ValueError as error:
            print(error, file=sys.stderr)
            return 1
    print(f"updated {changed} internal dependency pin(s)")
    return subprocess.run([sys.executable, __file__, "--check"], cwd=REPO, check=False).returncode


if __name__ == "__main__":
    sys.exit(main())

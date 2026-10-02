#!/usr/bin/env python3
"""Check the crates.io publish order against the manifests cargo actually packages.

Crate Release publishes one crate at a time, and `cargo package` resolves every
dependency left in a packaged manifest from the crates.io index. So each
workspace crate a packaged manifest still names has to be published earlier in
the cascade, whatever its kind. A version-less path dev-dependency is stripped
at packaging and does not count; one that carries a version (explicit, or
inherited with `workspace = true`) stays and does.

The plan reasons about those rules from `cargo metadata`. This check does not:
it packages every crate in the plan, reads each normalized Cargo.toml out of
the `.crate`, and fails if any surviving workspace dependency is published at
or after its dependant. v0.34.0 halted after two crates because the plan
ordered everruns-durable ahead of everruns-core, which it dev-depends on; this
check fails on that plan.

Usage:
  check-publish-order.py                     # plan from crate-release.yml, nothing published yet
  check-publish-order.py --plan plan.json    # a plan Crate Release produced

Packaging uses `--no-verify --exclude-lockfile`, so it builds nothing and takes
a few seconds for the whole publish set.
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import os
import re
import subprocess
import sys
import tarfile
import textwrap
import tomllib
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parent.parent
WORKFLOW = REPO / ".github/workflows/crate-release.yml"
DEPENDENCY_KINDS = ("dependencies", "build-dependencies", "dev-dependencies")


def metadata() -> dict[str, Any]:
    return json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=REPO, text=True
        )
    )


def plan_from_workflow() -> list[str]:
    """Run crate-release.yml's own plan script as if no crate were published yet.

    That is a fresh platform version: every published crate is in the plan, so
    the order covers every edge. The index is stubbed rather than queried, so
    the check is hermetic and does not depend on what crates.io holds today.
    """
    text = WORKFLOW.read_text()
    match = re.search(r"python3 - <<'PY' > release-plan\.json\n(.*?)\n\s*PY\n", text, re.S)
    if match is None:
        raise SystemExit(f"cannot find the release plan script in {WORKFLOW.relative_to(REPO)}")
    source = textwrap.dedent(match.group(1))

    def never_published(url, timeout=None):
        raise urllib.error.HTTPError(url, 404, "not published", None, None)

    original_urlopen = urllib.request.urlopen
    original_sha = os.environ.pop("GITHUB_SHA", None)
    urllib.request.urlopen = never_published
    captured = io.StringIO()
    cwd = os.getcwd()
    try:
        os.chdir(REPO)
        with contextlib.redirect_stdout(captured):
            exec(compile(source, "release-plan", "exec"), {"__name__": "__main__"})
    finally:
        os.chdir(cwd)
        urllib.request.urlopen = original_urlopen
        if original_sha is not None:
            os.environ["GITHUB_SHA"] = original_sha
    return [entry["package"] for entry in json.loads(captured.getvalue())]


def packaged_manifests(packages: list[str], versions: dict[str, str]) -> dict[str, dict[str, Any]]:
    args = ["cargo", "package", "--no-verify", "--exclude-lockfile", "--quiet"]
    for name in packages:
        args += ["-p", name]
    subprocess.run(args, cwd=REPO, check=True)
    target = Path(
        json.loads(
            subprocess.check_output(
                ["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=REPO, text=True
            )
        )["target_directory"]
    )
    manifests: dict[str, dict[str, Any]] = {}
    for name in packages:
        stem = f"{name}-{versions[name]}"
        with tarfile.open(target / "package" / f"{stem}.crate") as archive:
            member = archive.extractfile(f"{stem}/Cargo.toml")
            if member is None:
                raise SystemExit(f"{stem}.crate has no Cargo.toml")
            manifests[name] = tomllib.loads(member.read().decode())
    return manifests


def surviving_dependencies(manifest: dict[str, Any]) -> set[str]:
    """Package names a packaged manifest still depends on, across every kind and target."""
    tables: list[dict[str, Any]] = []
    for kind in DEPENDENCY_KINDS:
        tables.append(manifest.get(kind, {}))
    for target in manifest.get("target", {}).values():
        for kind in DEPENDENCY_KINDS:
            tables.append(target.get(kind, {}))
    names: set[str] = set()
    for table in tables:
        for key, declaration in table.items():
            package = declaration.get("package", key) if isinstance(declaration, dict) else key
            names.add(package)
    return names


def order_failures(plan: list[str], dependencies: dict[str, set[str]]) -> list[str]:
    position = {name: index for index, name in enumerate(plan)}
    failures: list[str] = []
    for name in plan:
        for dependency in sorted(dependencies[name] & position.keys()):
            if dependency != name and position[dependency] > position[name]:
                failures.append(
                    f"{name} (#{position[name] + 1}) packages a dependency on {dependency}, "
                    f"which publishes later (#{position[dependency] + 1})"
                )
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--plan", type=Path, help="release-plan.json written by Crate Release")
    args = parser.parse_args()

    if args.plan is not None:
        plan = [entry["package"] for entry in json.loads(args.plan.read_text())]
    else:
        plan = plan_from_workflow()
    if not plan:
        print("publish order: nothing to publish")
        return 0

    versions = {package["name"]: package["version"] for package in metadata()["packages"]}
    manifests = packaged_manifests(plan, versions)
    failures = order_failures(plan, {name: surviving_dependencies(m) for name, m in manifests.items()})
    if failures:
        print("publish order validation failed:", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    print(f"publish order verified against packaged manifests for {len(plan)} crate(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Catch release-numbering and publish-set mistakes before they burn a version.

On 2026-10-02 the platform needed three version numbers (0.34.0, 0.34.1,
0.34.2) to ship one release, because each crates.io cascade failed part-way
and a partly published version can only be finished by the next patch. The
publish-order check (check-publish-order.py) and the up-front tagging in
crate-release.yml fix the two failures seen that day. This script covers the
mistakes around them that nothing checked:

  offline (every PR, CI Lockfile job)
    - The publish set matches .github/crates-publish-set.txt, so a crate joining
      or leaving crates.io is a reviewed one-line edit, never a side effect of
      dropping `publish = false`. A new name needs crates.io permission and a
      slot in the new-crate rate limit, so it must be a deliberate act.
    - apps/ui/package.json and the newest CHANGELOG.md section carry the
      workspace version.

  --registry (release PRs, before merge)
    - Everything offline checks.
    - The new version is higher than every version any crate in the publish set
      already has on crates.io. A version some crate already holds is either a
      re-release or a burnt number, and its cascade cannot complete.
    - Every never-published name is free on crates.io, and the release does not
      create more new names than crates.io's new-crate burst allows.

  --names (Crate Release, before the first tag)
    - Only the name check. By then the version is on main, so "already
      published" is a normal re-run, not an error.

  --audit (daily, and `just release-status`)
    - Every crate in the publish set has the workspace version on crates.io.
      Reports the crates a halted or held-back cascade left behind, which
      otherwise nothing on main surfaces.
    - Every crate has its crate/<name>/v<version> release tag, and all of them
      point at one commit. 0.34.1 was on crates.io for most crates but had been
      published from three commits; a version check alone called that complete.

Registry reads use the public sparse index and API; no token is needed, so the
check cannot confirm the publish token may create a new name. It prints the
names so the person cutting the release confirms that before merging.

RELEASE_PREFLIGHT_REGISTRY_FIXTURE points at a JSON file standing in for
crates.io ({"crates": {name: {"versions": [...], "owners": [...]}}}), which
scripts/test-release-preflight.sh uses to stay hermetic.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tomllib
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

REPO = Path(os.environ.get("RELEASE_PREFLIGHT_REPO", Path(__file__).resolve().parent.parent))
PUBLISH_SET = ".github/crates-publish-set.txt"
# Owners of this crate are the reference for "a name we own". Every crate in
# the publish set is published by the same account.
OWNER_REFERENCE_CRATE = "everruns-core"
# crates.io lets an account create 5 new crates in a burst, then one every 10
# minutes. A cascade spaces publishes by a minute or two, so a release that
# introduces more names than the burst stalls on rate-limit errors part-way.
NEW_CRATE_BURST = 5
USER_AGENT = "everruns-release-preflight (https://github.com/everruns/everruns)"


class Failures:
    def __init__(self) -> None:
        self.items: list[str] = []

    def add(self, message: str) -> None:
        self.items.append(message)


# ---------------------------------------------------------------- workspace


def workspace_version() -> str:
    manifest = tomllib.loads((REPO / "Cargo.toml").read_text())
    return manifest["workspace"]["package"]["version"]


def publishable_crates() -> dict[str, str]:
    """Name -> resolved version for every package without `publish = false`."""
    meta = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=REPO, text=True
        )
    )
    return {p["name"]: p["version"] for p in meta["packages"] if p.get("publish") != []}


def read_publish_set() -> list[str]:
    path = REPO / PUBLISH_SET
    if not path.exists():
        return []
    names = []
    for line in path.read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            names.append(line)
    return names


def check_publish_set(crates: dict[str, str], failures: Failures) -> None:
    listed = read_publish_set()
    if not listed:
        failures.add(f"{PUBLISH_SET} is missing or empty")
        return
    if listed != sorted(set(listed)):
        failures.add(f"{PUBLISH_SET} must be sorted with no duplicates")
    joined = sorted(set(crates) - set(listed))
    left = sorted(set(listed) - set(crates))
    for name in joined:
        failures.add(
            f"{name} is publishable but not in {PUBLISH_SET}. Adding a crate to crates.io "
            f"is deliberate: confirm the name is free and the CARGO_REGISTRY_TOKEN may create "
            f"it, then add it to the list. Otherwise set `publish = false`."
        )
    for name in left:
        failures.add(
            f"{name} is in {PUBLISH_SET} but is not a publishable workspace package. "
            f"Remove it from the list and record it as retired in the next CHANGELOG "
            f"Crate Releases section."
        )


def check_version_files(version: str, failures: Failures) -> None:
    ui = json.loads((REPO / "apps/ui/package.json").read_text())["version"]
    if ui != version:
        failures.add(f"apps/ui/package.json is {ui}, workspace is {version}")
    changelog = (REPO / "CHANGELOG.md").read_text()
    match = re.search(r"^## \[(\d[^\]]*)\]", changelog, re.M)
    if match is None:
        failures.add("CHANGELOG.md has no released version section")
    elif match.group(1) != version:
        failures.add(
            f"newest CHANGELOG.md section is {match.group(1)}, workspace is {version}"
        )


# ---------------------------------------------------------------- versions


def version_key(version: str) -> tuple:
    """Semver precedence: a pre-release sorts below its release."""
    core, _, pre = version.partition("+")[0].partition("-")
    numbers = tuple(int(part) for part in core.split("."))
    if not pre:
        return numbers + ((1,),)
    ids = tuple((0, int(x), "") if x.isdigit() else (1, 0, x) for x in pre.split("."))
    return numbers + ((0,) + ids,)


# ---------------------------------------------------------------- registry


class Registry:
    def __init__(self) -> None:
        fixture = os.environ.get("RELEASE_PREFLIGHT_REGISTRY_FIXTURE")
        self.fixture = json.loads(Path(fixture).read_text())["crates"] if fixture else None
        self._versions: dict[str, list[str]] = {}

    @staticmethod
    def _index_path(name: str) -> str:
        n = name.lower()
        if len(n) <= 2:
            return f"{len(n)}/{n}"
        if len(n) == 3:
            return f"3/{n[0]}/{n}"
        return f"{n[0:2]}/{n[2:4]}/{n}"

    def _get(self, url: str) -> str | None:
        request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return response.read().decode()
        except urllib.error.HTTPError as error:
            if error.code == 404:
                return None
            raise

    def versions(self, name: str) -> list[str]:
        """Every version on crates.io, yanked included: a yanked number is still taken."""
        if name not in self._versions:
            if self.fixture is not None:
                self._versions[name] = list(self.fixture.get(name, {}).get("versions", []))
            else:
                body = self._get(f"https://index.crates.io/{self._index_path(name)}")
                self._versions[name] = (
                    [json.loads(line)["vers"] for line in body.splitlines() if line.strip()]
                    if body
                    else []
                )
        return self._versions[name]

    def owners(self, name: str) -> set[str]:
        if self.fixture is not None:
            return set(self.fixture.get(name, {}).get("owners", []))
        body = self._get(f"https://crates.io/api/v1/crates/{name}/owners")
        if body is None:
            return set()
        return {user["login"] for user in json.loads(body).get("users", [])}


def check_new_names(crates: dict[str, str], registry: Registry, failures: Failures) -> list[str]:
    """Never-published crates whose name is free; fails on a name someone else holds."""
    ours = registry.owners(OWNER_REFERENCE_CRATE)
    new: list[str] = []
    for name in sorted(crates):
        if registry.versions(name):
            if ours and not (registry.owners(name) & ours):
                failures.add(
                    f"{name} already exists on crates.io, owned by someone else. "
                    f"Rename the package (keep the lib name) before releasing."
                )
            continue
        new.append(name)
    if len(new) > NEW_CRATE_BURST:
        failures.add(
            f"{len(new)} crates would be created on crates.io in one release "
            f"({', '.join(new)}); crates.io allows a burst of {NEW_CRATE_BURST} and then "
            f"one new crate every 10 minutes, so the cascade would stall. Keep some "
            f"`publish = false` until the next release."
        )
    return new


def check_version_is_new(
    version: str, crates: dict[str, str], registry: Registry, failures: Failures
) -> None:
    held = sorted(name for name in crates if version in registry.versions(name))
    if held:
        failures.add(
            f"{version} is already on crates.io for {len(held)} crate(s) "
            f"({', '.join(held[:5])}{', ...' if len(held) > 5 else ''}). A version some crate "
            f"already holds cannot be finished from a new commit: use the next version."
        )
        return
    highest = max(
        (v for name in crates for v in registry.versions(name)), key=version_key, default=None
    )
    if highest is not None and version_key(version) <= version_key(highest):
        failures.add(
            f"{version} is not higher than {highest}, the highest version already on "
            f"crates.io in the publish set"
        )


def pending_first_release(
    version: str, crates: dict[str, str], registry: Registry, tags: dict[str, str]
) -> list[str]:
    """Never-published crates that joined the publish set after `version` was cut.

    Crate Release holds a new crate back once siblings are on crates.io from
    another commit, so it first ships with the next version. Without a
    crate/<name>/v<version> tag no cascade ever tried it at this version; that
    is the expected wait, not a halted cascade. A tagged one did fail to publish
    and stays in the audit.
    """
    return sorted(
        name
        for name in crates
        if not registry.versions(name) and f"crate/{name}/v{version}" not in tags
    )


def audit(version: str, crates: dict[str, str], registry: Registry) -> list[str]:
    missing = []
    for name in sorted(crates):
        versions = registry.versions(name)
        if version not in versions:
            have = max(versions, key=version_key) if versions else None
            missing.append(f"{name} (crates.io has {have})" if have else f"{name} (not on crates.io yet)")
    return missing


def release_tags(version: str) -> dict[str, str]:
    """crate/<name>/v<version> tag -> the commit it names, from the remote."""
    fixture = os.environ.get("RELEASE_PREFLIGHT_TAGS_FIXTURE")
    if fixture:
        return json.loads(Path(fixture).read_text())["tags"]
    out = subprocess.check_output(
        ["git", "ls-remote", "--tags", "origin", f"refs/tags/crate/*/v{version}*"],
        cwd=REPO,
        text=True,
    )
    tags: dict[str, str] = {}
    peeled: dict[str, str] = {}
    for line in out.splitlines():
        sha, ref = line.split("\t", 1)
        name = ref.removeprefix("refs/tags/")
        if name.endswith("^{}"):
            peeled[name[:-3]] = sha
        else:
            tags[name] = sha
    # An annotated tag lists its own object first; the peeled line is the commit.
    return {name: peeled.get(name, sha) for name, sha in tags.items()}


def audit_provenance(version: str, crates: dict[str, str], tags: dict[str, str]) -> list[str]:
    """A version is one release only if every crate was tagged at the same commit."""
    problems = []
    by_commit: dict[str, list[str]] = {}
    for name in sorted(crates):
        sha = tags.get(f"crate/{name}/v{version}")
        if sha is None:
            problems.append(f"{name} has no crate/{name}/v{version} release tag")
        else:
            by_commit.setdefault(sha, []).append(name)
    if len(by_commit) > 1:
        # The commit most crates came from is the release; list the others.
        release = max(by_commit, key=lambda sha: len(by_commit[sha]))
        for sha, names in sorted(by_commit.items()):
            if sha != release:
                problems.append(
                    f"{', '.join(names)} tagged at {sha[:9]}, not the release commit {release[:9]}"
                )
    return problems


# ---------------------------------------------------------------- main


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--registry", action="store_true", help="release PR: also check crates.io")
    mode.add_argument("--names", action="store_true", help="only check new crate names on crates.io")
    mode.add_argument("--audit", action="store_true", help="is the workspace version fully published?")
    args = parser.parse_args()

    version = workspace_version()
    crates = publishable_crates()
    failures = Failures()

    if args.audit:
        registry = Registry()
        tags = release_tags(version)
        pending = pending_first_release(version, crates, registry, tags)
        if pending:
            message = f"first crates.io publish waits for the next version: {', '.join(pending)}"
            print(f"::notice::{message}" if os.environ.get("GITHUB_ACTIONS") == "true" else message)
            crates = {name: v for name, v in crates.items() if name not in pending}
        missing = audit(version, crates, registry)
        if missing:
            print(
                f"{version} is not fully published: {len(missing)} of {len(crates)} crate(s) "
                f"missing on crates.io",
                file=sys.stderr,
            )
            for line in missing:
                print(f"  {line}", file=sys.stderr)
            return 1
        mixed = audit_provenance(version, crates, tags)
        if mixed:
            print(
                f"{version} is on crates.io but was not released from one commit. The "
                f"published crates cannot be changed: cut the next version, which republishes "
                f"every crate from one commit.",
                file=sys.stderr,
            )
            for line in mixed:
                print(f"  {line}", file=sys.stderr)
            return 1
        print(
            f"{version} is on crates.io for all {len(crates)} published crate(s), "
            f"all tagged at one commit"
        )
        return 0

    if not args.names:
        check_publish_set(crates, failures)
        check_version_files(version, failures)

    new: list[str] = []
    if args.registry or args.names:
        registry = Registry()
        new = check_new_names(crates, registry, failures)
        if args.registry:
            check_version_is_new(version, crates, registry, failures)

    if failures.items:
        print("release preflight failed:", file=sys.stderr)
        for failure in failures.items:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    scope = "crates.io" if (args.registry or args.names) else "offline"
    print(f"release preflight passed ({scope}): {len(crates)} published crate(s) at {version}")
    if new:
        # The index cannot say whether the publish token may create a name.
        # Surface it where the person merging the release will see it.
        print(
            f"  first publish for {len(new)} crate(s): {', '.join(new)}; the "
            f"CARGO_REGISTRY_TOKEN must be allowed to create them"
        )
        if os.environ.get("GITHUB_ACTIONS") == "true":
            print(f"::notice::first crates.io publish for: {', '.join(new)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

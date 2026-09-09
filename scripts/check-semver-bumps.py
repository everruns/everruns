#!/usr/bin/env python3
"""Deterministic guard against an under-bumped crate release.

`cargo-semver-checks` classifies a change breaking vs non-breaking against a
crate's latest crates.io release and fails a bump that is too small for it. The
release process already names it the authority for choosing a version, but
nothing enforced that, so a breaking change could ship in the patch slot: this
is how `everruns-host` 0.20.4 added a required `turn_id` argument to
`Runtime::append_accepted_inputs` while staying caret-compatible with every
already-published dependant's `^0.20.3` pin. Downstream consumers resolved the
new host into the old published `everruns` facade and hit a compile error in
code they do not own.

`check-publish-cone.py` cannot see that class of break. It compares version
*requirements*, and `^0.20.3` still admits 0.20.4 -- nothing looks stranded.
This gate compares *API*, which is the only thing that catches a breaking
change smuggled into a compatible range.

Scope is the crates this change is actually releasing: a published package
whose manifest version is not yet on crates.io. Ordinary PRs bump no versions
here (crate versions move only at release time), so they have nothing to check
and the gate is free; a release PR is checked at exactly the point the version
decision is made.

`--self-test` exercises the pure planning logic against fixtures with no
network access, so the gate's own correctness is guarded deterministically.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import urllib.error
import urllib.request


def index_path(name: str) -> str:
    n = name.lower()
    if len(n) == 1:
        return f"1/{n}"
    if len(n) == 2:
        return f"2/{n}"
    if len(n) == 3:
        return f"3/{n[0]}/{n}"
    return f"{n[0:2]}/{n[2:4]}/{n}"


def published_versions(name: str) -> set[str]:
    """Every version on the crates.io sparse index, yanked included.

    Yanked releases count: a version already burned on crates.io can never be
    published again, so it is not a version this change can be releasing.
    """
    try:
        body = urllib.request.urlopen(
            f"https://index.crates.io/{index_path(name)}", timeout=30
        ).read().decode()
    except urllib.error.HTTPError as exc:
        if exc.code == 404:
            return set()  # never published
        raise
    return {json.loads(line)["vers"] for line in body.splitlines() if line.strip()}


def workspace_versions() -> dict[str, str]:
    meta = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], text=True
        )
    )
    # Published packages are those whose manifest does not set publish = false
    # (cargo reports publish = [] for those).
    return {p["name"]: p["version"] for p in meta["packages"] if p.get("publish") != []}


def plan(
    current: dict[str, str], published: dict[str, set[str]]
) -> tuple[list[str], list[str]]:
    """Split published packages into (to check, skipped-with-reason).

    A package is checked when its manifest version is not on crates.io -- this
    change is releasing it, so its bump is a live decision. A first publish has
    no baseline to diff against and is skipped; so is a package whose version
    already exists upstream, because no version choice is being made for it.
    """
    check: list[str] = []
    skipped: list[str] = []
    for name in sorted(current):
        versions = published.get(name) or set()
        if not versions:
            skipped.append(f"{name} {current[name]}: first publish, no baseline")
        elif current[name] in versions:
            skipped.append(f"{name} {current[name]}: already published")
        else:
            check.append(name)
    return check, skipped


def run_semver_checks(packages: list[str]) -> int:
    """Run cargo-semver-checks over the release candidates in one invocation.

    The baseline is left to cargo-semver-checks (the crate's latest crates.io
    release), so a single call covers packages with different baselines. Its
    feature selection is left at the default heuristic too: pinning
    --all-features here would check feature combinations the crates never
    publish together.
    """
    # `cargo <missing-subcommand>` exits 101 like a real check failure, so probe
    # for the tool first rather than reporting a missing install as an
    # under-bump.
    probe = subprocess.run(
        ["cargo", "semver-checks", "--version"], capture_output=True, text=True
    )
    if probe.returncode != 0:
        print(
            "::error::cargo-semver-checks is not installed, so this change's "
            "crate bumps cannot be classified. Install it with "
            "`cargo install cargo-semver-checks --locked`."
        )
        return 1
    print(probe.stdout.strip())

    argv = ["cargo", "semver-checks", "check-release"]
    for name in packages:
        argv += ["--package", name]
    print(f"$ {' '.join(argv)}", flush=True)
    return subprocess.call(argv)


def run_check(plan_only: bool, candidates_only: bool) -> int:
    current = workspace_versions()
    published = {name: published_versions(name) for name in current}
    packages, skipped = plan(current, published)

    if candidates_only:
        # Bare package names on stdout, nothing else: CI decides whether to
        # install cargo-semver-checks at all by testing this for emptiness.
        for name in packages:
            print(name)
        return 0

    # The per-package reasons are long (one line per published crate) and only
    # matter when the plan itself looks wrong, so they stay behind --plan.
    print(f"{len(skipped)} published package(s) not being released here.")
    if plan_only:
        for note in skipped:
            print(f"  skip {note}")
    if not packages:
        print(
            f"No crate versions are being released across {len(current)} "
            "published packages; nothing to classify."
        )
        return 0

    print("Release candidates to classify against their latest crates.io release:")
    for name in packages:
        print(f"  - {name} {current[name]}")
    if plan_only:
        return 0

    code = run_semver_checks(packages)
    if code != 0:
        print(
            "::error::cargo-semver-checks did not pass. If it reports a "
            "required bump, take it -- for a 0.x crate the minor is the "
            "breaking slot (0.20.3 -> 0.21.0, not 0.20.4) -- then run "
            "`python3 scripts/sync-publish-pin-versions.py --write` and close "
            "the cone per knowledge/project/release-process.md. Shipping a "
            "breaking change as a patch leaves every published dependant's "
            "caret pin resolving into an API that no longer compiles, and a "
            "published version cannot be taken back. If it instead failed to "
            "run, check that its version understands this toolchain's rustdoc "
            "JSON format; the two are pinned together in ci.yml."
        )
    return code


def self_test() -> int:
    """Network-free checks of the pure planning logic."""
    failures: list[str] = []

    def expect(label: str, got, want) -> None:
        if got != want:
            failures.append(f"{label}: got {got!r}, want {want!r}")

    # The release that actually broke: host bumped (checked), the facade left at
    # its published version (nothing to classify), model-profiles never
    # published (no baseline to diff against).
    current = {
        "everruns-host": "0.20.4",
        "everruns": "0.19.1",
        "everruns-model-profiles": "0.1.0",
    }
    published = {
        "everruns-host": {"0.20.2", "0.20.3"},
        "everruns": {"0.19.0", "0.19.1"},
        "everruns-model-profiles": set(),
    }
    packages, skipped = plan(current, published)
    expect("bumped crate is checked", packages, ["everruns-host"])
    expect("unbumped crate is skipped", any("already published" in s for s in skipped), True)
    expect("first publish is skipped", any("no baseline" in s for s in skipped), True)

    # A yanked version is still burned upstream, so re-using it is not a release
    # this gate can classify.
    packages, _ = plan({"everruns": "0.19.1"}, {"everruns": {"0.19.1"}})
    expect("yanked-or-live published version is not a candidate", packages, [])

    # The facade republish this gate is meant to let through.
    packages, _ = plan({"everruns": "0.19.2"}, {"everruns": {"0.19.0", "0.19.1"}})
    expect("republish is a candidate", packages, ["everruns"])

    # A package absent from the index map behaves like one never published.
    packages, skipped = plan({"brand-new": "0.1.0"}, {})
    expect("unknown package is skipped", packages, [])
    expect("unknown package explains why", any("no baseline" in s for s in skipped), True)

    # Ordering is deterministic so CI output is diffable.
    packages, _ = plan(
        {"b-crate": "0.2.0", "a-crate": "0.2.0"},
        {"b-crate": {"0.1.0"}, "a-crate": {"0.1.0"}},
    )
    expect("candidates are sorted", packages, ["a-crate", "b-crate"])

    # Index path sharding must match crates.io, or every lookup 404s and the
    # gate silently passes by treating live crates as first publishes.
    expect("1-char shard", index_path("a"), "1/a")
    expect("2-char shard", index_path("ab"), "2/ab")
    expect("3-char shard", index_path("abc"), "3/a/abc")
    expect("4+-char shard", index_path("everruns-host"), "ev/er/everruns-host")

    for failure in failures:
        print(f"  FAIL {failure}")
    if failures:
        print(f"{len(failures)} self-test failure(s)")
        return 1
    print("semver-bump gate self-test passed")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--plan",
        action="store_true",
        help="list the release candidates without running cargo-semver-checks",
    )
    parser.add_argument(
        "--candidates",
        action="store_true",
        help="print only the release-candidate package names, one per line",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run the network-free checks of the planning logic",
    )
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    return run_check(plan_only=args.plan, candidates_only=args.candidates)


if __name__ == "__main__":
    sys.exit(main())

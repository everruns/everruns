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

When a candidate already advances the breaking slot, no API classification can
require a larger bump. The gate skips cargo-semver-checks for that candidate;
this also keeps an obsolete baseline that no longer compiles from blocking an
already-correct breaking release.

Candidates that still need classification use the exact crate release tag when
it exists. The tag carries the historical workspace and lockfile, unlike
cargo-semver-checks' registry placeholder, whose dependency resolution can
drift after a publish cascade.
"""

from __future__ import annotations

import argparse
import io
import json
import os
import subprocess
import sys
import tarfile
import tempfile
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor


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


def shard_slice(packages: list[str], index: int, total: int) -> list[str]:
    """The INDEX-th deterministic slice of TOTAL over the sorted candidates."""
    return sorted(packages)[index::total]


def parse_version_tuple(version: str):
    """Comparable key: numeric release tuple, pre-releases sort below the release."""
    core = version.split("+", 1)[0]
    base, dash, _pre = core.partition("-")
    try:
        nums = tuple(int(part) for part in base.split("."))
    except ValueError:
        return (0, ())
    return ((2, nums) if not dash else (1, nums))


def declared_version_allows_breaking_change(current: str, baseline: str) -> bool:
    """Whether CURRENT already advances Cargo's compatibility-breaking slot."""

    def stable_release(version: str) -> tuple[int, int, int] | None:
        core = version.split("+", 1)[0]
        if "-" in core:
            return None
        parts = core.split(".")
        if len(parts) != 3:
            return None
        try:
            return tuple(int(part) for part in parts)
        except ValueError:
            return None

    def breaking_slot(release: tuple[int, int, int]) -> tuple[int, int, int]:
        major, minor, patch = release
        if major:
            return major, 0, 0
        if minor:
            return 0, minor, 0
        return 0, 0, patch

    current_release = stable_release(current)
    baseline_release = stable_release(baseline)
    if current_release is None or baseline_release is None:
        return False
    return breaking_slot(current_release) > breaking_slot(baseline_release)


def index_records(name: str) -> list[tuple[str, bool]]:
    """(version, yanked) pairs from the sparse index; [] when never published."""
    try:
        body = urllib.request.urlopen(
            f"https://index.crates.io/{index_path(name)}", timeout=30
        ).read().decode()
    except urllib.error.HTTPError as exc:
        if exc.code == 404:
            return []
        raise
    return [
        (entry["vers"], bool(entry.get("yanked")))
        for line in body.splitlines()
        if line.strip()
        for entry in [json.loads(line)]
    ]


INDEX_WORKERS = 10


def fetch_many(names: list[str], fetch) -> dict:
    """Fetch per-crate index data concurrently (order-preserving)."""
    names = list(names)
    with ThreadPoolExecutor(max_workers=min(INDEX_WORKERS, len(names) or 1)) as pool:
        return dict(zip(names, pool.map(fetch, names)))


def latest_published_version(records: list[tuple[str, bool]]) -> str | None:
    """Highest non-yanked version -- the baseline cargo-semver-checks uses."""
    best: str | None = None
    for version, yanked in records:
        if yanked:
            continue
        if best is None or parse_version_tuple(version) > parse_version_tuple(best):
            best = version
    return best


def selected_baseline_version(
    records: list[tuple[str, bool]], exact_version: str | None = None
) -> str | None:
    """Select an exact published version, or the latest non-yanked baseline."""
    if exact_version is not None:
        return exact_version if any(version == exact_version for version, _ in records) else None
    return latest_published_version(records)


def crate_release_tag(name: str, version: str) -> str:
    """The trusted tag created for an independently published crate."""
    return f"crate/{name}/v{version}"


def available_baseline_tag(name: str, version: str) -> str | None:
    """Fetch and validate the historical release tag, if the repository has it."""
    tag = crate_release_tag(name, version)
    revision = f"refs/tags/{tag}^{{commit}}"

    def readable() -> bool:
        present = subprocess.run(
            ["git", "rev-parse", "--verify", revision],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ).returncode == 0
        if not present:
            return False
        # `git archive` verifies that the commit's complete tree is readable.
        # It also hydrates missing blobs before libgit2 clones a partial repo.
        return subprocess.run(
            ["git", "archive", "--format=tar", tag],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ).returncode == 0

    if readable():
        return tag
    fetched = subprocess.run(
        [
            "git",
            "fetch",
            "--force",
            "--depth=1",
            "origin",
            f"refs/tags/{tag}:refs/tags/{tag}",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ).returncode == 0
    return tag if fetched and readable() else None


def normalize_package_version(text: str) -> str:
    """Neutralize only the `[package] version = ...` line (a pure bump)."""
    out, section = [], None
    for line in text.splitlines(keepends=True):
        stripped = line.strip()
        if stripped.startswith("["):
            section = stripped.strip("[]").split()[0]
        if section == "package" and stripped.startswith("version") and "=" in stripped:
            out.append('version = "0.0.0"\n')
        else:
            out.append(line)
    return "".join(out)


GENERATED_PACKAGE_FILES = {
    ".cargo_vcs_info.json",
    "Cargo.lock",
}
PACKAGE_LIST_ONLY_FILES = GENERATED_PACKAGE_FILES | {"Cargo.toml.orig"}


def package_files(name: str) -> set[str]:
    """Files Cargo will include from the current workspace package."""
    output = subprocess.check_output(
        [
            "cargo",
            "package",
            "--list",
            "--allow-dirty",
            "--locked",
            "--package",
            name,
        ],
        text=True,
    )
    return {
        line.strip()
        for line in output.splitlines()
        if line.strip() and line.strip() not in PACKAGE_LIST_ONLY_FILES
    }


def trees_equal(
    workspace_dir: str,
    baseline_dir: str,
    current_package_files: set[str] | None = None,
) -> bool:
    """True when the packaged baseline tree matches the workspace crate.

    Every source file shipped in the .crate must match, with only the package
    version line neutralized. Cargo-generated metadata and lockfiles are
    ignored. ``current_package_files`` comes from ``cargo package --list`` so
    package excludes do not look like source changes, while newly shipped files
    still do.
    """
    baseline_paths: dict[str, str] = {}
    original_manifest = os.path.join(baseline_dir, "Cargo.toml.orig")
    for root, _dirs, files in os.walk(baseline_dir):
        for name in files:
            rel = os.path.relpath(os.path.join(root, name), baseline_dir)
            if rel not in GENERATED_PACKAGE_FILES:
                if rel == "Cargo.toml.orig":
                    baseline_paths["Cargo.toml"] = os.path.join(root, name)
                elif rel != "Cargo.toml" or not os.path.isfile(original_manifest):
                    baseline_paths[rel] = os.path.join(root, name)
    baseline_files = set(baseline_paths)
    if current_package_files is None:
        workspace_files: set[str] = set()
        for root, dirs, files in os.walk(workspace_dir):
            dirs[:] = [d for d in dirs if d != "target"]
            for name in files:
                workspace_files.add(
                    os.path.relpath(os.path.join(root, name), workspace_dir)
                )
    else:
        workspace_files = current_package_files - GENERATED_PACKAGE_FILES
    if baseline_files - workspace_files:
        return False
    if workspace_files - baseline_files:
        return False
    for rel in baseline_files:
        with open(baseline_paths[rel], "rb") as fh:
            baseline_bytes = fh.read()
        with open(os.path.join(workspace_dir, rel), "rb") as fh:
            workspace_bytes = fh.read()
        if rel == "Cargo.toml":
            try:
                baseline_text = normalize_package_version(baseline_bytes.decode("utf-8"))
                workspace_text = normalize_package_version(workspace_bytes.decode("utf-8"))
            except UnicodeDecodeError:
                return False
            if baseline_text != workspace_text:
                return False
        elif baseline_bytes != workspace_bytes:
            return False
    return True


_manifest_dirs: dict[str, str] | None = None


def manifest_dirs() -> dict[str, str]:
    """Workspace package name -> crate directory (one cached cargo metadata)."""
    global _manifest_dirs
    if _manifest_dirs is None:
        meta = json.loads(
            subprocess.check_output(
                ["cargo", "metadata", "--no-deps", "--format-version", "1"], text=True
            )
        )
        _manifest_dirs = {
            p["name"]: os.path.dirname(p["manifest_path"]) for p in meta["packages"]
        }
    return _manifest_dirs


def identical_to_baseline(
    name: str, records=None, exact_version: str | None = None
) -> tuple[bool, str]:
    """Whether a candidate's source is identical to its published baseline.

    Downloads either ``exact_version`` (including a yanked release) or the latest
    non-yanked .crate and compares trees. Returns (True, reason) only on a
    byte-level match (modulo the package version line); every other outcome --
    including any infrastructure failure -- returns False so the crate gets the
    full semver check. This function must never turn the gate green by itself.
    """
    if records is None:
        records = index_records(name)
    baseline = selected_baseline_version(records, exact_version)
    if baseline is None:
        if exact_version is not None:
            return False, f"published baseline {exact_version} not found"
        return False, "no published baseline to compare against"
    try:
        crate_dir = manifest_dirs()[name]
    except KeyError:
        return False, "crate not in workspace metadata"
    url = f"https://static.crates.io/crates/{name}/{name}-{baseline}.crate"
    try:
        request = urllib.request.Request(url)
        with tempfile.TemporaryDirectory(prefix="semver-baseline-") as tmp:
            with urllib.request.urlopen(request, timeout=60) as response:
                payload = response.read()
            with tarfile.open(fileobj=io.BytesIO(payload), mode="r:gz") as archive:
                archive.extractall(tmp, filter="data")
            unpacked = os.path.join(tmp, f"{name}-{baseline}")
            if not os.path.isdir(unpacked):
                return False, "baseline archive layout unexpected"
            if trees_equal(crate_dir, unpacked, package_files(name)):
                return True, f"source identical to published {name} {baseline}"
            return False, f"source differs from published {name} {baseline}"
    except Exception as exc:  # noqa: BLE001 -- fail open by design
        return False, f"baseline comparison unavailable ({exc}); running full check"


def run_semver_checks(packages: list[str], baselines: dict[str, str]) -> int:
    """Run cargo-semver-checks against reproducible per-crate baselines."""
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

    result = 0
    for name in packages:
        baseline = baselines[name]
        argv = ["cargo", "semver-checks", "check-release", "--package", name]
        tag = available_baseline_tag(name, baseline)
        if tag is not None:
            argv += ["--baseline-rev", tag]
        else:
            print(
                f"  - {name}: release tag {crate_release_tag(name, baseline)} "
                "is unavailable; using the registry baseline"
            )
            argv += ["--baseline-version", baseline]
        print(f"$ {' '.join(argv)}", flush=True)
        code = subprocess.call(argv)
        if code != 0:
            result = code
            break
    return result


def same_version_packages(
    current: dict[str, str], published: dict[str, set[str]]
) -> list[str]:
    """Published packages whose workspace version is already burned upstream."""
    return sorted(
        name
        for name, version in current.items()
        if version in (published.get(name) or set())
    )


def check_published_source_versions() -> int:
    """Reject packaged source changes that reuse an already-published version."""
    current = workspace_versions()
    published = fetch_many(current, published_versions)
    packages = same_version_packages(current, published)
    baselines = fetch_many(packages, index_records)
    drift: list[tuple[str, str, str]] = []
    for name in packages:
        identical, reason = identical_to_baseline(
            name, baselines[name], exact_version=current[name]
        )
        if not identical:
            drift.append((name, current[name], reason))
    if not drift:
        print(
            f"Published source versions are unchanged across {len(packages)} package(s)."
        )
        return 0
    print(
        "::error::Packaged source changed without a new crate version. "
        "Published versions are immutable:"
    )
    for name, version, reason in drift:
        print(f"  - {name} {version}: {reason}")
    print(
        "Run `python3 scripts/plan-crate-release.py`, apply the required bumps, "
        "sync internal pins, and close the publish cone before merging."
    )
    return 1


def run_check(plan_only: bool, candidates_only: bool, shard=None) -> int:
    current = workspace_versions()
    published = fetch_many(current, published_versions)
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

    # Crates whose packaged source is byte-identical to the published
    # baseline cannot have changed API; anything else (including any
    # comparison failure) falls through to the full check below.
    remaining: list[str] = []
    baselines = fetch_many(packages, index_records)
    for name in packages:
        identical, reason = identical_to_baseline(name, baselines[name])
        if identical:
            print(f"  - {name} (skipped: {reason})")
        else:
            baseline = latest_published_version(baselines[name])
            if baseline is not None and declared_version_allows_breaking_change(
                current[name], baseline
            ):
                print(
                    f"  - {name} (skipped: {current[name]} already advances the "
                    f"breaking slot from {baseline})"
                )
                continue
            if not reason.startswith("source differs"):
                print(f"  - {name}: {reason}")
            remaining.append(name)
    packages = remaining
    if shard is not None:
        index, total = shard
        packages = shard_slice(packages, index, total)
        print(f"Shard {index}/{total}: checking {len(packages)} candidate(s).")
    if not packages:
        print("No candidates left to check in this shard; nothing to do.")
        return 0

    baseline_versions = {
        name: latest_published_version(baselines[name]) for name in packages
    }
    if any(version is None for version in baseline_versions.values()):
        print("::error::A release candidate has no semver baseline.")
        return 1
    code = run_semver_checks(
        packages,
        {name: version for name, version in baseline_versions.items() if version is not None},
    )
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

    expect("shard 0/2", shard_slice(["c", "a", "b", "d", "e"], 0, 2), ["a", "c", "e"])
    expect("shard 1/2", shard_slice(["c", "a", "b", "d", "e"], 1, 2), ["b", "d"])
    expect("empty shard", shard_slice(["a"], 1, 4), [])
    expect(
        "shards partition",
        sorted(
            shard_slice(["a", "b", "c", "d"], 0, 4)
            + shard_slice(["a", "b", "c", "d"], 1, 4)
            + shard_slice(["a", "b", "c", "d"], 2, 4)
            + shard_slice(["a", "b", "c", "d"], 3, 4)
        ),
        ["a", "b", "c", "d"],
    )

    expect(
        "latest skips yanked",
        latest_published_version([("0.2.0", True), ("0.1.0", False)]),
        "0.1.0",
    )
    expect(
        "latest picks max",
        latest_published_version([("0.1.0", False), ("0.2.0", False)]),
        "0.2.0",
    )
    expect(
        "exact baseline allows yanked version",
        selected_baseline_version([("0.18.1", True), ("0.18.2", False)], "0.18.1"),
        "0.18.1",
    )
    expect(
        "missing exact baseline fails",
        selected_baseline_version([("0.18.2", False)], "0.18.1"),
        None,
    )
    expect(
        "crate release tag",
        crate_release_tag("everruns-host", "0.22.1"),
        "crate/everruns-host/v0.22.1",
    )
    expect("no baseline", latest_published_version([]), None)
    expect(
        "prerelease below release",
        parse_version_tuple("1.2.3") > parse_version_tuple("1.2.3-alpha"),
        True,
    )
    expect(
        "0.x minor bump allows breaking change",
        declared_version_allows_breaking_change("0.23.0", "0.22.1"),
        True,
    )
    expect(
        "0.x patch bump still needs classification",
        declared_version_allows_breaking_change("0.22.2", "0.22.1"),
        False,
    )
    expect(
        "0.0.x patch is the breaking slot",
        declared_version_allows_breaking_change("0.0.4", "0.0.3"),
        True,
    )
    expect(
        "stable major bump allows breaking change",
        declared_version_allows_breaking_change("2.0.0", "1.9.0"),
        True,
    )
    expect(
        "stable minor bump still needs classification",
        declared_version_allows_breaking_change("1.10.0", "1.9.0"),
        False,
    )
    expect(
        "prerelease bump stays fail-closed",
        declared_version_allows_breaking_change("0.23.0-rc.1", "0.22.1"),
        False,
    )
    expect(
        "fetch_many keeps order",
        fetch_many(["b", "a"], lambda n: n.upper()),
        {"b": "B", "a": "A"},
    )
    expect(
        "same-version packages are sorted",
        same_version_packages(
            {"b": "0.2.0", "a": "0.1.0", "new": "0.1.0"},
            {"b": {"0.2.0"}, "a": {"0.1.0"}, "new": set()},
        ),
        ["a", "b"],
    )

    before = '[package]\nname = "demo"\nversion = "0.18.1"\nedition = "2021"\n'
    after = '[package]\nname = "demo"\nversion = "0.18.2"\nedition = "2021"\n'
    expect(
        "pure bump compares equal",
        normalize_package_version(before) == normalize_package_version(after),
        True,
    )
    dep_change = '[package]\nname = "demo"\nversion = "0.19.0"\n[dependencies]\nfoo = "3"\n'
    dep_same = '[package]\nname = "demo"\nversion = "0.18.2"\n[dependencies]\nfoo = "2"\n'
    expect(
        "dep requirement change is visible",
        normalize_package_version(dep_change) == normalize_package_version(dep_same),
        False,
    )

    with (
        tempfile.TemporaryDirectory() as work,
        tempfile.TemporaryDirectory() as base,
        tempfile.TemporaryDirectory() as newer,
    ):
        os.makedirs(os.path.join(work, "src"))
        os.makedirs(os.path.join(base, "src"))
        os.makedirs(os.path.join(newer, "src"))
        with open(os.path.join(work, "Cargo.toml"), "w") as fh:
            fh.write(before)
        with open(os.path.join(base, "Cargo.toml"), "w") as fh:
            fh.write(before)
        with open(os.path.join(base, "Cargo.toml.orig"), "w") as fh:
            fh.write(before)
        with open(os.path.join(base, "Cargo.lock"), "w") as fh:
            fh.write("generated\n")
        with open(os.path.join(work, "src", "lib.rs"), "w") as fh:
            fh.write("pub fn f() {}\n")
        with open(os.path.join(base, "src", "lib.rs"), "w") as fh:
            fh.write("pub fn f() {}\n")
        expect("identical trees", trees_equal(work, base), True)
        with open(os.path.join(base, "src", "lib.rs"), "w") as fh:
            fh.write("pub fn g() {}\n")
        expect("content change detected", trees_equal(work, base), False)
        with open(os.path.join(base, "src", "lib.rs"), "w") as fh:
            fh.write("pub fn f() {}\n")
        with open(os.path.join(work, "extra.rs"), "w") as fh:
            fh.write("// new\n")
        expect("extra workspace file detected", trees_equal(work, base), False)
        expect(
            "package excludes ignore workspace-only files",
            trees_equal(work, base, {"Cargo.toml", "src/lib.rs"}),
            True,
        )
        expect(
            "new packaged files are detected",
            trees_equal(work, base, {"Cargo.toml", "src/lib.rs", "extra.rs"}),
            False,
        )
        with open(os.path.join(newer, "Cargo.toml.orig"), "w") as fh:
            fh.write(after)
        with open(os.path.join(newer, "src", "lib.rs"), "w") as fh:
            fh.write("pub fn g() {}\n")
        records = [("0.18.1", True), ("0.18.2", False)]
        archives = {"0.18.1": base, "0.18.2": newer}
        exact = selected_baseline_version(records, "0.18.1")
        latest = selected_baseline_version(records)
        expect(
            "current source matches its exact older archive",
            trees_equal(work, archives[exact], {"Cargo.toml", "src/lib.rs"}),
            True,
        )
        expect(
            "current source differs from the newer archive",
            trees_equal(work, archives[latest], {"Cargo.toml", "src/lib.rs"}),
            False,
        )

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
    parser.add_argument(
        "--check-source-versions",
        action="store_true",
        help="reject packaged source changes at versions already on crates.io",
    )
    parser.add_argument(
        "--shard",
        default=None,
        metavar="INDEX/COUNT",
        help="check only the INDEX-th slice (0-based) of COUNT candidate shards",
    )
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if args.check_source_versions:
        return check_published_source_versions()
    shard = None
    if args.shard:
        try:
            index_s, count_s = args.shard.split("/")
            shard = (int(index_s), int(count_s))
        except ValueError:
            print(f"--shard must look like INDEX/COUNT, got {args.shard!r}", file=sys.stderr)
            return 2
        if not 0 <= shard[0] < shard[1]:
            print(f"--shard index out of range: {args.shard!r}", file=sys.stderr)
            return 2
    return run_check(plan_only=args.plan, candidates_only=args.candidates, shard=shard)


if __name__ == "__main__":
    sys.exit(main())

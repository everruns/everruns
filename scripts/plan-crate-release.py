#!/usr/bin/env python3
"""Deterministically compute (and optionally apply) the crate version matrix for a release.

Bumping the independently versioned crates by hand is error-prone: a breaking
change smuggled into a patch slot (an added enum variant, a removed Cargo
feature) strands every published dependant, and the cone of dependants that
must be cascade-bumped is easy to under-count. This planner removes the
judgement calls:

1. Enumerate published crates and their latest crates.io baseline.
2. A *candidate* is a published crate whose packaged source differs from its
   baseline (reusing ``check-semver-bumps`` byte-comparison). Those are the
   crates this change actually releases.
3. Classify each candidate breaking vs additive with ``cargo-semver-checks``
   (authoritative), and assign the smallest compatible bump over its baseline:
   ``minor`` for a breaking change (the breaking slot for 0.x), else ``patch``.
4. Close the cone: any *published* crate whose latest crates.io release pins a
   crate that just took a breaking bump at a now-incompatible requirement must
   itself be patch-bumped and re-pinned so it republishes compatibly. Applied
   to a fixpoint (a freshly cascaded crate can strand its own dependants).
5. ``--write`` applies the versions, runs ``sync-publish-pin-versions.py
   --write``, and regenerates every workspace and non-workspace lockfile, then
   re-runs the ``check-semver-bumps`` and ``check-publish-cone`` gates so a
   green plan is proven locally before it ever reaches CI.

``--self-test`` exercises the pure version/cone logic with no network or cargo.

Typical use, before opening a release PR::

    python3 scripts/plan-crate-release.py            # show the plan
    python3 scripts/plan-crate-release.py --write     # apply + verify

The classification step builds rustdoc for each candidate twice, so a full run
is slow (tens of minutes); that is the cost of never shipping an under-bump,
which crates.io makes unrecoverable.
"""

from __future__ import annotations

import argparse
import contextlib
import importlib
import io
import json
import re
import subprocess
import sys
import urllib.request
from pathlib import Path
from unittest import mock

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "scripts"))

# Reuse the vetted crates.io/index and baseline-comparison helpers rather than
# re-deriving them: the two gates and this planner must agree on what "candidate"
# and "baseline" mean, or the plan and the check disagree.
_bumps = importlib.import_module("check-semver-bumps")


class ReleasePlanningError(RuntimeError):
    """The release matrix could not be computed safely."""


class SemverClassificationError(ReleasePlanningError):
    """cargo-semver-checks did not produce a complete classification."""


class LockfileRegenerationError(RuntimeError):
    """cargo could not regenerate a release lockfile."""


def parse_version(version: str) -> tuple[int, int, int]:
    core = version.split("-", 1)[0].split("+", 1)[0]
    parts = [int(x) for x in core.split(".")]
    while len(parts) < 3:
        parts.append(0)
    return parts[0], parts[1], parts[2]


def bump(version: str, level: str) -> str:
    """Smallest compatible next version. For 0.x the minor is the breaking slot."""
    major, minor, patch = parse_version(version)
    if level == "minor":  # breaking
        return f"{major}.{minor + 1}.0"
    if level == "patch":  # additive / cascade
        return f"{major}.{minor}.{patch + 1}"
    raise ValueError(level)


def release_version(current: str, baseline: str, level: str) -> str:
    """Keep an existing bump when it is at least the required semver bump."""
    minimum = bump(baseline, level)
    return max((current, minimum), key=parse_version)


def caret_allows(req: str, ver: str) -> bool:
    """Cargo default (caret) semantics for the all-caret internal graph."""
    r = req.lstrip("^").strip()
    try:
        rp = [int(x) for x in r.split("-")[0].split(".")]
        vp = [int(x) for x in ver.split("-")[0].split(".")]
    except ValueError:
        return True
    while len(rp) < 3:
        rp.append(0)
    while len(vp) < 3:
        vp.append(0)
    if vp < rp:
        return False
    if rp[0] != 0:
        return vp[0] == rp[0]
    if rp[1] != 0:
        return vp[0] == 0 and vp[1] == rp[1]
    return vp[0] == 0 and vp[1] == 0 and vp[2] == rp[2]


def cascade(
    plan: dict[str, str],
    baselines: dict[str, str],
    _breaking: set[str],
    published_deps: dict[str, list[tuple[str, str]]],
) -> dict[str, str]:
    """Close the publish cone to a fixpoint.

    ``plan`` maps crate -> chosen version (only crates being released).
    ``published_deps[name]`` is the ``(dep, req)`` list from ``name``'s latest
    crates.io release. A published crate whose baseline pins a dependency at a
    requirement the dependency's *planned* version no longer satisfies must be
    patch-bumped so it republishes with a compatible pin.
    """
    plan = dict(plan)
    changed = True
    while changed:
        changed = False
        for name, base in baselines.items():
            for dep, req in published_deps.get(name, []):
                dep_ver = plan.get(dep)
                if dep_ver is None:
                    continue
                if not caret_allows(req, dep_ver) and name not in plan:
                    # ``name``'s published pin excludes the planned dependency
                    # version. Republishing ``name`` updates that pin; its own
                    # version number cannot make the old dependency pin valid.
                    plan[name] = bump(base, "patch")
                    changed = True
    return plan


def classify(candidates: list[str], runner=subprocess.run) -> tuple[dict[str, str], str]:
    """crate -> 'minor' (breaking) or 'patch' (additive), via cargo-semver-checks.

    Runs one ``check-release`` over the candidates on the current workspace and
    reads the per-crate verdict. A crate cargo-semver-checks flags as requiring a
    new major (the breaking slot) is 'minor'; everything else is 'patch'.
    """
    if not candidates:
        return {}, ""
    argv = ["cargo", "semver-checks", "check-release"]
    for name in candidates:
        argv += ["--package", name]
    proc = runner(argv, capture_output=True, text=True)
    out = proc.stdout + proc.stderr
    if proc.returncode not in (0, 100):
        raise SemverClassificationError(
            f"cargo-semver-checks exited {proc.returncode} before classification"
        )
    level: dict[str, str] = {}
    requires_update = False
    current: str | None = None
    for line in out.splitlines():
        m = re.search(r"(?:Building|Checking)\s+(everruns\S*)\s+v", line)
        if m and m.group(1) in candidates:
            current = m.group(1)
        if current and "Summary semver requires new major version" in line:
            level[current] = "minor"
            requires_update = True
        elif current and re.search(
            r"Summary semver requires new (?:minor|patch) version", line
        ):
            level[current] = "patch"
            requires_update = True
        elif current and "Summary no semver update required" in line:
            level[current] = "patch"
    missing = sorted(set(candidates) - set(level))
    if missing:
        raise SemverClassificationError(
            "cargo-semver-checks produced no recognized verdict for "
            + ", ".join(missing)
        )
    if (proc.returncode == 100) != requires_update:
        raise SemverClassificationError(
            "cargo-semver-checks exit status disagrees with its package verdicts"
        )
    return level, out


def read_sparse_index(name: str) -> str:
    return (
        urllib.request.urlopen(
            f"https://index.crates.io/{_bumps.index_path(name)}", timeout=30
        )
        .read()
        .decode()
    )


def published_dependency_map(names: list[str]) -> dict[str, list[tuple[str, str]]]:
    records = _bumps.fetch_many(names, _bumps.index_records)
    out: dict[str, list[tuple[str, str]]] = {}
    for name in names:
        latest = _bumps.latest_published_version(records.get(name) or [])
        if latest is None:
            out[name] = []
            continue
        # The sparse index records carry deps on each version row.
        try:
            body = read_sparse_index(name)
            entries = [
                json.loads(row) for row in body.splitlines() if row.strip()
            ]
            selected = [entry for entry in entries if entry.get("vers") == latest]
            deps = [
                (dependency["name"], dependency["req"])
                for entry in selected
                for dependency in entry.get("deps", [])
                if dependency.get("kind") != "dev"
            ]
        except Exception as exc:
            raise ReleasePlanningError(
                f"failed to read published dependencies for {name}: {exc}"
            ) from exc
        if not selected:
            raise ReleasePlanningError(
                f"sparse index omitted selected version {name} {latest}"
            )
        out[name] = deps
    return out


def compute_plan() -> tuple[dict[str, str], dict[str, str], str]:
    current = _bumps.workspace_versions()
    published = _bumps.fetch_many(list(current), _bumps.published_versions)
    baselines = {
        name: _bumps.latest_published_version(_bumps.index_records(name))
        for name in current
    }
    baselines = {n: v for n, v in baselines.items() if v}

    # Candidates: source differs from baseline (this change releases them).
    candidates: list[str] = []
    for name in sorted(current):
        versions = published.get(name) or set()
        if not versions:
            continue  # first publish: nothing to classify
        identical, _ = _bumps.identical_to_baseline(name)
        if not identical:
            candidates.append(name)

    level, log = classify(candidates)
    breaking = {n for n, lv in level.items() if lv == "minor"}

    plan: dict[str, str] = {}
    for name in candidates:
        base = baselines.get(name, current[name])
        plan[name] = release_version(current[name], base, level[name])

    published_deps = published_dependency_map(list(current))
    plan = cascade(plan, baselines, breaking, published_deps)
    return plan, baselines, log


def set_manifest_version(name: str, version: str) -> None:
    meta = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], text=True
        )
    )
    path = next(
        Path(p["manifest_path"]) for p in meta["packages"] if p["name"] == name
    )
    lines = path.read_text().splitlines(keepends=True)
    section = None
    for i, line in enumerate(lines):
        s = line.strip()
        if s.startswith("["):
            section = s.strip("[]").split()[0]
        if section == "package" and re.match(r"version\s*=", s):
            lines[i] = re.sub(r'"[^"]+"', f'"{version}"', line, count=1)
            path.write_text("".join(lines))
            return
    raise SystemExit(f"no [package] version in {path}")


def regenerate_lockfiles() -> None:
    manifests = ["Cargo.toml"]
    manifests += [
        str(p.relative_to(REPO))
        for p in REPO.glob("**/Cargo.lock")
        if "target" not in p.parts and ".local" not in p.parts and p.parent != REPO
    ]
    seen: set[Path] = set()
    for lock in [REPO / "Cargo.lock", *[REPO / m for m in manifests]]:
        manifest = (lock.parent / "Cargo.toml") if lock.name == "Cargo.lock" else lock
        if manifest in seen or not manifest.exists():
            continue
        seen.add(manifest)
        argv = ["cargo", "generate-lockfile", "--manifest-path", str(manifest)]
        try:
            subprocess.run(argv, check=True, capture_output=True, text=True)
        except subprocess.CalledProcessError as exc:
            diagnostic = (exc.stderr or exc.stdout or "no diagnostic").strip()
            raise LockfileRegenerationError(
                f"cargo generate-lockfile failed for {manifest}: {diagnostic}"
            ) from exc


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="apply the plan and verify")
    parser.add_argument("--self-test", action="store_true", help="pure-logic checks, no network")
    args = parser.parse_args()
    if args.self_test:
        return self_test()

    try:
        plan, baselines, _log = compute_plan()
    except ReleasePlanningError as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 1
    if not plan:
        print("No crate releases required: every published crate matches its crates.io baseline.")
        return 0

    print("Planned crate release matrix (baseline -> new):")
    for name in sorted(plan):
        base = baselines.get(name, "?")
        kind = "BREAKING" if parse_version(plan[name])[1] != parse_version(base)[1] else "patch"
        print(f"  {name:44} {base:8} -> {plan[name]:8} ({kind})")

    if not args.write:
        print("\nDry run. Re-run with --write to apply, sync pins, and verify.")
        return 0

    for name, version in plan.items():
        set_manifest_version(name, version)
    subprocess.run(
        [sys.executable, str(REPO / "scripts/sync-publish-pin-versions.py"), "--write"],
        check=True,
    )
    try:
        regenerate_lockfiles()
    except LockfileRegenerationError as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 1
    print("\nApplied. Verifying with the release gates...")
    cone = subprocess.run(
        [sys.executable, str(REPO / "scripts/check-publish-cone.py"), "--pre-merge"]
    )
    bumps = subprocess.run(
        [sys.executable, str(REPO / "scripts/check-semver-bumps.py")]
    )
    return 0 if cone.returncode == 0 and bumps.returncode == 0 else 1


def self_test() -> int:
    failures: list[str] = []

    def expect(label, got, want):
        if got != want:
            failures.append(f"{label}: got {got!r}, want {want!r}")

    expect("0.x minor is breaking slot", bump("0.20.1", "minor"), "0.21.0")
    expect("0.x patch", bump("0.20.1", "patch"), "0.20.2")
    expect("1.x minor", bump("1.4.2", "minor"), "1.5.0")
    expect(
        "existing larger bump is preserved",
        release_version("0.22.0", "0.21.2", "patch"),
        "0.22.0",
    )
    expect(
        "insufficient published version takes breaking bump",
        release_version("0.22.1", "0.22.1", "minor"),
        "0.23.0",
    )
    expect("caret ^0.20 excludes 0.21", caret_allows("^0.20.1", "0.21.0"), False)
    expect(
        "caret ^0.20 allows 0.20.2", caret_allows("^0.20.1", "0.20.2"), True
    )

    def classified_runner(*_args, **_kwargs):
        return subprocess.CompletedProcess(
            [],
            returncode=100,
            stdout=(
                "Building everruns-provider v0.24.0 (current)\n"
                "Summary semver requires new major version: 5 major checks failed\n"
                "Finished [1.0s] everruns-provider\n"
            ),
            stderr="",
        )

    classified, _ = classify(["everruns-provider"], runner=classified_runner)
    expect("recognized major verdict", classified, {"everruns-provider": "minor"})


    def failed_runner(*_args, **_kwargs):
        return subprocess.CompletedProcess(
            [], returncode=101, stdout="", stderr="network or build failure"
        )

    def failed_compute_plan():
        classify(["everruns-provider"], runner=failed_runner)
        raise AssertionError("unreachable")

    def run_write_cli(compute, lockfile_effect=None):
        cli_stdout = io.StringIO()
        cli_stderr = io.StringIO()
        mutations: list[str] = []

        def record_lockfile_write():
            if lockfile_effect is None:
                mutations.append("lockfile")
            else:
                lockfile_effect()

        def record_command(argv, **_kwargs):
            mutations.append("command")
            return subprocess.CompletedProcess(argv, returncode=0)

        with (
            mock.patch.object(sys, "argv", [sys.argv[0], "--write"]),
            mock.patch(__name__ + ".compute_plan", side_effect=compute),
            mock.patch(
                __name__ + ".set_manifest_version",
                side_effect=lambda *_args: mutations.append("manifest"),
            ),
            mock.patch(
                __name__ + ".regenerate_lockfiles",
                side_effect=record_lockfile_write,
            ),
            mock.patch.object(subprocess, "run", side_effect=record_command),
            contextlib.redirect_stdout(cli_stdout),
            contextlib.redirect_stderr(cli_stderr),
        ):
            status = main()
        return status, cli_stdout.getvalue(), cli_stderr.getvalue(), mutations

    failed_cli_status, stdout, stderr, mutations = run_write_cli(failed_compute_plan)
    expect("semver failure exits CLI nonzero", failed_cli_status, 1)
    expect(
        "semver failure produces no matrix",
        "Planned crate release matrix" in stdout,
        False,
    )
    expect(
        "semver failure reports its diagnostic",
        "exited 101" in stderr,
        True,
    )
    expect("semver failure performs no mutations", mutations, [])

    def incomplete_runner(*_args, **_kwargs):
        return subprocess.CompletedProcess(
            [],
            returncode=0,
            stdout="Building everruns-provider v0.24.0 (current)\n",
            stderr="",
        )

    try:
        classify(["everruns-provider"], runner=incomplete_runner)
        incomplete_rejected = False
    except SemverClassificationError:
        incomplete_rejected = True
    expect("missing package verdict is rejected", incomplete_rejected, True)

    def missing_index(*_args, **_kwargs):
        raise OSError("registry unavailable")

    def failed_dependency_compute():
        with (
            mock.patch.object(
                _bumps,
                "fetch_many",
                return_value={"everruns-provider": [("0.24.0", False)]},
            ),
            mock.patch(__name__ + ".read_sparse_index", side_effect=missing_index),
        ):
            published_dependency_map(["everruns-provider"])
        raise AssertionError("unreachable")

    index_cli_status, index_stdout, index_stderr, index_mutations = run_write_cli(
        failed_dependency_compute
    )
    expect("dependency index failure exits CLI nonzero", index_cli_status, 1)
    expect(
        "dependency index failure produces no matrix",
        "Planned crate release matrix" in index_stdout,
        False,
    )
    expect(
        "dependency index failure reports its diagnostic",
        "registry unavailable" in index_stderr,
        True,
    )
    expect("dependency index failure performs no mutations", index_mutations, [])

    lockfile_commands: list[list[str]] = []

    def failed_lockfile_runner(argv, **_kwargs):
        lockfile_commands.append(argv)
        raise subprocess.CalledProcessError(
            101, argv, stderr="dependency resolution failed"
        )


    def successful_compute():
        return (
            {"everruns-provider": "0.25.0"},
            {"everruns-provider": "0.24.0"},
            "",
        )
    lock_stdout = io.StringIO()
    lock_stderr = io.StringIO()
    lock_mutations: list[str] = []

    def failed_write_runner(argv, **_kwargs):
        if argv[0] == "cargo" and argv[1] == "generate-lockfile":
            lock_mutations.append("lockfile")
            return failed_lockfile_runner(argv)
        lock_mutations.append("command")
        return subprocess.CompletedProcess(argv, returncode=0)

    with (
        mock.patch.object(sys, "argv", [sys.argv[0], "--write"]),
        mock.patch(__name__ + ".compute_plan", side_effect=successful_compute),
        mock.patch(
            __name__ + ".set_manifest_version",
            side_effect=lambda *_args: lock_mutations.append("manifest"),
        ),
        mock.patch.object(subprocess, "run", side_effect=failed_write_runner),
        contextlib.redirect_stdout(lock_stdout),
        contextlib.redirect_stderr(lock_stderr),
    ):
        lockfile_cli_status = main()
    lock_stdout = lock_stdout.getvalue()
    lock_stderr = lock_stderr.getvalue()
    expect("lockfile failure exits CLI nonzero", lockfile_cli_status, 1)
    expect("lockfile failure stops at first command", len(lockfile_commands), 1)
    expect(
        "lockfile failure reports cargo diagnostic",
        "dependency resolution failed" in lock_stderr,
        True,
    )
    expect(
        "lockfile failure makes no success claim",
        "Applied. Verifying" in lock_stdout,
        False,
    )
    expect(
        "lockfile failure skips release gates",
        lock_mutations,
        ["manifest", "command", "lockfile"],
    )

    # provider takes a breaking bump; a driver pinning ^0.20 must cascade to patch.
    baselines = {"everruns-provider": "0.20.1", "everruns-openai": "0.18.3"}
    plan = {"everruns-provider": "0.21.0"}
    deps = {"everruns-openai": [("everruns-provider", "^0.20.1")]}
    out = cascade(plan, baselines, {"everruns-provider"}, deps)
    expect("cascade patch-bumps stranded driver", out.get("everruns-openai"), "0.18.4")

    # a driver already bumped past the pin is not cascaded again.
    plan2 = {"everruns-provider": "0.21.0", "everruns-openai": "0.19.0"}
    out2 = cascade(plan2, baselines, {"everruns-provider"}, deps)
    expect("already-bumped driver untouched", out2.get("everruns-openai"), "0.19.0")

    # A dependant version that happens to fit the dependency's caret range still
    # needs republishing: its published dependency requirement is what matters.
    baselines3 = {"everruns-host": "0.22.1", "everruns": "0.22.2"}
    plan3 = {"everruns-host": "0.23.0"}
    deps3 = {"everruns": [("everruns-host", "^0.22.1")]}
    out3 = cascade(plan3, baselines3, {"everruns-host"}, deps3)
    expect("dependent version cannot heal an old pin", out3["everruns"], "0.22.3")

    # additive dep bump strands nobody.
    plan4 = {"everruns-provider": "0.20.2"}
    out4 = cascade(plan4, baselines, set(), deps)
    expect("additive dep bump: no cascade", "everruns-openai" in out4, False)

    for f in failures:
        print(f"  FAIL {f}")
    if failures:
        print(f"{len(failures)} self-test failure(s)")
        return 1
    print("plan-crate-release self-test passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

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
import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "scripts"))

# Reuse the vetted crates.io/index and baseline-comparison helpers rather than
# re-deriving them: the two gates and this planner must agree on what "candidate"
# and "baseline" mean, or the plan and the check disagree.
import importlib

_bumps = importlib.import_module("check-semver-bumps")


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
    breaking: set[str],
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
            current = plan.get(name, base)
            for dep, req in published_deps.get(name, []):
                dep_ver = plan.get(dep)
                if dep_ver is None:
                    continue
                if not caret_allows(req, dep_ver) and not caret_allows(req, current):
                    # ``name``'s published pin excludes the planned dep version,
                    # and ``name`` is not itself bumped past that pin yet.
                    if name not in plan:
                        plan[name] = bump(base, "patch")
                        changed = True
    return plan


def classify(candidates: list[str]) -> dict[str, str]:
    """crate -> 'minor' (breaking) or 'patch' (additive), via cargo-semver-checks.

    Runs one ``check-release`` over the candidates on the current workspace and
    reads the per-crate verdict. A crate cargo-semver-checks flags as requiring a
    new major (the breaking slot) is 'minor'; everything else is 'patch'.
    """
    argv = ["cargo", "semver-checks", "check-release"]
    for name in candidates:
        argv += ["--package", name]
    proc = subprocess.run(argv, capture_output=True, text=True)
    out = proc.stdout + proc.stderr
    level: dict[str, str] = {name: "patch" for name in candidates}
    current: str | None = None
    for line in out.splitlines():
        m = re.search(r"(?:Building|Checking)\s+(everruns\S*)\s+v", line)
        if m and m.group(1) in level:
            current = m.group(1)
        if "requires new major version" in line and current:
            level[current] = "minor"
    return level, out


def published_dependency_map(names: list[str]) -> dict[str, list[tuple[str, str]]]:
    records = _bumps.fetch_many(names, _bumps.index_records)
    out: dict[str, list[tuple[str, str]]] = {}
    for name in names:
        latest = _bumps.latest_published_version(records.get(name) or [])
        if latest is None:
            out[name] = []
            continue
        # The sparse index records carry deps on each version row.
        body = None
        try:
            import urllib.request

            body = (
                urllib.request.urlopen(
                    f"https://index.crates.io/{_bumps.index_path(name)}", timeout=30
                )
                .read()
                .decode()
            )
        except Exception:
            out[name] = []
            continue
        deps: list[tuple[str, str]] = []
        for row in body.splitlines():
            if not row.strip():
                continue
            entry = json.loads(row)
            if entry.get("vers") != latest:
                continue
            for d in entry.get("deps", []):
                if d.get("kind") != "dev":
                    deps.append((d["name"], d["req"]))
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
        plan[name] = bump(base, level[name])

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
        subprocess.run(
            ["cargo", "generate-lockfile", "--manifest-path", str(manifest)],
            check=False,
            capture_output=True,
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write", action="store_true", help="apply the plan and verify")
    parser.add_argument("--self-test", action="store_true", help="pure-logic checks, no network")
    args = parser.parse_args()
    if args.self_test:
        return self_test()

    plan, baselines, _log = compute_plan()
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
    regenerate_lockfiles()
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
    expect("caret ^0.20 excludes 0.21", caret_allows("^0.20.1", "0.21.0"), False)
    expect("caret ^0.20 allows 0.20.2", caret_allows("^0.20.1", "0.20.2"), True)

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

    # additive dep bump strands nobody.
    plan3 = {"everruns-provider": "0.20.2"}
    out3 = cascade(plan3, baselines, set(), deps)
    expect("additive dep bump: no cascade", "everruns-openai" in out3, False)

    for f in failures:
        print(f"  FAIL {f}")
    if failures:
        print(f"{len(failures)} self-test failure(s)")
        return 1
    print("plan-crate-release self-test passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

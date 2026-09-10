#!/usr/bin/env python3
"""Deterministically plan publish-crate version bumps for a release cycle.

Computes the full cascade (closure) of required crate bumps so the
pre-merge publish-cone gate (scripts/check-publish-cone.py --pre-merge) passes
on the first CI run -- no more chasing strand failures one round at a time.

Usage:
  plan-crate-bumps.py [--base REF] [--bump name:level ...] [--apply] [--check]

  --base REF        Cycle base; default is the latest `vN.N.N` product tag.
  --bump name:level Force a seed bump (level defaults to patch). Seeds are
                    otherwise auto-detected as published crates with
                    non-manifest changes since base.
  --apply           Rewrite the [package] versions in the planned manifests.
                    Afterwards run sync-publish-pin-versions.py --write,
                    `cargo update -p ...`, refresh the external-consumer
                    fixture lock, and check-semver-bumps.py to confirm levels.
  --check           Exit 1 when the plan is non-empty (CI wiring).
  --self-test       Exercise the pure version logic with no network/git.

How it works (fully offline, no network):
  1. Seeds: published crates whose shipped code changed since base, IGNORING
     manifest-only changes (pin syncs and dependency bumps are not contract
     changes) and tests/benches/examples-only churn (not shipped to
     downstream consumers). Force seeds with --bump when in doubt.
  2. Already-bumped crates (worktree version != base version) count as
     bumped at their implied level and are never re-planned.
  3. Closure: a minor/major bump strands every PUBLISHED direct dependant
     (normal + build deps, dev-deps excluded -- mirroring the cone gate)
     whose base requirement does not caret-admit the planned version; each
     stranded crate needs at least a patch. Patch bumps never propagate.
     Repeat to fixpoint.
  4. Bump levels follow the repo's 0.x rule: patch for additive/behavior
     (non-breaking), minor for breaking (the 0.x breaking slot). The planner
     takes seed levels as input (default patch); check-semver-bumps.py
     remains the authority -- if it demands a higher level, re-run the
     planner with --bump name:minor and the closure expands accordingly.

Correctness invariant: every version bump in <base> is already published
(true on main after the publish workflows run, and on product tags). Then
"worktree version != base version" is exactly "bumped in this change" and
"base manifest requirement" is exactly "latest published requirement", so
the plan matches the cone gate's verdict deterministically.
"""

from __future__ import annotations

import argparse
import glob
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LEVELS = ("patch", "minor", "major")


def sh(*args: str) -> str:
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def caret_allows(req: str, ver: str) -> bool:
    """Cargo default (caret) semantics. Mirrors check-publish-cone.py."""
    r = req.lstrip("^").strip()
    try:
        rp = [int(x) for x in r.split("-")[0].split(".")]
        vp = [int(x) for x in ver.split("-")[0].split(".")]
    except ValueError:
        return True  # non-caret / complex req: don't flag, avoid false positives
    while len(rp) < 3:
        rp.append(0)
    while len(vp) < 3:
        vp.append(0)
    if vp < rp:  # lower bound
        return False
    # upper bound: first non-zero component of the requirement fixes the range
    if rp[0] != 0:
        return vp[0] == rp[0]
    if rp[1] != 0:
        return vp[0] == 0 and vp[1] == rp[1]
    return vp[0] == 0 and vp[1] == 0 and vp[2] == rp[2]


def bump_level(old: str, new: str) -> str:
    """Implied bump level between two versions under the repo's 0.x rule."""
    o = [int(x) for x in old.split("-")[0].split(".")]
    n = [int(x) for x in new.split("-")[0].split(".")]
    if n[0] != o[0]:
        return "major"
    if n[1] != o[1]:
        return "minor"
    return "patch"


def bumped_version(old: str, level: str) -> str:
    o = [int(x) for x in old.split("-")[0].split(".")]
    while len(o) < 3:
        o.append(0)
    if level == "major":
        return f"{o[0] + 1}.0.0"
    if level == "minor":
        return f"{o[0]}.{o[1] + 1}.0"
    return f"{o[0]}.{o[1]}.{o[2] + 1}"


def load_manifest(path: Path) -> dict:
    with open(path, "rb") as f:
        return tomllib.load(f)


def workspace_deps() -> dict:
    return load_manifest(ROOT / "Cargo.toml").get("workspace", {}).get("dependencies", {})


def all_crates() -> dict[str, dict]:
    """name -> {dir, version, published}. Workspace-wide, manifest truth."""
    out = {}
    for path in sorted(glob.glob("crates/**/Cargo.toml", root_dir=ROOT, recursive=True)):
        d = load_manifest(ROOT / path)
        if "package" not in d:
            continue
        name = d["package"]["name"]
        out[name] = {
            "dir": str(Path(path).parent),
            "version": d["package"]["version"],
            "published": d["package"].get("publish", True) is not False,
        }
    return out


def dep_edges(ws_deps: dict) -> dict[str, list[tuple[str, str]]]:
    """name -> [(dep_package, req)] for non-dev deps, workspace-resolved."""
    edges: dict[str, list[tuple[str, str]]] = {}
    for path in sorted(glob.glob("crates/**/Cargo.toml", root_dir=ROOT, recursive=True)):
        d = load_manifest(ROOT / path)
        if "package" not in d:
            continue
        name = d["package"]["name"]
        sections: dict = {}
        for sec in ("dependencies", "build-dependencies"):
            sections.update(d.get(sec, {}))
        for tgt in d.get("target", {}).values():
            for sec in ("dependencies", "build-dependencies"):
                sections.update(tgt.get(sec, {}))
        deps = []
        for key, spec in sections.items():
            pkg = key
            req = None
            if isinstance(spec, dict):
                pkg = spec.get("package", key)
                if spec.get("workspace", False):
                    w = ws_deps.get(key, {})
                    req = w if isinstance(w, str) else w.get("version")
                    if isinstance(w, dict):
                        pkg = w.get("package", pkg)
                else:
                    req = spec.get("version")
            else:
                req = spec
            if req:
                deps.append((pkg, str(req)))
        edges[name] = deps
    return edges


def base_manifest(name: str, crate_dir: str, base: str) -> dict | None:
    try:
        blob = sh("git", "show", f"{base}:{crate_dir}/Cargo.toml")
    except subprocess.CalledProcessError:
        return None
    import io as _io

    return tomllib.load(_io.BytesIO(blob.encode()))


def latest_product_tag() -> str:
    tags = sh("git", "tag", "--list", "v[0-9]*", "--sort=-v:refname").splitlines()
    if not tags:
        raise SystemExit("no vN.N.N product tag found; pass --base explicitly")
    return tags[0]


def plan(base: str, forced: dict[str, str]) -> tuple[dict[str, str], dict[str, str], list[str]]:
    """Returns (planned_bumps, already_bumped, notes). Levels are patch/minor/major."""
    crates = all_crates()
    ws = workspace_deps()
    edges = dep_edges(ws)

    already: dict[str, str] = {}
    for name, info in crates.items():
        b = base_manifest(name, info["dir"], base)
        if b is None:
            continue
        old = b["package"]["version"]
        if info["version"] != old:
            already[name] = bump_level(old, info["version"])

    changed = sh("git", "diff", "--name-only", f"{base}...HEAD", "--", "crates/").splitlines()
    seeds: dict[str, str] = dict(forced)
    notes: list[str] = []
    for f in changed:
        p = Path(f)
        if p.name in ("Cargo.toml", "Cargo.lock"):
            continue  # pin syncs / dep bumps are not contract changes
        for name, info in crates.items():
            prefix = info["dir"] + "/"
            if p.as_posix() == info["dir"] or p.as_posix().startswith(prefix):
                rel = p.as_posix()[len(prefix):].split("/")
                if rel[0] in ("tests", "benches", "examples"):
                    break  # test-only churn is not a contract change
                if name not in already and crates[name]["published"] and name not in seeds:
                    seeds[name] = "patch"
                break

    # Planned versions start at the seed levels over the BASE versions, so the
    # closure sees the true published requirement of each dependant.
    planned: dict[str, str] = {}
    for name, level in {**already, **seeds}.items():
        if name not in already:
            b = base_manifest(name, crates[name]["dir"], base)
            old = b["package"]["version"] if b else crates[name]["version"]
            planned[name] = bumped_version(old, level)
        else:
            planned[name] = crates[name]["version"]

    def req_in_base(user: str, dep: str) -> str | None:
        info = crates[user]
        b = base_manifest(user, info["dir"], base)
        if b is None:
            return None
        base_ws = tomllib.loads(sh("git", "show", f"{base}:Cargo.toml")).get("workspace", {}).get(
            "dependencies", {}
        )
        sections: dict = {}
        for sec in ("dependencies", "build-dependencies"):
            sections.update(b.get(sec, {}))
        for tgt in b.get("target", {}).values():
            for sec in ("dependencies", "build-dependencies"):
                sections.update(tgt.get(sec, {}))
        for key, spec in sections.items():
            pkg = key
            if isinstance(spec, dict):
                if spec.get("workspace", False):
                    w = base_ws.get(key, {})
                    pkg = w.get("package", key) if isinstance(w, dict) else key
                    if pkg == dep:
                        return w if isinstance(w, str) else w.get("version")
                else:
                    pkg = spec.get("package", key)
                    if pkg == dep:
                        return spec.get("version")
            elif key == dep:
                return spec
        return None

    # Fixpoint: minor/major bumps strand published dependants (dev-deps excluded,
    # mirroring the cone gate); each stranded crate needs at least a patch.
    # Patches never propagate.
    changed_loop = True
    while changed_loop:
        changed_loop = False
        for name, ver in list(planned.items()):
            base_ver = None
            b = base_manifest(name, crates[name]["dir"], base)
            if b is not None:
                base_ver = b["package"]["version"]
            level = bump_level(base_ver, ver) if base_ver else "patch"
            if level == "patch":
                continue
            for user, deps in edges.items():
                if user in planned or not crates[user]["published"]:
                    continue
                if not any(pkg == name for pkg, _ in deps):
                    continue
                req = req_in_base(user, name)
                if req is None or not caret_allows(req, ver):
                    ub = base_manifest(user, crates[user]["dir"], base)
                    uold = ub["package"]["version"] if ub else crates[user]["version"]
                    planned[user] = bumped_version(uold, "patch")
                    notes.append(f"{user} stranded by {name} {ver} (req {req}); +patch")
                    changed_loop = True

    new = {n: v for n, v in planned.items() if n not in already}
    return new, already, notes


def apply_bumps(bumps: dict[str, str], crates: dict[str, dict]) -> None:
    for name, ver in sorted(bumps.items()):
        path = ROOT / crates[name]["dir"] / "Cargo.toml"
        lines = path.read_text().splitlines(keepends=True)
        in_pkg, done = False, False
        for i, line in enumerate(lines):
            s = line.strip()
            if s.startswith("["):
                in_pkg = s == "[package]"
            elif in_pkg and s.startswith("version") and not done:
                lines[i] = f'version = "{ver}"\n'
                done = True
        assert done, f"no [package] version found in {path}"
        path.write_text("".join(lines))
        print(f"  {name}: -> {ver}")


def self_test() -> None:
    assert caret_allows("0.19.1", "0.19.2") and caret_allows("0.19.1", "0.19.9")
    assert not caret_allows("0.19.1", "0.20.0")
    assert caret_allows("0.20.0", "0.20.0")
    assert not caret_allows("0.20.1", "0.20.0")
    assert bump_level("0.19.1", "0.20.0") == "minor"
    assert bump_level("0.18.6", "0.18.7") == "patch"
    assert bumped_version("0.19.1", "minor") == "0.20.0"
    assert bumped_version("0.18.6", "patch") == "0.18.7"
    print("self-test ok")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--base", default=None)
    ap.add_argument("--bump", action="append", default=[],
                    help="name[:patch|minor|major] seed bump (default patch)")
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    base = args.base or latest_product_tag()
    forced: dict[str, str] = {}
    for item in args.bump:
        n, _, lvl = item.partition(":")
        lvl = lvl or "patch"
        assert lvl in LEVELS, f"bad level in --bump {item}"
        forced[n] = lvl
    bumps, already, notes = plan(base, forced)
    print(f"base: {base}")
    if already:
        print("already bumped:")
        for n in sorted(already):
            print(f"  {n}: {already[n]}")
    if not bumps:
        print("plan: empty -- publish cone is closed, nothing to bump.")
        return 0
    print("plan (apply in any order; publish-crates orders the release):")
    crates = all_crates()
    for n in sorted(bumps):
        print(f"  {n}: -> {bumps[n]} (patch)")
    for note in notes:
        print(f"  note: {note}")
    if args.apply:
        print("applying:")
        apply_bumps(bumps, crates)
        print("next: sync-publish-pin-versions.py --write, then "
              "`cargo update -p <crates>`, refresh tests/fixtures/external-consumer "
              "lock (`cargo check` there), then check-semver-bumps.py.")
    return 1 if args.check else 0


if __name__ == "__main__":
    sys.exit(main())

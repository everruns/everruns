#!/usr/bin/env python3
"""Deterministic guard against a partial (stranded) crate-publish cone.

A breaking bump of a foundational crate (e.g. everruns-core 0.18 -> 0.19, or
everruns-provider 0.19 -> 0.20) leaves every *published* dependant whose latest
crates.io version still pins the old, now-incompatible requirement unable to
resolve alongside the new foundational crate. A downstream consumer then pulls
two copies of the foundational crate, and any facade that re-exports both sides
fails to compile during `cargo publish` verification -- exactly how the
everruns 0.19.0 facade publish broke in the v0.22.0 release.

This checker compares each published crate's latest crates.io pins against the
versions the workspace now ships, using the same crates.io sparse index and
caret semantics as crate-release.yml's post-publish strand check. It runs in two
modes:

  (default, strict)  Flag every stranded published dependant. Intended to run
                     *after* publishing (crate-release.yml), where a freshly
                     cascaded version already counts as fixed.

  --pre-merge        Flag a strand only when the stranded crate is NOT being
                     bumped in this change (its workspace version already equals
                     its latest published version). A crate whose version is
                     bumped here will be republished by the same crate-release
                     run and re-pinned via workspace inheritance, so it is not a
                     permanent strand. This makes the check a deterministic
                     pre-merge gate: a release PR that bumps a foundational
                     crate but forgets to cascade its published dependants fails
                     CI, while the cascade-fix PR that bumps them passes.

`--self-test` exercises the pure caret/strand logic against fixtures with no
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


def latest_published(name: str) -> dict | None:
    """Latest non-yanked release row from the crates.io sparse index, or None."""
    try:
        body = urllib.request.urlopen(
            f"https://index.crates.io/{index_path(name)}", timeout=30
        ).read().decode()
    except urllib.error.HTTPError as exc:
        if exc.code == 404:
            return None
        raise
    rows = [
        json.loads(line)
        for line in body.splitlines()
        if line.strip() and not json.loads(line).get("yanked")
    ]
    return rows[-1] if rows else None


def caret_allows(req: str, ver: str) -> bool:
    """Cargo default (caret) semantics, sufficient for this all-caret graph."""
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


def workspace_versions() -> dict[str, str]:
    meta = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], text=True
        )
    )
    # Published packages are those whose manifest does not set publish = false
    # (cargo reports publish = [] for those).
    return {p["name"]: p["version"] for p in meta["packages"] if p.get("publish") != []}


def find_strands(current: dict[str, str], pre_merge: bool) -> list[str]:
    """Return human-readable strand descriptions.

    A strand is a *published* crate whose latest crates.io release pins a sibling
    workspace crate at a requirement the version now in the workspace violates.
    In pre_merge mode a strand is ignored when the stranding crate is itself
    being bumped in this change (workspace version != latest published), because
    that republish heals it.
    """
    strands: list[str] = []
    for name in sorted(current):
        latest = latest_published(name)
        if not latest:
            continue  # never published: cannot strand a consumer yet
        if pre_merge and current[name] != latest["vers"]:
            continue  # bumped here -> will be republished and re-pinned; not permanent
        for dep in latest.get("deps", []):
            dname = dep["name"]
            if dname in current and dep.get("kind") != "dev":
                if not caret_allows(dep["req"], current[dname]):
                    strands.append(
                        f"{name} {latest['vers']} pins {dname} {dep['req']}, "
                        f"but the workspace now ships {dname} {current[dname]}"
                    )
    return strands


def run_check(pre_merge: bool) -> int:
    current = workspace_versions()
    strands = find_strands(current, pre_merge)
    if strands:
        if pre_merge:
            print(
                "::error::Publish cone is inconsistent: a foundational crate was "
                "bumped incompatibly but these published dependants are not "
                "cascade-bumped in this change and would strand on publish:"
            )
        else:
            print(
                "::error::Partial release: published crates stranded by an "
                "incompatible dependency:"
            )
        for s in strands:
            print(f"  - {s}")
        print(
            "Cascade-bump and republish each stranded crate (a patch bump is "
            "enough; the pin re-syncs via workspace inheritance), or yank it via "
            "the Yank Crate workflow if it was absorbed/removed."
        )
        return 1
    scope = "unhealed" if pre_merge else "stranded"
    print(f"No {scope} published dependants across {len(current)} crates.")
    return 0


def self_test() -> int:
    """Network-free checks of the pure caret and strand logic."""
    failures: list[str] = []

    def expect(label: str, got, want) -> None:
        if got != want:
            failures.append(f"{label}: got {got!r}, want {want!r}")

    # Caret upper/lower bounds around the release that actually broke.
    expect("^0.18.1 allows 0.19.0", caret_allows("^0.18.1", "0.19.0"), False)
    expect("^0.18.1 allows 0.18.4", caret_allows("^0.18.1", "0.18.4"), True)
    expect("^0.19.0 allows 0.20.0", caret_allows("^0.19.0", "0.20.0"), False)
    expect("^0.19.0 allows 0.19.2", caret_allows("^0.19.0", "0.19.2"), True)
    expect("^1.2 allows 1.9", caret_allows("^1.2", "1.9.0"), True)
    expect("^1.2 allows 2.0", caret_allows("^1.2", "2.0.0"), False)
    expect("below lower bound", caret_allows("^0.19.0", "0.18.9"), False)

    # Strand detection over a fixture cone, monkeypatching the index lookup.
    global latest_published
    real = latest_published
    published = {
        # facade re-exports host (new core) and filesystem (old core) -> conflict
        "everruns-host": {"vers": "0.20.3", "deps": [{"name": "everruns-core", "req": "^0.19.0", "kind": "normal"}]},
        "everruns-integrations-filesystem": {"vers": "0.18.2", "deps": [{"name": "everruns-core", "req": "^0.18.1", "kind": "normal"}]},
        "everruns-core": {"vers": "0.18.4", "deps": []},
    }
    latest_published = lambda name: published.get(name)  # noqa: E731
    try:
        workspace = {
            "everruns-core": "0.19.0",
            "everruns-host": "0.20.3",
            "everruns-integrations-filesystem": "0.18.2",  # NOT bumped -> unhealed strand
        }
        strict = find_strands(workspace, pre_merge=False)
        expect("strict flags filesystem strand", any("filesystem" in s for s in strict), True)
        pre = find_strands(workspace, pre_merge=True)
        expect("pre-merge flags un-bumped filesystem", any("filesystem" in s for s in pre), True)

        healed = dict(workspace, **{"everruns-integrations-filesystem": "0.18.3"})  # bumped
        pre_healed = find_strands(healed, pre_merge=True)
        expect("pre-merge ignores bumped filesystem", pre_healed, [])
        strict_healed = find_strands(healed, pre_merge=False)
        expect("strict still flags until republished", any("filesystem" in s for s in strict_healed), True)
    finally:
        latest_published = real

    if failures:
        print("check-publish-cone self-test FAILED:", file=sys.stderr)
        for f in failures:
            print(f"  - {f}", file=sys.stderr)
        return 1
    print("check-publish-cone self-test passed.")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--pre-merge",
        action="store_true",
        help="ignore strands healed by a version bump in this change (PR gate)",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run network-free logic checks and exit",
    )
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    return run_check(pre_merge=args.pre_merge)


if __name__ == "__main__":
    raise SystemExit(main())

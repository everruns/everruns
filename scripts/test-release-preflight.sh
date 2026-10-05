#!/usr/bin/env bash
# Guard scripts/release-preflight.py: the real tree passes offline, and each
# release-numbering mistake it exists for fails against a stubbed crates.io.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_ROOT"

python3 scripts/release-preflight.py

python3 - <<'PY'
import importlib.util
import json
import os
import pathlib
import tempfile

failures: list[str] = []


def require(condition: bool, message: str) -> None:
    if not condition:
        failures.append(message)


tmp = pathlib.Path(tempfile.mkdtemp())
(tmp / ".github").mkdir()
(tmp / "apps/ui").mkdir(parents=True)


def tree(version="0.5.0", ui=None, changelog=None, listed=("crate-a", "crate-b")):
    (tmp / "Cargo.toml").write_text(f'[workspace.package]\nversion = "{version}"\n')
    (tmp / "apps/ui/package.json").write_text(json.dumps({"version": ui or version}))
    (tmp / "CHANGELOG.md").write_text(
        f"# Changelog\n\n## [Unreleased]\n\n## [{changelog or version}] - 2026-10-02\n\n## [0.4.0]\n"
    )
    (tmp / ".github/crates-publish-set.txt").write_text(
        "# comment\n" + "".join(f"{name}\n" for name in listed)
    )


def registry(crates):
    path = tmp / "registry.json"
    path.write_text(json.dumps({"crates": crates}))
    os.environ["RELEASE_PREFLIGHT_REGISTRY_FIXTURE"] = str(path)


os.environ["RELEASE_PREFLIGHT_REPO"] = str(tmp)
spec = importlib.util.spec_from_file_location("preflight", "scripts/release-preflight.py")
preflight = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preflight)

CRATES = {"crate-a": "0.5.0", "crate-b": "0.5.0"}
OURS = ["chaliy"]
PUBLISHED = {
    "everruns-core": {"versions": ["0.4.0"], "owners": OURS},
    "crate-a": {"versions": ["0.3.0", "0.4.0"], "owners": OURS},
    "crate-b": {"versions": ["0.4.0"], "owners": OURS},
}


def offline():
    f = preflight.Failures()
    preflight.check_publish_set(CRATES, f)
    preflight.check_version_files(preflight.workspace_version(), f)
    return f.items


def online(crates=CRATES, version="0.5.0"):
    f = preflight.Failures()
    reg = preflight.Registry()
    new = preflight.check_new_names(crates, reg, f)
    preflight.check_version_is_new(version, crates, reg, f)
    return f.items, new


# Offline: a consistent tree passes.
tree()
require(offline() == [], f"consistent tree must pass offline: {offline()}")

# A crate that drops `publish = false` without joining the list fails.
tree(listed=("crate-a",))
items = offline()
require(any("crate-b is publishable but not in" in i for i in items), f"unlisted crate must fail: {items}")

# A listed crate that is no longer publishable fails.
tree(listed=("crate-a", "crate-b", "crate-c"))
items = offline()
require(any("crate-c is in" in i for i in items), f"stale list entry must fail: {items}")

# The list must stay sorted.
tree(listed=("crate-b", "crate-a"))
require(any("must be sorted" in i for i in offline()), "unsorted list must fail")

# package.json and CHANGELOG must carry the workspace version.
tree(ui="0.4.0")
require(any("package.json is 0.4.0" in i for i in offline()), "package.json drift must fail")
tree(changelog="0.4.1")
require(any("newest CHANGELOG.md section is 0.4.1" in i for i in offline()), "changelog drift must fail")

# Online: the next version over a fully published 0.4.0 passes.
tree()
registry(PUBLISHED)
items, new = online()
require(items == [] and new == [], f"clean release must pass: {items} {new}")

# A version some crate already holds is a burnt number (0.34.1 after 0.34.0 halted).
registry({**PUBLISHED, "crate-a": {"versions": ["0.4.0", "0.5.0"], "owners": OURS}})
items, _ = online()
require(any("already on crates.io for 1 crate(s) (crate-a)" in i for i in items), f"partial version must fail: {items}")

# A version at or below what crates.io holds fails.
registry({**PUBLISHED, "crate-b": {"versions": ["0.4.0", "0.6.0"], "owners": OURS}})
items, _ = online()
require(any("not higher than 0.6.0" in i for i in items), f"lower version must fail: {items}")

# Pre-releases sort below their release.
require(preflight.version_key("0.5.0-rc.1") < preflight.version_key("0.5.0"), "pre-release must sort first")
require(preflight.version_key("0.10.0") > preflight.version_key("0.9.9"), "versions compare numerically")

# A never-published name is reported as new and passes.
registry({k: v for k, v in PUBLISHED.items() if k != "crate-b"})
items, new = online()
require(items == [] and new == ["crate-b"], f"free new name must pass and be reported: {items} {new}")

# A name someone else owns fails.
registry({**PUBLISHED, "crate-b": {"versions": ["9.0.0"], "owners": ["someone-else"]}})
items, _ = online()
require(any("crate-b already exists on crates.io, owned by someone else" in i for i in items), f"taken name must fail: {items}")

# More new names than the crates.io burst fails.
many = {f"crate-{i}": "0.5.0" for i in range(preflight.NEW_CRATE_BURST + 1)}
registry({"everruns-core": PUBLISHED["everruns-core"]})
items, _ = online(crates=many)
require(any("crates would be created on crates.io in one release" in i for i in items), f"burst must fail: {items}")

# Audit lists what a halted cascade left behind.
registry({**PUBLISHED, "crate-a": {"versions": ["0.5.0"], "owners": OURS}})
missing = preflight.audit("0.5.0", CRATES, preflight.Registry())
require(missing == ["crate-b (crates.io has 0.4.0)"], f"audit must list the missing crate: {missing}")
registry({"crate-a": {"versions": ["0.5.0"]}})
missing = preflight.audit("0.5.0", CRATES, preflight.Registry())
require(missing == ["crate-b (not on crates.io yet)"], f"audit must list a never-published crate: {missing}")

# Provenance: one commit for every tag passes; a mixed or untagged version fails.
SHA_A, SHA_B = "a" * 40, "b" * 40
both = {"crate/crate-a/v0.5.0": SHA_A, "crate/crate-b/v0.5.0": SHA_A}
require(preflight.audit_provenance("0.5.0", CRATES, both) == [], "one release commit must pass")
mixed = preflight.audit_provenance(
    "0.5.0", {**CRATES, "crate-c": "0.5.0"}, {**both, "crate/crate-c/v0.5.0": SHA_B}
)
require(mixed == [f"crate-c tagged at {SHA_B[:9]}, not the release commit {SHA_A[:9]}"],
        f"a crate released from another commit must fail (0.34.1): {mixed}")
untagged = preflight.audit_provenance("0.5.0", CRATES, {"crate/crate-a/v0.5.0": SHA_A})
require(untagged == ["crate-b has no crate/crate-b/v0.5.0 release tag"], f"an untagged crate must fail: {untagged}")
# A crate that joined the publish set after the version was cut waits for the
# next version (durable-engine after 0.41.0); a tagged one failed to publish.
registry({"crate-a": {"versions": ["0.5.0"]}})
pending = preflight.pending_first_release("0.5.0", CRATES, preflight.Registry(), both)
require(pending == [], f"a tagged never-published crate is a failed publish, not pending: {pending}")
pending = preflight.pending_first_release(
    "0.5.0", CRATES, preflight.Registry(), {"crate/crate-a/v0.5.0": SHA_A}
)
require(pending == ["crate-b"], f"an untagged never-published crate must wait: {pending}")
registry({"crate-a": {"versions": ["0.5.0"]}, "crate-b": {"versions": ["0.4.0"]}})
pending = preflight.pending_first_release(
    "0.5.0", CRATES, preflight.Registry(), {"crate/crate-a/v0.5.0": SHA_A}
)
require(pending == [], f"a crate already on crates.io is never pending: {pending}")

tags_path = tmp / "tags.json"
tags_path.write_text(json.dumps({"tags": both}))
os.environ["RELEASE_PREFLIGHT_TAGS_FIXTURE"] = str(tags_path)
require(preflight.release_tags("0.5.0") == both, "tags fixture must load")

if failures:
    raise SystemExit("release preflight test failures:\n  " + "\n  ".join(failures))
print("release preflight tests passed")
PY

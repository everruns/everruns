#!/usr/bin/env bash
# Guard crates.io release ordering and the single platform version.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_ROOT"

python3 scripts/sync-publish-pin-versions.py --check

python3 - <<'PY'
import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile
import tomllib

repo = pathlib.Path.cwd()
workflow = (repo / ".github/workflows/publish-crates.yml").read_text()
release = (repo / ".github/workflows/release.yml").read_text()
crate_release = (repo / ".github/workflows/crate-release.yml").read_text()
failures: list[str] = []


def require(condition: bool, message: str) -> None:
    if not condition:
        failures.append(message)


require("types: [publish-crate]" in workflow, "crate workflow must use the singular publish-crate dispatch")
require("package:" in workflow, "crate workflow must require a package input")
require("crate/$DISPATCH_PACKAGE/v$VERSION" in workflow, "crate tags must bind package name and version")
require('cargo publish --locked --no-verify --package "$PACKAGE"' in workflow, "workflow must publish only the selected package")
require("CRATES=(" not in workflow, "workflow must not retain a bulk crate publish list")
# The tag-vs-manifest check must read the version Cargo resolved, not the raw
# manifest TOML. Every published crate inherits the platform version with
# `version.workspace = true`, which parses as {'workspace': True} and can never
# equal a tag, so a raw read fails every publish closed — it halted the 0.28.0
# cascade at its first crate. cargo metadata resolves the inheritance and is the
# same source crate-release.yml tags from, so the two cannot disagree.
require(
    'manifest_version = package["version"]' in workflow,
    "publish workflow must verify the tag against the Cargo-resolved version",
)
require(
    "tomllib" not in workflow,
    "publish workflow must not read the raw manifest version, which is inherited",
)
require("workspace package version" not in workflow.lower(), "workflow must not validate against the product workspace version")
require("publish-crates" not in release and "publish-crate" not in release, "product releases must not dispatch library publishing")
require("run-name: Publish " in workflow, "publish workflow must expose correlated inputs in its run name")
require("correlation:" in workflow, "publish workflow must require a correlation input")
require('--ref "$TAG"' in crate_release, "crate controller must dispatch from the trusted package tag")
require(
    "scripts/select_publish_run.py" in crate_release,
    "crate controller must select the uniquely correlated publish run",
)

metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--no-deps", "--format-version", "1"], text=True
))
published = [package for package in metadata["packages"] if package.get("publish") != []]
for package in published:
    manifest_path = pathlib.Path(package["manifest_path"])
    with manifest_path.open("rb") as handle:
        manifest = tomllib.load(handle)
    require(
        manifest.get("package", {}).get("version") == {"workspace": True},
        f"{manifest_path.relative_to(repo)}: published package must inherit the "
        "platform version with `version.workspace = true`",
    )

# Pin-sync unit coverage. The ard -> host dev pin drifted through a release
# because the sync script skipped dev edges wholesale, so dev-dependencies are
# now discovered too, and a rewrite has to stay inside the table its drift came
# from. The whole-workspace `--check` above covers the real edge; these cases pin
# the discovery and rewrite behaviour behind it.
spec = importlib.util.spec_from_file_location(
    "sync_publish_pin_versions", repo / "scripts/sync-publish-pin-versions.py"
)
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)

packages = [
    {
        "name": "public-owner",
        "manifest_path": "/repo/public-owner/Cargo.toml",
        "publish": None,
        "dependencies": [
            {
                "name": "private-target",
                "path": "/repo/private-target",
                "kind": None,
                "optional": True,
            },
            {
                "name": "private-dev-target",
                "path": "/repo/private-dev-target",
                "kind": "dev",
                "optional": False,
            },
            {
                "name": "registry-restricted-target",
                "path": "/repo/registry-restricted-target",
                "kind": None,
                "optional": False,
            },
            {
                "name": "explicit-crates-io-target",
                "path": "/repo/explicit-crates-io-target",
                "kind": None,
                "optional": False,
            },
            {
                "name": "private-target",
                "path": "/repo/private-target",
                "kind": "build",
                "optional": False,
            },
        ],
    },
    {
        "name": "private-target",
        "manifest_path": "/repo/private-target/Cargo.toml",
        "publish": [],
        "dependencies": [],
    },
    {
        "name": "registry-restricted-target",
        "manifest_path": "/repo/registry-restricted-target/Cargo.toml",
        "publish": ["internal"],
        "dependencies": [],
    },
    {
        "name": "explicit-crates-io-target",
        "manifest_path": "/repo/explicit-crates-io-target/Cargo.toml",
        "publish": ["crates-io"],
        "dependencies": [],
    },
    {
        "name": "private-dev-target",
        "manifest_path": "/repo/private-dev-target/Cargo.toml",
        "publish": [],
        "dependencies": [],
    },
]
require(sync.crates_io_publishable(packages[0]), "default publish target must include crates.io")
require(
    not sync.crates_io_publishable(packages[2]),
    "registry-restricted package must not be treated as crates.io-publishable",
)
require(
    sync.crates_io_publishable(packages[3]),
    "package that explicitly allows crates-io must be treated as publishable",
)
private_failures = sync.private_dependency_failures(packages)
require(
    private_failures
    == [
        "public-owner: private-target is an optional normal path dependency on "
        "non-crates.io workspace package private-target",
        "public-owner: registry-restricted-target is a normal path dependency on "
        "non-crates.io workspace package registry-restricted-target",
        "public-owner: private-target is a build path dependency on non-crates.io "
        "workspace package private-target",
    ],
    "crates.io packages must reject optional normal and build path dependencies "
    "on private or registry-restricted workspace packages while permitting "
    f"dev-only and crates.io edges, got {private_failures}",
)

manifest = tomllib.loads("""
[dependencies]
everruns-core = { path = "../core", version = "1.0.0" }

[dev-dependencies]
everruns-host = { version = "0.20.0", path = "../host", features = ["direct-egress"] }
everruns-llmsim = { path = "../drivers/llmsim" }

[target."cfg(unix)".dev-dependencies]
everruns-sim = { path = "../sim", version = "0.1.0" }
""")
kinds = {
    key: kind
    for kind, table in sync.dependency_tables(manifest)
    for key in table
}
require(
    kinds
    == {
        "everruns-core": "dependencies",
        "everruns-host": "dev-dependencies",
        "everruns-llmsim": "dev-dependencies",
        "everruns-sim": "dev-dependencies",
    },
    f"dependency_tables must report every dependency kind, got {kinds}",
)

with tempfile.TemporaryDirectory() as directory:
    fixture = pathlib.Path(directory) / "Cargo.toml"
    fixture.write_text(
        '[dependencies]\n'
        'everruns-host = { path = "../host" }\n'
        '\n'
        '[dev-dependencies]\n'
        'everruns-host = { version = "0.20.0", path = "../host", features = ["direct-egress"] }\n'
    )
    require(
        sync.rewrite_inline_dependency(fixture, "dev-dependencies", "everruns-host", "0.21.0"),
        "rewriting a drifted dev-dependency pin must report a change",
    )
    rewritten = fixture.read_text()
    require(
        'everruns-host = { version = "0.21.0", path = "../host", features = ["direct-egress"] }'
        in rewritten,
        f"dev-dependency rewrite must keep features and update only the version, got:\n{rewritten}",
    )
    require(
        'everruns-host = { path = "../host" }' in rewritten,
        "a rewrite scoped to [dev-dependencies] must not touch the [dependencies] entry",
    )
    require(
        not sync.rewrite_inline_dependency(fixture, "dev-dependencies", "everruns-host", "0.21.0"),
        "rewriting an already-current pin must report no change",
    )

selector_spec = importlib.util.spec_from_file_location(
    "select_publish_run", repo / "scripts/select_publish_run.py"
)
selector = importlib.util.module_from_spec(selector_spec)
selector_spec.loader.exec_module(selector)
package = "everruns-core"
tag = "crate/everruns-core/v0.24.0"
sha = "a" * 40
correlation = f"100-1-4-{sha}"
intended = {
    "databaseId": 42,
    "displayTitle": selector.expected_title(package, tag, sha, correlation),
    "event": "workflow_dispatch",
    "headSha": sha,
}
runs = [
    {
        "databaseId": 99,
        "displayTitle": selector.expected_title(
            "everruns-host",
            "crate/everruns-host/v0.23.0",
            sha,
            f"manual-{sha}",
        ),
        "event": "workflow_dispatch",
        "headSha": sha,
    },
    intended,
]
require(
    selector.select_run(runs, package, tag, sha, correlation) == 42,
    "newer concurrent unrelated dispatch must not displace the intended run",
)
require(
    selector.select_run(runs, package, tag, "b" * 40, correlation) is None,
    "publish-run selection must reject the wrong release SHA",
)

# The release plan must not dispatch a never-published crate at a platform
# version that was already cut: its pins name that version, but the siblings
# published at it came from the older commit, so cargo's verification build
# compiles it against stale dependency source. Exercised rather than asserted
# textually, because the failure mode is logic, not wording.
import contextlib
import io
import re
import textwrap
import urllib.request

plan_src = textwrap.dedent(
    re.search(r"python3 - <<'PY' > release-plan\.json\n(.*?)\n\s*PY\n", crate_release, re.S).group(1)
)
plan_state: dict = {}


class _FakeResponse:
    def __init__(self, text: str) -> None:
        self._text = text

    def read(self) -> bytes:
        return self._text.encode()

    def __enter__(self):
        return self

    def __exit__(self, *exc) -> bool:
        return False


def _fake_urlopen(url, timeout=None):
    name = plan_state["lookup"][url.rsplit("/", 1)[-1]]
    versions = plan_state["registry"].get(name)
    if not versions:
        raise urllib.error.HTTPError(url, 404, "not published", None, None)
    return _FakeResponse("\n".join(json.dumps({"vers": v}) for v in sorted(versions)))


def release_plan(packages: dict[str, str], registry: dict[str, set[str]]) -> list[str]:
    """Run the workflow's own plan script against a stubbed crates.io index."""
    plan_state["registry"] = registry
    plan_state["lookup"] = {n.lower(): n for n in packages}
    meta = {"packages": [
        {"name": n, "version": v, "dependencies": []} for n, v in packages.items()
    ]}
    original_urlopen = urllib.request.urlopen
    original_check_output = subprocess.check_output
    urllib.request.urlopen = _fake_urlopen
    subprocess.check_output = lambda *a, **k: json.dumps(meta)
    captured = io.StringIO()
    try:
        with contextlib.redirect_stdout(captured):
            exec(compile(plan_src, "release-plan", "exec"), {"__name__": "__main__"})
    finally:
        urllib.request.urlopen = original_urlopen
        subprocess.check_output = original_check_output
    return [entry["package"] for entry in json.loads(captured.getvalue())]


PLATFORM = "0.28.0"
with contextlib.redirect_stderr(io.StringIO()):
    deferred = release_plan(
        {"everruns-core": PLATFORM, "everruns-integrations-typesafe": PLATFORM},
        {"everruns-core": {"0.27.0", PLATFORM}},
    )
    joined = release_plan(
        {"everruns-core": PLATFORM, "everruns-integrations-typesafe": PLATFORM},
        {"everruns-core": {"0.27.0"}},
    )
    resumed = release_plan(
        {"everruns-core": PLATFORM, "everruns-host": PLATFORM},
        {"everruns-core": {"0.27.0", PLATFORM}, "everruns-host": {"0.27.0"}},
    )
require(deferred == [], "a never-published crate must not publish at an already-cut platform version")
require(
    sorted(joined) == ["everruns-core", "everruns-integrations-typesafe"],
    "a new crate must publish with the platform version being cut",
)
require(resumed == ["everruns-host"], "a re-run must still finish a partially published cascade")

legacy_macros = repo / "crates/everruns-macros"
require(not legacy_macros.exists(), "legacy crates/everruns-macros path must not exist")
macros_manifest = repo / "crates/macros/Cargo.toml"
require(macros_manifest.is_file(), "everruns-macros must live at crates/macros/Cargo.toml")

if failures:
    for failure in failures:
        print(f"FAIL: {failure}", file=sys.stderr)
    raise SystemExit(1)

print(f"crate publishing verified for {len(published)} crate(s)")
PY

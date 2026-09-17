#!/usr/bin/env bash
# Guard independent crates.io releases and dependency-version ownership.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_ROOT"

python3 scripts/sync-publish-pin-versions.py --check
python3 scripts/plan-crate-release.py --self-test

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
        isinstance(manifest.get("package", {}).get("version"), str),
        f"{manifest_path.relative_to(repo)}: published package must own an explicit version",
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

legacy_macros = repo / "crates/everruns-macros"
require(not legacy_macros.exists(), "legacy crates/everruns-macros path must not exist")
macros_manifest = repo / "crates/macros/Cargo.toml"
require(macros_manifest.is_file(), "everruns-macros must live at crates/macros/Cargo.toml")

if failures:
    for failure in failures:
        print(f"FAIL: {failure}", file=sys.stderr)
    raise SystemExit(1)

print(f"independent publishing verified for {len(published)} crate(s)")
PY

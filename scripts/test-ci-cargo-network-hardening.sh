#!/usr/bin/env bash
# Keep uncached Cargo jobs resilient to transient crates.io transport failures.

set -euo pipefail

cd "$(dirname "$0")/.."

python3 - <<'PY'
from pathlib import Path

import yaml

workflow = Path(".github/workflows/ci.yml")
env = yaml.safe_load(workflow.read_text()).get("env", {})

if env.get("CARGO_NET_RETRY") != 10:
    raise SystemExit(f"{workflow}: CI must set CARGO_NET_RETRY to 10")
if env.get("CARGO_HTTP_MULTIPLEXING") != "false":
    raise SystemExit(f"{workflow}: CI must disable Cargo HTTP multiplexing")

print("CI Cargo network hardening: dependency fetches retry over HTTP/1.1")
PY

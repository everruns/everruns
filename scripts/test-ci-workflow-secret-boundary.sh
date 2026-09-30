#!/usr/bin/env bash
# Ensure the credentialed workflow server cannot inherit the Doppler vault token.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$(dirname "$SCRIPT_DIR")"

python3 - <<'PY'
import sys
from pathlib import Path

workflow = Path(".github/workflows/ci.yml")
contents = workflow.read_text()
start = contents.find("      - name: Start API server\n")
if start < 0:
    sys.exit(f"{workflow}: workflow-test has no `Start API server` step")

end = contents.find("\n      - name:", start + 1)
command = contents[start:end if end >= 0 else None]
launch = next(
    (line for line in command.splitlines() if "./bin/everruns-server" in line), None
)
if launch is None:
    sys.exit(f"{workflow}: Start API server does not launch everruns-server")

prefix = command[: command.index(launch)]
if "env -u DOPPLER_TOKEN" not in prefix:
    sys.exit(
        f"{workflow}: everruns-server can inherit DOPPLER_TOKEN; launch it through "
        "`env -u DOPPLER_TOKEN`"
    )

print("ci.yml: everruns-server launch removes DOPPLER_TOKEN")
PY

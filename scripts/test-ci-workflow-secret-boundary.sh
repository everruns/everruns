#!/usr/bin/env bash
# Ensure the credentialed workflow server cannot inherit the Doppler vault token.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$(dirname "$SCRIPT_DIR")"

python3 - <<'PY'
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

import yaml

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

# Execute the real launch command with a fake server and vault. Ordinary Rust
# pushes must neither fetch keys nor seed them; paid runs still seed both keys
# without letting the server inherit the vault token.
step = next(
    s for s in yaml.safe_load(contents)["jobs"]["workflow-test"]["steps"]
    if s.get("name") == "Start API server"
)
assert "outputs.provider_live == 'true'" in step["env"]["DOPPLER_TOKEN"]
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    (root / "bin").mkdir()
    server = root / "bin/everruns-server"
    server.write_text('''#!/usr/bin/env python3
import json, os
from pathlib import Path
Path(os.environ["SERVER_CAPTURE"]).write_text(json.dumps({
    "vault": "DOPPLER_TOKEN" in os.environ,
    "openai": bool(os.environ.get("DEFAULT_OPENAI_API_KEY")),
    "anthropic": bool(os.environ.get("DEFAULT_ANTHROPIC_API_KEY")),
}))
''')
    server.chmod(0o755)
    vault = root / "bin/doppler"
    vault.write_text('''#!/usr/bin/env bash
echo "$*" >> "$VAULT_CAPTURE"
echo fixture-key
''')
    vault.chmod(0o755)
    for paid in (False, True):
        capture = root / "server.json"
        calls = root / "vault.calls"
        calls.unlink(missing_ok=True)
        env = dict(os.environ, PATH=f"{root / 'bin'}:{os.environ['PATH']}",
                   RUN_LIVE_TESTS=str(paid).lower(), DOPPLER_TOKEN="fixture-token",
                   SERVER_CAPTURE=str(capture), VAULT_CAPTURE=str(calls))
        subprocess.run(["bash", "-c", step["run"] + "\nwait"], cwd=root,
                       env=env, check=True, capture_output=True, timeout=10)
        assert json.loads(capture.read_text()) == {
            "vault": False, "openai": paid, "anthropic": paid,
        }
        assert (len(calls.read_text().splitlines()) if calls.exists() else 0) == (2 if paid else 0)

print("ci.yml: ordinary workflow runs fetch no keys; paid server drops the vault token")
PY

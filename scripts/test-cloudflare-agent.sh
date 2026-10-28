#!/usr/bin/env bash
# Pure protocol/tool regressions; the live smoke is an explicit deployment task.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
node --test examples/cloudflare-data-agent/test/*.test.js
python3 - <<'PY'
from pathlib import Path
source = Path('examples/cloudflare-data-agent/src/index.js').read_text()
guide = Path('docs/how-to/cloudflare-agent.md').read_text()
assert source in guide, 'Refresh the guide’s inline Agent code from the runnable example'
print('Cloudflare guide inline code matches the runnable example')
PY

#!/usr/bin/env bash
# Pure protocol/tool regressions; the live smoke is an explicit deployment task.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
node --test examples/cloudflare-data-agent/test/*.test.js
python3 -m unittest discover -s examples/cloudflare-data-agent/test -p "*_test.py"
python3 - <<'PY'
from pathlib import Path
guide = Path('docs/how-to/cloudflare-agent.md').read_text()
# The guide shows each agent file in full, one titled block per file.
for name in ('tool.js', 'model.js', 'agent.js', 'index.js'):
    source = Path(f'examples/cloudflare-data-agent/src/{name}').read_text()
    block = f'```javascript title="src/{name}"\n{source}```'
    assert block in guide, f'Refresh the guide’s src/{name} block from the runnable example'
print('Cloudflare guide inline code matches the runnable example')
PY

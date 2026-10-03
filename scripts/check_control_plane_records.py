#!/usr/bin/env python3
"""Keep persistence/API aggregates in the server, including private worker code."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess

# Framework handles own runtime configuration; they are not database/API rows.
RUNTIME_HANDLES = {
    'crates/everruns/src/agent.rs': {'Agent', 'Model'},
    'crates/everruns/src/session.rs': {'Session'},
    'crates/contracts/src/runtime_provider.rs': {'Provider'},
    'crates/everruns/src/harness.rs': {'Harness'},
    'crates/host/src/workspace.rs': {'Workspace'},
    'crates/host/tests/agents_api_support/mod.rs': {'Harness'},
    'crates/serve/src/agent.rs': {'Agent'},
    'crates/serve/src/app.rs': {'App'},
    # This is the external Cursor service's status, not an Everruns agent row.
    'integrations/cursor/src/client.rs': {'AgentStatus'},
}
PUBLIC_DECLARATION = re.compile(r'\bpub(?:\([^)]*\))?\s+(?:struct|enum|type)\s+(\w+)\b')
DECLARATION = re.compile(r'\b(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|type)\s+(\w+)\b')
SERVER_REFERENCE = re.compile(r'\beverruns_server\s*::\s*records\b')
ROW = re.compile(r'\bpub\s+struct\s+(Model|Provider)\b[^;{]*\{([^}]+)\}', re.S)


def violations(root: Path) -> list[str]:
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=root, text=True,
    ))
    members = set(metadata['workspace_members'])
    # The canonical server owner supplies the vocabulary; no second catalog.
    record_names = {
        match[1]
        for source in (root / 'crates/server/src/records').rglob('*.rs')
        for match in PUBLIC_DECLARATION.finditer(source.read_text())
    }
    failures: list[str] = []
    for package in metadata['packages']:
        if package['id'] not in members or package['name'] == 'everruns-server':
            continue
        crate = Path(package['manifest_path']).parent
        if package.get('publish') != []:
            for dependency in package['dependencies']:
                if dependency['name'] == 'everruns-server':
                    failures.append(f"{package['name']}: published crate depends on the server")
        for tree in ('src', 'tests', 'examples'):
            for source in (crate / tree).rglob('*.rs'):
                path = str(source.relative_to(root))
                text = source.read_text()
                for match in DECLARATION.finditer(text):
                    name = match[1]
                    if name in record_names and name not in RUNTIME_HANDLES.get(path, set()):
                        line = text.count('\n', 0, match.start()) + 1
                        failures.append(f'{path}:{line}: control-plane {name} must be server-owned')
                if SERVER_REFERENCE.search(text):
                    failures.append(f'{path}: references server records')
                for match in ROW.finditer(text):
                    fields = match[2]
                    if 'pub created_at:' in fields and (
                        'pub provider_id:' in fields or 'pub api_key_set:' in fields
                    ):
                        failures.append(f'{path}: persisted {match[1]} row outside the server')
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    failures = violations(args.root.resolve())
    if failures:
        print('\n'.join(failures))
        return 1
    print('Control-plane record guard passed: only the server owns persisted aggregates.')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())

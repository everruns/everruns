#!/usr/bin/env python3
"""Affected workspace tests, with reverse dependencies and no credentialed features."""
import argparse
import json
import os
from pathlib import Path
import subprocess

import libtest

# These are the existing offline CI configurations. Live-test features remain
# opt-in in their credentialed workflows; --all-features is unsafe for them.
FEATURES = {
    'everruns-core': ['all'], 'everruns-contracts': ['all'],
    'everruns-drivers': ['all'], 'everruns-llmsim': ['all'],
    'everruns': ['all'], 'everruns-docs-snippets': ['all'],
    'everruns-test-support': ['all'],
    'everruns-capabilities': ['hosted-integration-catalog', 'environment-capabilities', 'portable-builtins'],
    'everruns-integrations': ['bashkit', 'brave-search', 'brave-search-hosted', 'browserless',
        'cursor', 'daytona', 'daytona-test-util', 'docker', 'duckduckgo', 'e2b', 'filesystem',
        'github', 'lua', 'openai-decisions', 'openai-image', 'openrouter', 'parallel',
        'typesafe', 'typesafe-hosted', 'web-fetch', 'modal', 'modal-test-util'],
    'everruns-integrations-experimental': ['sprites'],
}


def affected(packages, paths, root):
    owners = {p['name']: Path(p['manifest_path']).parent.relative_to(root).as_posix() + '/' for p in packages}
    changed = set()
    for path in paths:
        if path.endswith(('.md', '.mdx')):
            continue
        if path in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml') or path.startswith('.github/scripts/ci/') or path == '.github/workflows/ci.yml':
            return set(owners)
        matches = [(len(folder), name) for name, folder in owners.items() if path.startswith(folder)]
        if matches:
            changed.add(max(matches)[1])
        if path.startswith('skills/'):
            changed.add('everruns-server')
    # Include dev dependencies too: downstream tests exercise contracts that
    # normal compilation alone cannot establish.
    while True:
        prior = set(changed)
        changed.update(p['name'] for p in packages if any(d['name'] in changed for d in p['dependencies']))
        if prior == changed:
            return changed


def risk_filters(paths):
    filters = set()
    if any(p.startswith(('crates/server/src/domains/change_history/',
                         'crates/server/src/storage/entity_changes',
                         'crates/server/src/storage/manager_context',
                         'crates/server/src/storage/repositories/entity_changes',
                         'crates/server/src/storage/repositories/manager_context'))
           or p in ('crates/server/src/api/history.rs',
                    'crates/server/src/api/manager_context.rs',
                    'crates/server/src/domains/common/mod.rs',
                    'crates/server/src/storage/transaction.rs',
                    'crates/server/tests/domain/change_history_test.rs',
                    '.github/scripts/ci/focused.py') for p in paths):
        # History/context writes must retain their baseline and revision guards
        # across the real REST and command adapters, not just unit fixtures.
        filters.add('change_history_test::')
    if any(p in (
            'crates/server/src/api/agent_api_idempotency.rs',
            'crates/server/src/api/command_dispatch.rs',
            'crates/server/src/storage/transaction.rs',
            'crates/server/src/storage/repositories/command_idempotency.rs',
            'crates/server/tests/domain/agent_api_retry_cors_test.rs',
            'crates/server/tests/domain/command_dispatch_test.rs',
            '.github/scripts/ci/focused.py') for p in paths):
        # Admission, effect and replay must survive cancellation together on
        # the actual Agent API and Console command adapters.
        filters.update(['agent_api_retry_cors_test::', 'command_dispatch_test::'])
    if any(p.startswith('crates/server/src/domains/knowledge/')
           or p in ('crates/server/src/api/knowledge.rs',
                    'crates/server/src/api/agent_api/work_chat.rs',
                    'crates/server/tests/domain/work_chat_test.rs') for p in paths):
        # Agent Knowledge mounts remain private to runtime views; a consumer's
        # workspace must not expose them through normalized workspace aliases.
        filters.add('work_chat_test::')
    if any(p.startswith(('crates/server/src/domains/agents/credentials.rs',
                         'crates/server/src/domains/credentials/',
                         'crates/server/src/domains/resource_access/',
                         'crates/server/src/domains/mcp_servers/',
                         'crates/server/src/worker_link/'))
           or p == 'crates/server/tests/domain/mcp_acts_as_grpc_test.rs' for p in paths):
        # Credential parameter lookup must not preempt native OAuth's identity
        # and connection-required behavior over the actual worker transport.
        filters.add('mcp_acts_as_grpc_test::')
    if any(p.startswith(('crates/server/src/api/', 'crates/server/src/openapi/')) for p in paths):
        # Reuse the domain invocation: API shape and prose are public contracts.
        filters.update(['openapi_coverage_test::', 'openapi_descriptions_test::', 'pat_scope_coverage_test::'])
    if any('/migrations/' in p or '/setup/migrations' in p for p in paths):
        filters.update(['migration_baseline_test::', 'migration_history_test::'])
    if '.github/scripts/ci/focused.py' in paths or any(any(x in p for x in ('/auth', '/permissions', '/policy', '/org_', '/organization', '/principal', '/roles', '/members', '/pat_', '/storage/repositories/', '/runtime_identity/')) for p in paths):
        filters.update(['auth_integration_test::', 'org_isolation_test::', 'command_policy_enforcement_test::', 'pat_scope_coverage_test::'])
    if any(p.endswith('/tests/test_harness.rs') or '/tests/test_harness/' in p
           or p.endswith('/storage/test_database.rs') for p in paths):
        filters.update(['auth_integration_test::', 'org_isolation_test::',
                        'db_pool_isolation_test::', 'service_mcp_oauth_lifecycle_test::',
                        'migration_baseline_test::', 'migration_history_test::'])
    if 'crates/server/tests/domain/db_pool_isolation_test.rs' in paths:
        filters.add('db_pool_isolation_test::')
    return sorted(filters)


def real_server_targets(paths):
    # These production-router tests configure their own process environment and
    # bypass TestServer; backend changes must exercise their database setup too.
    targets = ['background_sweep_wiring_test', 'pat_scope_sweep_test']
    if any(p.startswith(('crates/server/tests/test_harness',
                         'crates/server/src/storage/embedded_postgres/'))
           or p in ('crates/server/src/storage/test_database.rs',
                    'crates/server/src/setup/storage_init.rs') for p in paths):
        return targets
    selected = {target for target in targets if f'crates/server/tests/{target}.rs' in paths}
    if any(p.startswith(('crates/server/src/api/',
                         'crates/server/src/domains/',
                         'crates/server/src/app_builder/',
                         'crates/server-macros/'))
           or p == '.github/scripts/ci/focused.py' for p in paths):
        # OpenAPI command declarations do not mount handlers. Exercise the
        # production router and its token scopes before merging route changes.
        selected.add('pat_scope_sweep_test')
    return [target for target in targets if target in selected]


def select(metadata, paths, root):
    packages = [p for p in metadata['packages'] if p['id'] in metadata['workspace_members']]
    names = affected(packages, paths, root)
    groups = {}
    for package in packages:
        name = package['name']
        if name not in names or name == 'everruns-llm-tests':
            continue
        targets = package['targets']
        # Binary-only examples have real-model entry points. Their unit tests
        # are offline, and cargo test never runs main().
        if not any('lib' in t['kind'] or 'proc-macro' in t['kind'] or 'bin' in t['kind'] for t in targets):
            continue
        group = 'server' if name == 'everruns-server' else 'facade' if name in ('everruns', 'everruns-cli', 'everruns-docs-snippets') else 'runtime' if name in ('everruns-core', 'everruns-capabilities', 'everruns-worker', 'everruns-integrations', 'everruns-integrations-experimental', 'everruns-test-support') else 'foundation'
        groups.setdefault(group, []).append(name)
    return {'include': [{'group': group, 'packages': sorted(names),
                         'risk_filters': risk_filters(paths) if group == 'server' else [],
                         'risk_targets': real_server_targets(paths) if group == 'server' else []}
                        for group, names in sorted(groups.items())]}


def commands(selection):
    packages = selection['packages']
    command = ['cargo', 'test', '--locked']
    for name in packages:
        command += ['-p', name]
    if packages == ['everruns-server']:
        command += ['--lib', '--test', 'contracts']
        for target in selection.get('risk_targets', []):
            command += ['--test', target]
        if selection.get('risk_filters'):
            command += ['--test', 'domain']
        # Build the whole selected target graph together. Running cargo again
        # for domain can rebuild the server's shared dependencies/codegen.
        command += ['--no-run', '--message-format=json']
    features = []
    for name in packages:
        if FEATURES.get(name) == ['all']:
            # Enumerate this package's features rather than enabling every
            # feature of every other package in the same invocation.
            metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], text=True))
            package = next(p for p in metadata['packages'] if p['name'] == name)
            features += [f'{name}/{f}' for f in package['features']]
        else:
            features += [f'{name}/{f}' for f in FEATURES.get(name, [])]
    if features:
        command += ['--features', ','.join(features)]
    # Retain serialization for process-global env/host fixtures. Parallel
    # private-database unit tests in the server retain their existing default.
    if packages != ['everruns-server']:
        command += ['--', '--test-threads=1']
    yield command


def run_server(selection, command):
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--no-deps', '--format-version', '1'], text=True))
    package = next(p for p in metadata['packages'] if p['name'] == 'everruns-server')
    manifest = Path(package['manifest_path']).resolve()
    library = next(t['name'] for t in package['targets'] if 'lib' in t['kind'])
    targets = {(library, 'lib'): [], ('contracts', 'test'): []}
    targets.update({(target, 'test'): [] for target in selection.get('risk_targets', [])})
    if selection.get('risk_filters'):
        targets[('domain', 'test')] = [*selection['risk_filters'], '--test-threads=1']
    executables = libtest.compile_tests(command, package, targets)
    libtest.execute_tests(manifest, executables, targets)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--base')
    parser.add_argument('--run', action='store_true')
    args = parser.parse_args()
    if args.run:
        selection = json.loads(os.environ['TEST_SELECTION'])
        for command in commands(selection):
            print(' '.join(command), flush=True)
            if selection['packages'] == ['everruns-server']:
                run_server(selection, command)
            else:
                subprocess.run(command, check=True)
        return
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], text=True))
    root = Path(metadata['workspace_root'])
    paths = subprocess.check_output(['git', 'diff', '--name-only', args.base, 'HEAD'], text=True).splitlines()
    matrix = select(metadata, paths, root)
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
        print('matrix=' + json.dumps(matrix, separators=(',', ':')), file=output)
        print('has_tests=' + str(bool(matrix['include'])).lower(), file=output)
    print(json.dumps(matrix, indent=2))


if __name__ == '__main__':
    main()

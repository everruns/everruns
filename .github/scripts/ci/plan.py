#!/usr/bin/env python3
"""Select a trustworthy comparison base and validation tier for this CI run."""
import json
import os
from pathlib import Path
import subprocess


def gh_json(endpoint):
    return json.loads(subprocess.check_output(['gh', 'api', endpoint], text=True))


def successful_base(runs, revision, is_ancestor):
    # No PR, label-only, cancelled, or still-running run can advance the batch.
    for run in sorted(runs, key=lambda r: r['id'], reverse=True):
        if (run['event'] == 'push' and run['conclusion'] == 'success'
                and run['head_branch'] == 'main' and run['head_sha'] != revision
                and run.get('display_title', '').startswith(('CI batch ', 'CI full '))
                and is_ancestor(run['head_sha'], revision)):
            return run['head_sha']
    return ''


def mode(event, full, subject):
    if event == 'pull_request':
        return 'pr'
    if full or subject.casefold().startswith('chore(release): prepare v'):
        return 'full'
    return 'nightly' if event == 'schedule' else 'batch'


def main():
    event = os.environ['GITHUB_EVENT_NAME']
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
    subject = subprocess.check_output(['git', 'log', '-1', '--format=%s'], text=True).strip()
    tier = mode(event, os.environ.get('FULL_VALIDATION') == 'true', subject)
    repository = os.environ['GITHUB_REPOSITORY']
    base = ''
    if tier == 'pr':
        payload = json.loads(Path(os.environ['GITHUB_EVENT_PATH']).read_text())
        base = payload['pull_request']['base']['sha']
    elif tier == 'batch':
        # Only successful push runs are candidates. If this bounded history
        # has no compatible ancestor, full validation is the safe fallback.
        runs = gh_json(f'repos/{repository}/actions/workflows/ci.yml/runs?branch=main&event=push&status=success&per_page=100')['workflow_runs']
        base = successful_base(runs, revision, lambda a, b: subprocess.run(
            ['git', 'merge-base', '--is-ancestor', a, b], check=False).returncode == 0)
    # No baseline: compare from the empty tree and run every surface.
    force = tier == 'full' or (tier == 'batch' and not base)
    if tier == 'nightly':
        history = gh_json(f'repos/{repository}/actions/workflows/ci.yml/runs?branch=main&event=schedule&per_page=100')['workflow_runs']
        prior = [r for r in history if str(r['id']) != os.environ['GITHUB_RUN_ID']
                 and r.get('display_title', '').startswith('CI nightly ')]
        # A failed attempt is still an attempt: retry unchanged code only by
        # explicit dispatch. Dynamic provider/advisory checks remain scheduled.
        latest = max(prior, key=lambda r:r['id'], default=None)
        force = latest is None or latest['head_sha'] != revision
    if not base:
        base = subprocess.check_output(['git', 'hash-object', '-w', '-t', 'tree', '/dev/null'], text=True).strip()
    outputs = {'mode': tier, 'revision': revision, 'base': base, 'force': str(force).lower()}
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
        for key, value in outputs.items():
            print(f'{key}={value}', file=output)
    summary = f'Validation: {tier}; comparison {base} → {revision}; full surface scan: {force}'
    print(summary)
    with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as output:
        print(summary, file=output)


if __name__ == '__main__':
    main()

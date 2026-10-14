#!/usr/bin/env python3
"""Release gate: wait for full CI on this exact trusted main revision."""
import argparse
import json
import os
import re
import subprocess
import time


def latest_full(runs, revision):
    candidates = [r for r in runs if r.get('display_title') == f'CI full {revision}'
                  and r['event'] in ('push', 'workflow_dispatch') and r['head_branch'] == 'main'
                  and r.get('path') == '.github/workflows/ci.yml']
    return max(candidates, key=lambda r: r['id'], default=None)


def api(endpoint):
    return json.loads(subprocess.check_output(['gh', 'api', endpoint], text=True))


def find_full(repository, revision, read=api):
    root = f'repos/{repository}/actions/workflows/ci.yml/runs'
    runs = read(f'{root}?branch=main&event=push&head_sha={revision}&per_page=100')['workflow_runs']
    # Normal releases have a matching push run. Only manual runs newer than
    # the selected candidate can supersede it, so do not scan years of pushes.
    for page in range(1, 11):
        manual = read(f'{root}?branch=main&event=workflow_dispatch&per_page=100&page={page}')['workflow_runs']
        runs += manual
        candidate = latest_full(runs, revision)
        if len(manual) < 100 or (candidate and candidate['id'] >= min(r['id'] for r in manual)):
            return candidate
    raise SystemExit('Validation history is too old; explicitly run fresh full CI for this revision')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--sha', default=os.environ.get('GITHUB_SHA'))
    parser.add_argument('--timeout', type=int, default=5400)
    args = parser.parse_args()
    if not args.sha or not re.fullmatch('[0-9a-f]{40}', args.sha):
        raise SystemExit('Full validation requires an exact commit SHA')
    subprocess.run(['git', 'merge-base', '--is-ancestor', args.sha, 'origin/main'], check=True)
    repository = os.environ['GITHUB_REPOSITORY']
    deadline = time.monotonic() + args.timeout
    dispatched = False
    run = None
    while time.monotonic() < deadline:
        if run is None:
            run = find_full(repository, args.sha)
        else:
            run = api(f'repos/{repository}/actions/runs/{run["id"]}')
        if run:
            print(f'Full validation: {run["html_url"]} ({run["status"]}, {run["conclusion"]})', flush=True)
            if run['status'] == 'completed':
                # Recheck once at completion so a later explicit full attempt
                # cannot be hidden behind an older green (or failed) run.
                run = find_full(repository, args.sha) or run
                if run['status'] == 'completed':
                    if run['conclusion'] != 'success':
                        raise SystemExit('Exact-revision full validation did not pass; inspect and fix before releasing')
                    return
        elif not dispatched:
            # The trusted workflow validates this exact main ancestor even if
            # the main ref advances before the manual run begins.
            subprocess.run(['gh', 'workflow', 'run', 'ci.yml', '--repo', repository,
                            '--ref', 'main', '-f', 'full=true', '-f', f'revision={args.sha}'], check=True)
            dispatched = True
        time.sleep(30)
    raise SystemExit('Timed out waiting for full validation; no release permitted')


if __name__ == '__main__':
    main()

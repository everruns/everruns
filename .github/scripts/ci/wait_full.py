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
    while time.monotonic() < deadline:
        pages = json.loads(subprocess.check_output([
            'gh', 'api', '--paginate', '--slurp',
            f'repos/{repository}/actions/workflows/ci.yml/runs?branch=main&per_page=100'
        ], text=True))
        run = latest_full([r for page in pages for r in page['workflow_runs']], args.sha)
        if run:
            print(f'Full validation: {run["html_url"]} ({run["status"]}, {run["conclusion"]})', flush=True)
            if run['status'] == 'completed':
                if run['conclusion'] != 'success':
                    raise SystemExit('Exact-revision full validation did not pass; inspect and fix before releasing')
                return
        elif not dispatched:
            # workflow_dispatch runs the trusted workflow from main; its jobs
            # all check out and attest this exact ancestor, even as main moves.
            subprocess.run(['gh', 'workflow', 'run', 'ci.yml', '--repo', repository,
                            '--ref', 'main', '-f', 'full=true', '-f', f'revision={args.sha}'], check=True)
            dispatched = True
        time.sleep(30)
    raise SystemExit('Timed out waiting for full validation; no release permitted')


if __name__ == '__main__':
    main()

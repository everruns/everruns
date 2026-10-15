#!/usr/bin/env python3
"""Delete only caches under merge refs belonging to currently closed PRs."""
import json
import os
from pathlib import Path
import re
import subprocess


def api(endpoint, method='GET'):
    command = ['gh', 'api', '--method', method, endpoint]
    if method == 'GET':
        command += ['--paginate', '--slurp']
        return json.loads(subprocess.check_output(command, text=True))
    subprocess.run(command, check=True)


def cleanup(repository, number=None, request=api):
    pages = request(f'repos/{repository}/actions/caches?per_page=100')
    groups = {}
    for page in pages:
        for cache in page['actions_caches']:
            match = re.fullmatch(r'refs/pull/([1-9][0-9]*)/merge', cache['ref'])
            if match and (number is None or int(match[1]) == number):
                groups.setdefault(int(match[1]), []).append(cache)
    deleted_bytes = deleted = 0
    for pr, caches in groups.items():
        # Re-read state immediately before each deletion: a closed event may
        # arrive after a reopen. Never use the event payload as deletion proof.
        for cache in caches:
            pull = request(f'repos/{repository}/pulls/{pr}')[0]
            if (pull['state'] != 'closed' or pull['number'] != pr
                    or pull['base']['repo']['full_name'] != repository):
                break
            if type(cache['id']) is not int or cache['id'] <= 0:
                raise ValueError('Invalid cache identity')
            request(f'repos/{repository}/actions/caches/{cache["id"]}', 'DELETE')
            deleted += 1
            deleted_bytes += cache['size_in_bytes']
            print(f'Deleted closed PR #{pr} cache {cache["id"]}: {cache["size_in_bytes"]} bytes', flush=True)
    print(f'Deleted {deleted} caches / {deleted_bytes} bytes', flush=True)
    return deleted, deleted_bytes


def main():
    repository = os.environ['GITHUB_REPOSITORY']
    event = json.loads(Path(os.environ['GITHUB_EVENT_PATH']).read_text())
    number = None
    if os.environ['GITHUB_EVENT_NAME'] == 'pull_request_target':
        pull = event['pull_request']
        if (event['action'] != 'closed' or pull['base']['repo']['full_name'] != repository
                or pull['base']['ref'] != 'main' or type(event['number']) is not int or event['number'] <= 0):
            raise RuntimeError('Refusing an unexpected PR cleanup event')
        number = event['number']
    elif not (os.environ['GITHUB_EVENT_NAME'] == 'workflow_dispatch'
              and os.environ['GITHUB_REF'] == 'refs/heads/main'):
        raise RuntimeError('Manual cleanup requires trusted main')
    cleanup(repository, number)


if __name__ == '__main__':
    main()

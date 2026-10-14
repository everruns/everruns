#!/usr/bin/env python3
"""Reconcile trusted main CI failures into owned, deduplicated Linear issues."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import urllib.request
import urllib.error


def gh(endpoint):
    return json.loads(subprocess.check_output(['gh', 'api', endpoint], text=True))


def linear(query, variables=None):
    request = urllib.request.Request('https://api.linear.app/graphql',
        data=json.dumps({'query': query, 'variables': variables or {}}).encode(),
        headers={'Authorization': os.environ['LINEAR_API_KEY'], 'Content-Type': 'application/json'})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            result = json.load(response)
    except urllib.error.HTTPError as error:
        try:
            messages = [e['message'] for e in json.load(error).get('errors', [])]
        except (ValueError, KeyError):
            messages = []
        raise RuntimeError(f'Linear HTTP {error.code}: {messages}') from None
    if result.get('errors'):
        raise RuntimeError('Linear rejected CI reconciliation: ' + json.dumps(result['errors']))
    data = result['data']
    if any(isinstance(value, dict) and value.get('success') is False for value in data.values()):
        raise RuntimeError('Linear mutation did not apply')
    return data


def check_key(name):
    # SDK versions change, but the language/cohort remains the same check.
    stable = re.sub(r' / v[0-9][^)]*', ' / version', name)
    return hashlib.sha256(stable.encode()).hexdigest()[:20]


def state_from(description):
    match = re.search(r'<!-- ci-state:(.*?) -->', description or '')
    return json.loads(match[1]) if match else None


def decision(issue_state, run, check, contains):
    if issue_state and int(issue_state['run_id']) > run['id']:
        return 'ignore'
    if check['conclusion'] in ('failure', 'timed_out', 'action_required'):
        return 'failure'
    if issue_state and issue_state.get('resolved') and check['conclusion'] == 'success':
        return 'ignore'
    if (issue_state and check['conclusion'] == 'success' and run['conclusion'] == 'success'
            and issue_state['failed_sha'] != run['revision']
            and contains(issue_state['failed_sha'], run['revision'])):
        return 'resolved'
    return 'ignore'


def replace_report(description, report):
    start, end = '<!-- ci-report-start -->', '<!-- ci-report-end -->'
    block = f'{start}\n{report}\n{end}'
    if start in description and end in description:
        return re.sub(re.escape(start) + r'.*?' + re.escape(end), lambda _: block, description, flags=re.S)
    return (description + '\n\n' + block).strip()


def main():
    event = json.loads(Path(os.environ['GITHUB_EVENT_PATH']).read_text())
    run = event['workflow_run']
    repository = os.environ['GITHUB_REPOSITORY']
    if (run['head_repository']['full_name'] != repository or run['head_branch'] != 'main'
            or run['event'] not in ('push', 'schedule', 'workflow_dispatch')
            or run['path'] != '.github/workflows/ci.yml'):
        raise SystemExit('Refusing to reconcile an untrusted CI run')
    title = re.fullmatch(r'CI (batch|nightly|full) ([0-9a-f]{40})', run['display_title'])
    if not title:
        raise SystemExit('Run has no validation identity')
    run['revision'] = title[2]
    jobs = subprocess.check_output(['gh', 'api', '--paginate', '--slurp',
        f'repos/{repository}/actions/runs/{run["id"]}/jobs?per_page=100'], text=True)
    checks = [j for page in json.loads(jobs) for j in page['jobs']
              if j['name'] != 'Build Check']
    context = linear('''query {
      teams(first:1, filter: {key: {eq: "EVE"}}) { nodes { id
        states { nodes { id type name } }
        projects(first:100) { nodes { id name } }
        members(first:100) { nodes { id email } }
      } }
    }''')['teams']['nodes']
    if len(context) != 1:
        raise RuntimeError('Expected the EVE team')
    team = context[0]
    project = next(p['id'] for p in team['projects']['nodes'] if p['name'] == 'OSS')
    owner = next(u['id'] for u in team['members']['nodes'] if u['email'] == 'mike@chaliy.name')
    todo = next(s['id'] for s in team['states']['nodes'] if s['type'] == 'unstarted')
    done = next(s['id'] for s in team['states']['nodes'] if s['type'] == 'completed')
    issues, cursor = [], None
    while True:
        page = linear('''query($cursor:String) {
          issues(first:100, after:$cursor, filter:{team:{key:{eq:"EVE"}}, description:{contains:"<!-- ci-state:"}}) {
            nodes { id description state { type } assignee { id name } }
            pageInfo { hasNextPage endCursor }
          }
        }''', {'cursor': cursor})['issues']
        issues += page['nodes']
        if not page['pageInfo']['hasNextPage']:
            break
        cursor = page['pageInfo']['endCursor']
    by_key = {}
    for issue in issues:
        state = state_from(issue['description'])
        if state and (state['check'] not in by_key or state['run_id'] > state_from(by_key[state['check']]['description'])['run_id']):
            by_key[state['check']] = issue

    def contains(older, newer):
        comparison = gh(f'repos/{repository}/compare/{older}...{newer}')
        return comparison['status'] in ('ahead', 'identical')

    if not any(decision(state_from(by_key[check_key(c['name'])]['description']) if check_key(c['name']) in by_key else None, run, c, contains) != 'ignore' for c in checks):
        print('No actionable failures or recoveries')
        return

    # Only a successful earlier push batch is a regression baseline. Nightly
    # failures and cancelled/coalesced attempts never advance it.
    history = gh(f'repos/{repository}/actions/workflows/ci.yml/runs?branch=main&event=push&status=success&per_page=100')['workflow_runs']
    earlier = [r for r in history if r['id'] < run['id'] and r['head_sha'] != run['revision']
               and r.get('display_title', '').startswith(('CI batch ', 'CI full '))]
    earlier.sort(key=lambda r:r['id'], reverse=True)
    baseline = next((r['head_sha'] for r in earlier if contains(r['head_sha'],run['revision'])), '')
    candidates = []
    if baseline:
        comparison = gh(f'repos/{repository}/compare/{baseline}...{run["revision"]}?per_page=100')
        for commit in comparison['commits']:
            for pr in gh(f'repos/{repository}/commits/{commit["sha"]}/pulls?per_page=100'):
                if pr['merged_at']:
                    candidates.append(f'- [#{pr["number"]}: {pr["title"]}]({pr["html_url"]})')
    for check in checks:
        key = check_key(check['name'])
        issue = by_key.get(key)
        previous = state_from(issue['description']) if issue else None
        action = decision(previous, run, check, contains)
        if action == 'ignore':
            continue
        state = {'check': key, 'run_id': run['id'], 'failed_sha': run['revision'] if action == 'failure' else previous['failed_sha'], 'resolved': action == 'resolved'}
        failed_steps = ', '.join(s['name'] for s in check['steps'] if s['conclusion'] in ('failure','timed_out'))
        owner_name = issue['assignee']['name'] if issue and issue.get('assignee') else 'Mykhailo Chalyi'
        report = f'''<!-- ci-state:{json.dumps(state, separators=(',', ':'))} -->
Check: {check['name']}
State: {action}. Owner: {owner_name}.
[Run]({run['html_url']}) · [Check]({check['html_url']})
Failed steps: {failed_steps or 'none'}
Last successful batch: `{baseline or 'no baseline'}`
Current revision: `{run['revision']}`

Candidate PRs in the range (suspects, not attribution):
{chr(10).join(dict.fromkeys(candidates)) or '- No PR range available.'}

{'A later green batch contains the failing revision; verification complete.' if action == 'resolved' else 'Investigate the failed check; record cause and fix outside this generated block. A same-revision green rerun still needs human infrastructure triage.'}'''
        description = replace_report(issue['description'] if issue else '', report)
        if issue:
            linear('''mutation($id:String!, $input:IssueUpdateInput!) {
                issueUpdate(id:$id,input:$input) { success }
            }''', {'id': issue['id'], 'input': {'description': description, **({'assigneeId':owner} if not issue.get('assignee') else {}), **({'stateId':done} if action == 'resolved' else {'stateId':todo} if issue['state']['type'] in ('completed','canceled') else {})}})
        else:
            linear('''mutation($input:IssueCreateInput!) { issueCreate(input:$input) { success issue { identifier } } }''',
                {'input': {'teamId':team['id'], 'projectId':project, 'assigneeId':owner, 'stateId':todo,
                           'title':'CI batch: '+check['name'], 'description':description, 'priority':2}})
        print(f'{action}: {check["name"]}')


if __name__ == '__main__':
    main()

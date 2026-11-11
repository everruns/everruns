---
title: Repositories
description: Work on several GitHub repositories in one sandbox, with one work branch per session and no GitHub credential kept in the sandbox.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `repositories` |
| **Category** | Integrations |
| **Features** | `repositories` (enables the Changes tab) |
| **Dependencies** | None |

The Repositories capability gives a session a set of GitHub repositories to
work on. Each one is checked out in its own directory under `/workspace`, so a
change that touches a service and the library it uses is one session, not two.
One repository is just a set with one entry.

It needs a full sandbox (a [Sandbox Worker](/built-ins/harnesses/sandbox-worker/)
agent). The [Coding](/built-ins/) harness example includes it.

## Tools

| Tool | What it does |
|---|---|
| `repo_status` | Checks out the session's repositories that are not in `/workspace` yet, then reports every repository there: path, access, branch, uncommitted files, commits ahead of the base branch and commits not pushed. |
| `repo_add` | Checks out another repository next to the ones already there. Not offered when `add_repositories` is `off`. |
| `repo_sync` | Fetches a branch (the base branch by default) into `origin/<branch>` and reports how far the checkout is ahead and behind. It merges nothing. |
| `repo_push` | Pushes the session's work branch of one writable repository. Never forces. Only offered when something is writable. |

Commits, diffs, merges and rebases are ordinary git in the shell. Open the pull
request with `create_github_pull_request` from the
[GitHub](/capabilities/github/) capability.

## Configuration

```json
{
  "repositories": [
    { "repo": "acme/app", "access": "write" },
    { "repo": "acme/lib", "ref": "release/2.x" }
  ],
  "add_repositories": "read"
}
```

| Setting | Default | Meaning |
|---|---|---|
| `repositories` | none | The repositories to check out. `access` is `read` (default) or `write`; `ref` is the branch to start from, the default branch when omitted. Up to 20. |
| `add_repositories` | `read` | What the agent may do with a repository it adds itself: `off` (it cannot add any), `read`, or `write`. |
| `branch_prefix` | `everruns/` | Start of the work branch name. The session's short id is appended. |
| `depth` | `200` | Commits of history to fetch. `repo_sync` can fetch more later. |
| `commit_name`, `commit_email` | `Everruns Agent`, `agent@everruns.invalid` | Author of the agent's commits when the sandbox has no git identity. Set the email to your GitHub App's `<id>+<slug>[bot]@users.noreply.github.com` address to show the bot as author. |
| `acts_as` | `agent` | Whose GitHub identity clones and pushes. See [Authentication](#authentication). |

Set an agent's usual repositories on the agent. To start one session on a
different set, pass the capability with its own config when you create the
session:

```json
{
  "agent_id": "agent_...",
  "capabilities": [
    {
      "ref": "repositories",
      "config": { "repositories": [{ "repo": "acme/docs", "access": "write" }] }
    }
  ]
}
```

A session's config replaces the agent's for that session.

## In the console

An agent's **Repositories** settings list its usual repositories with read or
write access each, choose what the agent may add on its own, and set the work
branch prefix. The repositories offered are the ones the agent's GitHub App
installation can reach.

Starting a chat with such an agent shows the same list. Remove a repository,
turn write access down to read, or add another one for that chat alone. The
form offers no more than the agent's settings allow: a repository the agent
only reads stays read-only, and nothing can be added when `add_repositories` is
`off`.

A session's **Changes** tab shows each repository as the sandbox has it right
now: its branch, commits ahead of the base branch, files not committed, whether
the work is pushed, the changed files and the diff against the base branch, and
a link to the pull request once the agent opened one. It reads from the running
sandbox and never starts one; when the sandbox is stopped, pushed work is still
on GitHub. The same data is at `GET /v1/sessions/{session_id}/repositories`.
The tab has no merge, push or discard buttons: the agent acts, you review.

## Layout and branches

A repository is checked out at `/workspace/<name>`. When two repositories share
a name, the second goes to `/workspace/<owner>-<name>`. `/workspace` itself is
not a repository.

Writable repositories start on the session's work branch, `everruns/<short id>`
by default, with the same name in every repository. Read-only repositories stay
on their starting branch. `repo_push` pushes the work branch and nothing else,
so a change across repositories ends as one pull request per repository, all
from branches with the same name.

The first time a repository is checked out, its `AGENTS.md` comes back with the
result, so the agent reads each repository's own rules before changing it.

## Authentication

The sandbox never keeps a GitHub credential. Each checkout, fetch and push gets
its own short-lived token from the agent's GitHub App installation:

- checkout and fetch: **Contents: read**, limited to the repositories being
  fetched;
- push: **Contents: write**, limited to the one repository being pushed.

The token is handed to the sandbox for that one git command and removed when it
ends. `git push` or `git fetch` typed into the shell has nothing to
authenticate with, so private code only moves through these tools, and only to
the repositories the configuration allows.

This works with a GitHub App: the agent's own, or the shared Everruns app an org
admin linked. A person's own GitHub sign-in cannot be narrowed to a repository
set, so it is never handed to a sandbox; with `acts_as: person` the tools report
that an App installation is needed.

Pushing needs the App's **Contents: write** permission. An agent's own App asks
for it when the agent is connected to GitHub after `repositories` was given a
writable repository (or `add_repositories: write`); every other agent's App
stays read-only. An App created earlier keeps its old permissions: the agent's
GitHub page lists `contents:write` as missing until the App's owner raises it on
GitHub and the installation accepts.

Access is decided by the configuration, by repository name. Pointing a
checkout's `origin` somewhere else does not change where `repo_push` pushes or
which repository its token is for.

While a git command runs, other code in the same sandbox can read that
command's token. For a push that is a token that can write to one repository's
contents for up to an hour. Give `write` access only to repositories where a
pushed branch is reviewed before it merges.

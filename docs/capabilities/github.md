---
title: GitHub
description: Read and review GitHub pull requests with inline comments, keep one comment per pull request up to date, and file deduplicated issues.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `github` |
| **Category** | Integrations |
| **Features** | None |
| **Dependencies** | None |

The GitHub capability gives an agent what it needs to work on one pull request:
read it, read its diff, review it with inline comments, and leave a comment it
can keep editing. It can also file findings as issues without duplicating them.
Pair it with a GitHub trigger to build a pull request reviewer, or with a
schedule trigger and a sandbox to build a security scanner. The
[PR Reviewer and Security Scanner templates](/how-to/set-up-review-and-security-agents/)
do both for you.

## Tools

| Tool | What it does |
|---|---|
| `get_github_pull_request` | Title, description, author, state, branches, size, and up to 100 changed files. |
| `get_github_pull_request_diff` | The unified diff, cut to `max_bytes` (default 60,000, at most 200,000). `truncated` says whether it was cut. |
| `upsert_github_comment` | Posts a Markdown comment, or edits the one it posted earlier with the same `marker` (default `summary`). |
| `submit_github_pull_request_review` | Submits one review (`COMMENT` or `REQUEST_CHANGES`; it cannot approve) with a summary and inline comments on changed lines. |
| `upsert_github_issue` | Files a finding as an issue keyed by a `fingerprint`; updates the open issue for the same fingerprint, leaves a closed one closed. |
| `create_github_pull_request` | Opens a pull request (draft by default) from a branch already pushed to the same repository. Only offered when `allow_pull_requests` is on. |

The pull request tools take `repo` (`owner/repo`) and `number`.

### Reviews without repeats

Each inline comment from `submit_github_pull_request_review` starts with a
hidden `<!-- everruns:finding:<key> -->` line. Pass a short, stable `key` per
finding (such as `null-deref-parse-config`); a key the agent already posted on
the pull request is skipped, so a review after a new push only adds what is
new. A second review of the same head commit is skipped entirely, which makes
webhook redeliveries harmless. Comments on lines that are not part of the diff
would make GitHub reject the whole review, so the tool moves them into the
review summary instead.

Only comments, reviews and issues written by a bot count as the agent's own,
so a person cannot hide a finding by pasting a marker into their own comment.

## Settings

| Setting | Default | Effect |
|---|---|---|
| `acts_as` | `agent` | Whose GitHub identity the tools use: `agent`, `person`, or `person_or_agent`. See [Authentication](#authentication). |
| `allow_pull_requests` | `false` | Adds `create_github_pull_request`. Pushing the branch also needs the GitHub App's **Contents: write** permission. |
| `private_issues_only` | `false` | `upsert_github_issue` refuses public repositories, so a finding is never disclosed in a public issue. |

`upsert_github_comment` starts the comment with a hidden
`<!-- everruns:<marker> -->` line and looks for it on the next run, so an agent
that runs on every push updates one comment instead of adding a new one each
time. Use a different marker for each kind of comment.

## Authentication

The tools act on GitHub as the identity `acts_as` names; only
`person_or_agent` ever uses a second one:

- `agent` (default) uses the agent's own GitHub connection: its GitHub App when
  you connected one, so comments are posted as the agent's bot and the tools
  reach only the repositories the App is installed on. Use it for bots, and for
  anything triggered without a person.
- `person` uses the GitHub connection of the person talking to the agent. Use it
  for a personal agent. A triggered run has no person, so it gets nothing.
- `person_or_agent` uses the person's connection when they have one, otherwise
  the agent's.

Each call gets a short-lived token limited to the one repository it names and
the permissions that tool needs: the read tools ask for **Pull requests: read**,
the comment tool for **Issues** and **Pull requests: write**, the review tool for
**Pull requests: write**, the issue tool for **Issues: write**, and
`create_github_pull_request` for **Pull requests: write** and **Contents: read**.
Without a connection for that identity the tools return `connection_required`.

The session's network access policy must allow `https://api.github.com/` when
one is set.
